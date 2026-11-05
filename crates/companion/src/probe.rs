//! `wowtracker-companion probe`: watches what the game writes to disk while
//! you play, to find out which files update live and how far behind they are.
//!
//! It only reads files in the game folder (Logs, WTF, Screenshots). It never
//! touches the game process.

use chrono::{Local, NaiveDate, NaiveDateTime, Utc};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Combat log timestamp: local wall-clock time, plus the UTC offset when the
/// client writes one (e.g. `11/5/2026 13:45:12.345-5`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogTime {
    pub time: NaiveDateTime,
    pub utc_offset_hours: Option<i32>,
}

impl LogTime {
    /// Seconds between the event and `now` (positive = the line is that old).
    pub fn age_seconds(&self, now_local: NaiveDateTime, now_utc: NaiveDateTime) -> f64 {
        match self.utc_offset_hours {
            Some(off) => {
                let event_utc = self.time - chrono::Duration::hours(off as i64);
                (now_utc - event_utc).num_milliseconds() as f64 / 1000.0
            }
            None => (now_local - self.time).num_milliseconds() as f64 / 1000.0,
        }
    }
}

/// Parses the timestamp at the start of a combat log line. Accepts
/// `M/D/YYYY HH:MM:SS.ffff`, `M/D HH:MM:SS.fff` (older clients, no year) and
/// an optional `+N` / `-N` UTC offset after the fraction.
pub fn parse_timestamp(line: &str, current_year: i32) -> Option<LogTime> {
    let (date, rest) = line.trim_start_matches('\u{feff}').split_once(' ')?;
    let time_token = rest.split_whitespace().next()?;

    let mut d = date.split('/');
    let month: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    let year: i32 = match d.next() {
        Some(y) => y.parse().ok()?,
        None => current_year,
    };

    // Split off an offset such as "-5" or "+1" after the seconds fraction.
    let (clock, offset) = match time_token.rfind(['+', '-']) {
        Some(i) if i > 0 => (&time_token[..i], time_token[i..].parse::<i32>().ok()),
        _ => (time_token, None),
    };
    let mut t = clock.split(':');
    let h: u32 = t.next()?.parse().ok()?;
    let m: u32 = t.next()?.parse().ok()?;
    let sec_str = t.next()?;
    let (whole, frac) = sec_str.split_once('.').unwrap_or((sec_str, "0"));
    let s: u32 = whole.parse().ok()?;
    let frac_digits: String = frac.chars().take(9).collect();
    let nanos = format!("{frac_digits:0<9}").parse::<u32>().ok()?;

    let time = NaiveDate::from_ymd_opt(year, month, day)?.and_hms_nano_opt(h, m, s, nanos)?;
    Some(LogTime { time, utc_offset_hours: offset })
}

/// Event name and the comma-separated fields after the timestamp.
pub fn event_and_fields(line: &str) -> Option<(String, Vec<String>)> {
    let body = line.split_once("  ").map(|(_, b)| b)?;
    let fields = split_fields(body);
    let event = fields.first()?.clone();
    if event.is_empty() || !event.bytes().all(|b| b.is_ascii_uppercase() || b == b'_' || b.is_ascii_digit()) {
        return None;
    }
    Some((event, fields))
}

/// Splits on commas outside double quotes, brackets and parentheses.
pub fn split_fields(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' if !quoted => {
                depth += 1;
                cur.push(c);
            }
            ']' | ')' if !quoted => {
                depth -= 1;
                cur.push(c);
            }
            ',' if !quoted && depth <= 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Best-effort: finds the advanced-logging position block (positionX,
/// positionY, uiMapID, facing, level) in an event's fields. The block's
/// offset depends on the event and client version, so search for a
/// plausible float, float, integer map ID, float pattern.
pub fn find_position(fields: &[String]) -> Option<(f64, f64, u32)> {
    if fields.len() < 20 {
        return None;
    }
    // positionX/Y are world coordinates (yards), uiMapID a positive integer.
    for i in 9..fields.len().saturating_sub(4) {
        let (Ok(x), Ok(y), Ok(map), Ok(facing)) = (
            fields[i].trim().parse::<f64>(),
            fields[i + 1].trim().parse::<f64>(),
            fields[i + 2].trim().parse::<u32>(),
            fields[i + 3].trim().parse::<f64>(),
        ) else {
            continue;
        };
        let looks_like_coords = fields[i].contains('.') && fields[i + 1].contains('.') && x.abs() < 40_000.0 && y.abs() < 40_000.0;
        if looks_like_coords && map > 0 && (0.0..=6.3).contains(&facing) {
            return Some((x, y, map));
        }
    }
    None
}

#[derive(Default)]
struct CombatStats {
    flushes: u32,
    lines: u64,
    newest_ages: Vec<f64>,
    oldest_ages: Vec<f64>,
    positions: u64,
}

struct Tracked {
    size: u64,
    modified: Option<SystemTime>,
}

/// Runs until Ctrl+C.
pub fn run(game_dir: &Path, show_lines: bool, log_file: &Path) -> Result<(), String> {
    let roots: Vec<PathBuf> = ["Logs", "WTF", "Screenshots", "Errors"]
        .iter()
        .map(|n| game_dir.join(n))
        .collect();

    match open_log(log_file) {
        Ok(()) => {
            let shown = if log_file.is_absolute() {
                log_file.to_path_buf()
            } else {
                std::env::current_dir().map(|d| d.join(log_file)).unwrap_or_else(|_| log_file.to_path_buf())
            };
            say(&format!("---- probe started; also writing to {} ----", shown.display()));
        }
        Err(e) => say(&format!("probe: cannot write {} ({e}); output goes to the screen only", log_file.display())),
    }

    say(&format!("probe: watching {} (Logs, WTF, Screenshots, Errors)", game_dir.display()));
    say("probe: this only reads files; it never touches the game. Ctrl+C to stop.");
    say("probe: for the combat log test, in game type:  /console advancedCombatLogging 1   then  /combatlog");
    say("       Then walk, mount up, fight something, eat or drink, and change zone.");

    let mut known: HashMap<PathBuf, Tracked> = HashMap::new();
    for f in walk(&roots) {
        if let Ok(m) = std::fs::metadata(&f) {
            known.insert(f, Tracked { size: m.len(), modified: m.modified().ok() });
        }
    }
    say(&format!("probe: {} existing files noted; reporting changes from now on", known.len()));

    let mut offsets: HashMap<PathBuf, u64> = HashMap::new();
    let mut partial: HashMap<PathBuf, String> = HashMap::new();
    let mut seen_events: HashSet<String> = HashSet::new();
    let mut stats = CombatStats::default();
    let mut write_counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut last_summary = Instant::now();
    let year = Local::now().date_naive().format("%Y").to_string().parse().unwrap_or(2026);

    loop {
        std::thread::sleep(Duration::from_millis(500));
        for f in walk(&roots) {
            let Ok(meta) = std::fs::metadata(&f) else { continue };
            let size = meta.len();
            let modified = meta.modified().ok();
            let rel = rel_path(game_dir, &f);
            let prev = known.get(&f);
            let changed = match prev {
                None => true,
                Some(p) => p.size != size || p.modified != modified,
            };
            if !changed {
                continue;
            }
            let old_size = prev.map(|p| p.size);
            known.insert(f.clone(), Tracked { size, modified });
            *write_counts.entry(group_name(&rel)).or_default() += 1;

            if is_combat_log(&f) {
                let start = *offsets.entry(f.clone()).or_insert(old_size.unwrap_or(0));
                let start = if size < start { 0 } else { start };
                match read_from(&f, start, size) {
                    Ok(text) => {
                        offsets.insert(f.clone(), size);
                        let carry = partial.remove(&f).unwrap_or_default();
                        let joined = carry + &text;
                        let mut lines: Vec<&str> = joined.split('\n').collect();
                        let tail = lines.pop().unwrap_or("");
                        if !tail.is_empty() {
                            partial.insert(f.clone(), tail.to_string());
                        }
                        report_combat_batch(&rel, &lines, year, show_lines, &mut seen_events, &mut stats);
                    }
                    Err(e) => say(&format!("{rel}: cannot read ({e})")),
                }
            } else {
                let delta = match old_size {
                    None => format!("new file, {}", bytes(size)),
                    Some(o) if size >= o => format!("+{}", bytes(size - o)),
                    Some(o) => format!("-{}", bytes(o - size)),
                };
                say(&format!("{rel} written ({delta})"));
            }
        }

        // Summarise only the last minute, and only if something was written.
        if last_summary.elapsed() >= Duration::from_secs(60) {
            last_summary = Instant::now();
            if !write_counts.is_empty() {
                summary(&stats, &write_counts);
                write_counts.clear();
            }
        }
    }
}

fn report_combat_batch(
    rel: &str,
    lines: &[&str],
    year: i32,
    show_lines: bool,
    seen: &mut HashSet<String>,
    stats: &mut CombatStats,
) {
    let now_local = Local::now().naive_local();
    let now_utc = Utc::now().naive_utc();
    let mut newest: Option<f64> = None;
    let mut oldest: Option<f64> = None;
    let mut count = 0u64;
    let mut positions = 0u64;
    let mut last_pos: Option<(f64, f64, u32)> = None;

    for raw in lines {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        count += 1;
        if let Some(ts) = parse_timestamp(line, year) {
            let age = ts.age_seconds(now_local, now_utc);
            newest = Some(newest.map_or(age, |n: f64| n.min(age)));
            oldest = Some(oldest.map_or(age, |o: f64| o.max(age)));
        }
        if let Some((event, fields)) = event_and_fields(line) {
            if let Some(p) = find_position(&fields) {
                positions += 1;
                last_pos = Some(p);
            }
            if seen.insert(event.clone()) || show_lines {
                say(&format!("  first {event} line: {}", truncate(line, 220)));
            }
        }
    }
    if count == 0 {
        return;
    }
    stats.flushes += 1;
    stats.lines += count;
    stats.positions += positions;
    if let (Some(n), Some(o)) = (newest, oldest) {
        stats.newest_ages.push(n);
        stats.oldest_ages.push(o);
        say(&format!(
            "{rel}: +{count} lines; newest event {n:.1}s old, oldest {o:.1}s old{}",
            last_pos
                .map(|(x, y, m)| format!("; {positions} with position, last: map {m} at ({x:.1}, {y:.1})"))
                .unwrap_or_default()
        ));
    } else {
        say(&format!("{rel}: +{count} lines (timestamps not recognised)"));
    }
}

fn summary(stats: &CombatStats, writes: &BTreeMap<String, u32>) {
    let list: Vec<String> = writes
        .iter()
        .map(|(group, n)| if *n > 1 { format!("{group} ×{n}") } else { group.clone() })
        .collect();
    say(&format!("last minute, files written: {}", list.join(", ")));
    if stats.flushes > 0 {
        let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let max = |v: &[f64]| v.iter().cloned().fold(f64::MIN, f64::max);
        say(&format!(
            "combat log so far: {} flushes, {} lines, {} with position; newest-line age avg {:.1}s max {:.1}s; oldest-line age avg {:.1}s max {:.1}s",
            stats.flushes,
            stats.lines,
            stats.positions,
            avg(&stats.newest_ages),
            max(&stats.newest_ages),
            avg(&stats.oldest_ages),
            max(&stats.oldest_ages),
        ));
    }
}

fn walk(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = roots.iter().filter(|r| r.is_dir()).cloned().collect();
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(p),
                Ok(t) if t.is_file() => out.push(p),
                _ => {}
            }
        }
    }
    out
}

fn read_from(path: &Path, start: u64, end: u64) -> std::io::Result<String> {
    // Never read more than 4 MB at once (e.g. when a big log appears).
    let start = start.max(end.saturating_sub(4 * 1024 * 1024));
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((end - start) as usize);
    f.take(end - start).read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn is_combat_log(p: &Path) -> bool {
    p.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .is_some_and(|n| n.starts_with("wowcombatlog") && n.ends_with(".txt"))
}

fn rel_path(base: &Path, p: &Path) -> String {
    p.strip_prefix(base).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// Groups per-account / per-character paths so the summary stays short.
fn group_name(rel: &str) -> String {
    let parts: Vec<&str> = rel.split('/').collect();
    match parts.as_slice() {
        ["WTF", "Account", _, "SavedVariables", file] => format!("WTF/Account/*/SavedVariables/{file}"),
        ["WTF", "Account", _, _, _, "SavedVariables", file] => format!("WTF/Account/*/*/*/SavedVariables/{file}"),
        ["WTF", "Account", _, rest @ ..] => format!("WTF/Account/*/{}", rest.join("/")),
        ["Screenshots", ..] => "Screenshots/*".to_string(),
        ["Logs", f] if f.to_lowercase().starts_with("wowcombatlog") => "Logs/WoWCombatLog*.txt".to_string(),
        _ => rel.to_string(),
    }
}

fn bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / 1024.0 / 1024.0)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// Copy of everything the probe prints, so nothing is lost to terminal scrollback.
static LOG_FILE: std::sync::OnceLock<std::sync::Mutex<File>> = std::sync::OnceLock::new();

pub const DEFAULT_LOG_FILE: &str = "wowtracker-probe.log";

fn open_log(path: &Path) -> std::io::Result<()> {
    let f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    let _ = LOG_FILE.set(std::sync::Mutex::new(f));
    Ok(())
}

fn say(msg: &str) {
    let line = format!("[{}] {msg}", Local::now().format("%H:%M:%S"));
    println!("{line}");
    if let Some(Ok(mut f)) = LOG_FILE.get().map(|m| m.lock()) {
        use std::io::Write;
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timestamps() {
        let t = parse_timestamp("11/5/2026 13:45:12.3450  SPELL_CAST_SUCCESS,x", 2026).unwrap();
        assert_eq!(t.time.to_string(), "2026-11-05 13:45:12.345");
        assert_eq!(t.utc_offset_hours, None);

        let t = parse_timestamp("4/22 19:23:56.683  SWING_DAMAGE,x", 2026).unwrap();
        assert_eq!(t.time.to_string(), "2026-04-22 19:23:56.683");

        let t = parse_timestamp("11/5/2026 13:45:12.345-5  ZONE_CHANGE,1,\"Elwynn\",0", 2026).unwrap();
        assert_eq!(t.utc_offset_hours, Some(-5));
        assert_eq!(t.time.to_string(), "2026-11-05 13:45:12.345");

        assert!(parse_timestamp("garbage", 2026).is_none());
    }

    #[test]
    fn age_uses_offset_when_present() {
        let t = parse_timestamp("11/5/2026 13:45:10.000+1  X", 2026).unwrap();
        let now_utc = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap().and_hms_opt(12, 45, 12).unwrap();
        let now_local = now_utc; // ignored when an offset is present
        assert!((t.age_seconds(now_local, now_utc) - 2.0).abs() < 0.001);
    }

    #[test]
    fn splits_fields_with_quotes_and_lists() {
        let f = split_fields(r#"SPELL_CAST_SUCCESS,Player-1-0001,"Bob, the Great",0x511,[1,2],(3,4),"x""#);
        assert_eq!(f[2], "Bob, the Great");
        assert_eq!(f[4], "[1,2]");
        assert_eq!(f[5], "(3,4)");
        assert_eq!(f.len(), 7);
    }

    #[test]
    fn finds_advanced_position() {
        // SPELL_CAST_SUCCESS with the 17 advanced parameters, as documented for 9.x-11.x.
        let line = "11/5/2026 13:45:12.3450  SPELL_CAST_SUCCESS,Player-1-0A1B2C3D,\"Testchar-Firemaw\",0x511,0x0,\
                    0000000000000000,nil,0x80000000,0x80000000,6603,\"Auto Attack\",0x1,\
                    Player-1-0A1B2C3D,0000000000000000,1200,1200,0,0,500,0,1,0,100,0,\
                    -10520.31,601.72,1436,3.1416,11";
        let (event, fields) = event_and_fields(line).unwrap();
        assert_eq!(event, "SPELL_CAST_SUCCESS");
        let (x, y, map) = find_position(&fields).unwrap();
        assert_eq!(map, 1436);
        assert!((x + 10520.31).abs() < 1e-6 && (y - 601.72).abs() < 1e-6);
    }

    #[test]
    fn no_position_in_plain_events() {
        let (_, fields) = event_and_fields("11/5/2026 13:45:12.3450  ZONE_CHANGE,1436,\"Westfall\",0").unwrap();
        assert!(find_position(&fields).is_none());
    }

    #[test]
    fn groups_paths() {
        assert_eq!(group_name("WTF/Account/ACC/SavedVariables/X.lua"), "WTF/Account/*/SavedVariables/X.lua");
        assert_eq!(group_name("Logs/WoWCombatLog-110526_1345.txt"), "Logs/WoWCombatLog*.txt");
        assert_eq!(group_name("WTF/Config.wtf"), "WTF/Config.wtf");
    }
}
