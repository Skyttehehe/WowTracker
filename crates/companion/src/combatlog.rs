//! Reads the player's position from the game's combat log.
//!
//! With `/combatlog` and advanced combat logging on, the game writes
//! `Logs\WoWCombatLog-*.txt` in batches of about 32 KB (every 40-65 s in a busy
//! zone, measured 2026-11-05). Many lines carry an "advanced info" block with
//! one unit's world position. This module keeps only lines whose advanced
//! block is about the player, converts the position to map coordinates with
//! the bounds from the log's MAP_CHANGE line, and never keeps anything about
//! other players.
//!
//! Combat log v22 layout (Forever 1.60.1): the advanced block has 19 fields,
//! starting at field 9 for SWING_* and ENVIRONMENTAL_* events and at field 12
//! for SPELL_*, RANGE_* and DAMAGE_* events:
//! infoGUID, ownerGUID, hp, maxHP, attackPower, spellPower, armor, absorb,
//! ?, ?, powerType, power, maxPower, powerCost, positionX, positionY, uiMapID,
//! facing, level.

use chrono::{Local, TimeZone};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::probe::{parse_timestamp, split_fields, LogTime};
use wowtracker_protocol::{LivePosition, TrailPoint};

/// Fields in the advanced block (combat log version 22).
const ADVANCED_LEN: usize = 19;
const POS_X: usize = 14;
const POS_Y: usize = 15;
const MAP_ID: usize = 16;
const FACING: usize = 17;

/// Most bytes read from the log per poll, so a huge old log is read in steps.
const MAX_READ: u64 = 8 * 1024 * 1024;

/// A map's world bounds, from `MAP_CHANGE,id,"name",xTop,xBottom,yLeft,yRight`.
#[derive(Debug, Clone, PartialEq)]
pub struct MapBounds {
    pub id: u32,
    pub name: String,
    pub x_top: f64,
    pub x_bottom: f64,
    pub y_left: f64,
    pub y_right: f64,
}

impl MapBounds {
    /// World position to normalized map coordinates (0..1), or None if far
    /// outside this map.
    pub fn to_map(&self, wx: f64, wy: f64) -> Option<(f64, f64)> {
        let x = (self.y_left - wy) / (self.y_left - self.y_right);
        let y = (self.x_top - wx) / (self.x_top - self.x_bottom);
        let ok = |v: f64| v.is_finite() && (-0.05..=1.05).contains(&v);
        (ok(x) && ok(y)).then(|| (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
    }
}

/// Turns combat log lines into the player's positions.
#[derive(Debug, Default)]
pub struct Parser {
    /// The player's GUID: the unit whose flags say "mine" and "player".
    pub my_guid: Option<String>,
    pub my_name: Option<String>,
    pub map: Option<MapBounds>,
    year: i32,
    /// Newest position not yet taken by [`Parser::take_update`].
    latest: Option<LivePosition>,
    /// Earlier positions since the last update, oldest first.
    pending_trail: Vec<TrailPoint>,
    last_point: Option<TrailPoint>,
    /// Newest event time handed out, so re-reading a file sends nothing twice.
    sent_up_to: i64,
}

impl Parser {
    pub fn new() -> Self {
        Parser { year: Local::now().format("%Y").to_string().parse().unwrap_or(2026), ..Default::default() }
    }

    /// Feeds one line. Returns a message worth logging, if any (new map,
    /// player found).
    pub fn line(&mut self, raw: &str) -> Option<String> {
        let line = raw.trim_end_matches(['\r', '\n']);
        let (_, body) = line.split_once("  ")?;
        let f = split_fields(body);
        let event = f.first()?.as_str();
        let mut note = None;

        match event {
            "MAP_CHANGE" if f.len() >= 7 => {
                let num = |i: usize| f[i].trim().parse::<f64>().ok();
                if let (Ok(id), Some(a), Some(b), Some(c), Some(d)) = (f[1].trim().parse::<u32>(), num(3), num(4), num(5), num(6)) {
                    let bounds = MapBounds { id, name: f[2].clone(), x_top: a, x_bottom: b, y_left: c, y_right: d };
                    if self.map.as_ref() != Some(&bounds) {
                        note = Some(format!("combat log: map {} ({id})", bounds.name));
                    }
                    self.map = Some(bounds);
                }
                return note;
            }
            "COMBAT_LOG_VERSION" | "ZONE_CHANGE" => return None,
            _ => {}
        }
        if f.len() < 9 {
            return None;
        }

        // Learn who "me" is from the source/dest flags.
        for (guid, name, flags) in [(1, 2, 3), (5, 6, 7)] {
            if is_me_flags(&f[flags]) && f[guid].starts_with("Player-") && self.my_guid.as_deref() != Some(f[guid].as_str()) {
                self.my_guid = Some(f[guid].clone());
                self.my_name = Some(f[name].clone());
                note = Some(format!("combat log: your character is {} ({})", f[name], f[guid]));
            }
        }
        self.my_position(line, &f);
        note
    }

    /// Records the position if this line's advanced block is about the player.
    fn my_position(&mut self, line: &str, f: &[String]) -> Option<()> {
        let me = self.my_guid.clone()?;

        // Advanced block: find it at either known offset and check its shape.
        let start = [9usize, 12].into_iter().find(|&s| {
            f.len() >= s + ADVANCED_LEN && is_guid(&f[s]) && is_guid(&f[s + 1]) && f[s + MAP_ID].trim().parse::<u32>().is_ok()
        })?;
        if f[start] != me {
            return None;
        }
        let wx = f[start + POS_X].trim().parse::<f64>().ok()?;
        let wy = f[start + POS_Y].trim().parse::<f64>().ok()?;
        let map_id = f[start + MAP_ID].trim().parse::<u32>().ok()?;
        let facing = f[start + FACING].trim().parse::<f64>().ok()?;
        if map_id == 0 || !(0.0..=7.0).contains(&facing) {
            return None; // 0 = inside an instance; no usable position
        }
        let bounds = self.map.as_ref().filter(|m| m.id == map_id)?;
        let (x, y) = bounds.to_map(wx, wy)?;
        let t = unix_time(&parse_timestamp(line, self.year)?)?;
        if t < self.sent_up_to {
            return None;
        }

        // The other unit in the line, if it is a creature (never a player).
        let other = if f[1] == me { 5 } else if f[5] == me { 1 } else { 0 };
        let opponent = (other > 0 && (f[other].starts_with("Creature-") || f[other].starts_with("Vehicle-")))
            .then(|| f[other + 1].clone());

        let point = TrailPoint { t, map_id, x, y };
        if let Some(prev) = self.latest.take() {
            // Keep the previous one as a breadcrumb if it differs enough.
            let p = TrailPoint { t: prev.t, map_id: prev.map_id, x: prev.x, y: prev.y };
            if keep_breadcrumb(self.last_point.as_ref(), &p) {
                self.pending_trail.push(p);
                self.last_point = Some(p);
            }
        }
        self.latest = Some(LivePosition {
            t,
            guid: Some(me),
            name: self.my_name.clone(),
            map_id,
            map: Some(bounds.name.clone()),
            x: point.x,
            y: point.y,
            opponent,
            trail: Vec::new(),
        });
        Some(())
    }

    /// The newest position plus breadcrumbs since the last call, if any.
    pub fn take_update(&mut self) -> Option<LivePosition> {
        let mut live = self.latest.take()?;
        live.trail = std::mem::take(&mut self.pending_trail);
        self.sent_up_to = live.t + 1;
        self.last_point = Some(TrailPoint { t: live.t, map_id: live.map_id, x: live.x, y: live.y });
        Some(live)
    }
}

/// A breadcrumb every 5 s, or sooner after moving about 1% of the map.
fn keep_breadcrumb(prev: Option<&TrailPoint>, p: &TrailPoint) -> bool {
    match prev {
        None => true,
        Some(q) => q.map_id != p.map_id || p.t - q.t >= 5 || ((p.x - q.x).powi(2) + (p.y - q.y).powi(2)).sqrt() > 0.01,
    }
}

/// Flags of a unit controlled by and belonging to the logging player.
fn is_me_flags(flags: &str) -> bool {
    let v = u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16).unwrap_or(0);
    const AFFILIATION_MINE: u32 = 0x1;
    const CONTROL_PLAYER: u32 = 0x100;
    const TYPE_PLAYER: u32 = 0x400;
    v & 0xF == AFFILIATION_MINE && v & CONTROL_PLAYER != 0 && v & TYPE_PLAYER != 0
}

fn is_guid(s: &str) -> bool {
    s == "0000000000000000"
        || ["Player-", "Creature-", "Pet-", "Vehicle-", "GameObject-", "Vignette-"].iter().any(|p| s.starts_with(p))
}

/// Combat log time to Unix seconds. Without an explicit offset the time is
/// this PC's local time, which is right because the companion runs on it.
fn unix_time(t: &LogTime) -> Option<i64> {
    match t.utc_offset_hours {
        Some(off) => Some((t.time - chrono::Duration::hours(off as i64)).and_utc().timestamp()),
        None => Local.from_local_datetime(&t.time).earliest().map(|d| d.timestamp()),
    }
}

/// Follows the newest `WoWCombatLog*.txt` in the game's Logs folder.
pub struct Reader {
    logs_dir: PathBuf,
    file: Option<PathBuf>,
    offset: u64,
    partial: Vec<u8>,
    pub parser: Parser,
}

impl Reader {
    pub fn new(game_dir: &Path) -> Self {
        Reader { logs_dir: game_dir.join("Logs"), file: None, offset: 0, partial: Vec::new(), parser: Parser::new() }
    }

    pub fn current_file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Reads whatever the game has written since the last call. Returns
    /// messages worth logging.
    pub fn poll(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        let Some(newest) = newest_log(&self.logs_dir) else { return notes };
        if self.file.as_ref() != Some(&newest) {
            notes.push(format!(
                "combat log: reading {}",
                newest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            ));
            self.file = Some(newest.clone());
            self.offset = 0;
            self.partial.clear();
            self.parser.map = None; // the new file starts with its own MAP_CHANGE
        }
        let Ok(size) = std::fs::metadata(&newest).map(|m| m.len()) else { return notes };
        if size < self.offset {
            self.offset = 0; // file was replaced
            self.partial.clear();
        }
        // Read in steps so a very large log does not have to fit in memory.
        while self.offset < size {
            let want = (size - self.offset).min(MAX_READ);
            let mut buf = Vec::with_capacity(want as usize);
            let read = File::open(&newest).and_then(|mut f| {
                f.seek(SeekFrom::Start(self.offset))?;
                f.take(want).read_to_end(&mut buf)
            });
            match read {
                Ok(0) | Err(_) => break,
                Ok(n) => self.offset += n as u64,
            }
            self.partial.extend_from_slice(&buf);
            // Hand complete lines to the parser; keep a trailing partial line.
            let Some(last_nl) = self.partial.iter().rposition(|&b| b == b'\n') else { continue };
            let rest = self.partial.split_off(last_nl + 1);
            let complete = std::mem::replace(&mut self.partial, rest);
            for line in complete.split(|&b| b == b'\n') {
                if line.is_empty() {
                    continue;
                }
                if let Some(n) = self.parser.line(&String::from_utf8_lossy(line)) {
                    notes.push(n);
                }
            }
        }
        notes
    }
}

fn newest_log(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.starts_with("wowcombatlog") && n.ends_with(".txt")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real lines from a Forever 1.60.1 log (2026-11-05), shortened where noted.
    const HEADER: &str = "11/5/2026 18:19:19.2531  COMBAT_LOG_VERSION,22,ADVANCED_LOG_ENABLED,1,BUILD_VERSION,1.60.1,PROJECT_ID,18";
    const MAP: &str = "11/5/2026 18:19:19.2531  MAP_CHANGE,1438,\"Teldrassil\",11831.250000,8437.500000,3814.583252,-1277.083252";
    const MY_SWING: &str = "11/5/2026 18:19:41.8231  SWING_DAMAGE,Player-6940-00AF5B76,\"Fractured-Archive-PvP-EU\",0x511,0x80000000,Creature-0-7092-1-470-1999-00006CB49E,\"Webwood Venomfang\",0x10a48,0x80000000,Player-6940-00AF5B76,0000000000000000,93,174,85,0,314,0,0,0,1,76,1000,0,9325.56,734.17,1438,3.5546,1,12,15,-1,1,0,0,0,nil,nil,nil";
    const MY_CAST: &str = "11/5/2026 18:19:44.4921  SPELL_CAST_SUCCESS,Player-6940-00AF5B76,\"Fractured-Archive-PvP-EU\",0x511,0x80000000,Creature-0-7092-1-470-1999-00006CB49E,\"Webwood Venomfang\",0x10a48,0x80000000,772,\"Rend\",0x1,Player-6940-00AF5B76,0000000000000000,85,174,85,0,314,0,0,0,1,162,1000,100,9325.56,734.17,1438,3.5417,1";
    const OTHER_PLAYER: &str = "11/5/2026 18:19:20.0591  SWING_DAMAGE_LANDED,Creature-0-7092-1-470-2043-00006CB314,\"Nightsaber Stalker\",0xa28,0x80000000,Player-6942-006C8362,\"Stink-Ring-PvP-EU\",0x518,0x80000000,Player-6942-006C8362,0000000000000000,214,226,164,0,309,0,0,0,0,241,330,0,9445.52,596.85,1438,2.9273,2,6,8,-1,1,0,0,0,nil,nil,nil";
    const MOB_HIT_BY_ME: &str = "11/5/2026 18:19:48.4981  SPELL_DAMAGE,Player-6940-00AF5B76,\"Fractured-Archive-PvP-EU\",0x511,0x80000000,Creature-0-7092-1-470-1999-00006CB49E,\"Webwood Venomfang\",0x10a48,0x80000000,78,\"Heroic Strike\",0x1,Creature-0-7092-1-470-1999-00006CB49E,0000000000000000,76,137,21,0,245,0,0,0,1,0,0,0,9323.23,733.02,1438,0.4568,7,21,26,-1,1,0,0,0,nil,nil,nil,ST";
    const HIT_ON_ME_LATER: &str = "11/5/2026 18:20:39.6241  SWING_DAMAGE_LANDED,Creature-0-7092-1-470-2043-00006CB9A4,\"Nightsaber Stalker\",0x10a48,0x80000000,Player-6940-00AF5B76,\"Fractured-Archive-PvP-EU\",0x511,0x80000000,Player-6940-00AF5B76,0000000000000000,120,174,85,0,314,0,0,0,1,30,1000,0,9331.48,735.36,1438,3.0,1,5,5,-1,1,0,0,0,nil,nil,nil";

    fn feed(p: &mut Parser, lines: &[&str]) {
        for l in lines {
            p.line(l);
        }
    }

    #[test]
    fn map_bounds_convert_known_points() {
        let mut p = Parser::new();
        feed(&mut p, &[HEADER, MAP]);
        let m = p.map.clone().unwrap();
        assert_eq!((m.id, m.name.as_str()), (1438, "Teldrassil"));
        // Night elf start at Aldrassil: world (10311, 832) is about 58.6, 44.8.
        let (x, y) = m.to_map(10311.0, 832.0).unwrap();
        assert!((x - 0.586).abs() < 0.002 && (y - 0.448).abs() < 0.002, "{x} {y}");
        assert!(m.to_map(0.0, 0.0).is_none(), "far outside the map");
    }

    #[test]
    fn keeps_only_my_lines() {
        let mut p = Parser::new();
        feed(&mut p, &[HEADER, MAP, OTHER_PLAYER]);
        assert!(p.take_update().is_none(), "nothing before we know who 'me' is, and never others");

        // The line that reveals "me" is reported even though its advanced
        // block is about the mob.
        assert!(p.line(MOB_HIT_BY_ME).is_some_and(|n| n.contains("your character is Fractured-Archive-PvP-EU")));
        feed(&mut p, &[MY_SWING, OTHER_PLAYER, MOB_HIT_BY_ME]);
        assert_eq!(p.my_guid.as_deref(), Some("Player-6940-00AF5B76"));
        let u = p.take_update().unwrap();
        assert_eq!(u.map_id, 1438);
        assert_eq!(u.map.as_deref(), Some("Teldrassil"));
        assert!((u.x - 0.605).abs() < 0.002 && (u.y - 0.738).abs() < 0.002, "{} {}", u.x, u.y);
        assert_eq!(u.opponent.as_deref(), Some("Webwood Venomfang"));
        assert_eq!(u.name.as_deref(), Some("Fractured-Archive-PvP-EU"));
        assert!(p.take_update().is_none());
    }

    #[test]
    fn breadcrumbs_and_no_repeats() {
        let mut p = Parser::new();
        feed(&mut p, &[HEADER, MAP, MY_SWING, MY_CAST, HIT_ON_ME_LATER]);
        let u = p.take_update().unwrap();
        assert_eq!(u.opponent.as_deref(), Some("Nightsaber Stalker"));
        // The first swing becomes a breadcrumb; the cast 3 s later at the same
        // spot does not.
        assert_eq!(u.trail.len(), 1);
        assert!(u.trail[0].t < u.t);

        // Reading the same lines again (e.g. a restart) sends nothing new.
        feed(&mut p, &[MY_SWING, MY_CAST, HIT_ON_ME_LATER]);
        assert!(p.take_update().is_none());
    }

    #[test]
    fn needs_map_bounds() {
        let mut p = Parser::new();
        feed(&mut p, &[HEADER, MY_SWING]);
        assert!(p.take_update().is_none(), "no MAP_CHANGE yet, so no map coordinates");
    }

    #[test]
    fn flags() {
        assert!(is_me_flags("0x511"));
        assert!(!is_me_flags("0x518"), "another player in the group/zone");
        assert!(!is_me_flags("0x1111"), "my pet");
        assert!(!is_me_flags("0xa28"));
    }

    #[test]
    fn reader_follows_the_newest_file_in_steps() {
        let dir = std::env::temp_dir().join(format!("wowtracker-cl-{}", std::process::id()));
        let logs = dir.join("Logs");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&logs).unwrap();
        let file = logs.join("WoWCombatLog-110526_181919.txt");

        // First write ends mid-line, like a 32 KB batch can.
        let first = format!("{HEADER}\r\n{MAP}\r\n{MY_SWING}\r\n");
        let (a, b) = MY_CAST.split_at(40);
        std::fs::write(&file, format!("{first}{a}")).unwrap();
        let mut r = Reader::new(&dir);
        let notes = r.poll();
        assert!(notes.iter().any(|n| n.contains("reading WoWCombatLog")));
        assert!(notes.iter().any(|n| n.contains("Teldrassil")));
        let u = r.parser.take_update().unwrap();
        assert!(u.trail.is_empty());

        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        write!(f, "{b}\r\n{HIT_ON_ME_LATER}\r\n").unwrap();
        drop(f);
        r.poll();
        let u2 = r.parser.take_update().unwrap();
        assert!(u2.t > u.t);
        assert_eq!(u2.opponent.as_deref(), Some("Nightsaber Stalker"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
