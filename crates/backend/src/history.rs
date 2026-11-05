//! Everything ever received, kept in SQLite so nothing is lost when the
//! addon's capped lists roll over or a save replaces the last one.
//!
//! All tables are append-only. Their primary keys include the time, so an
//! addon save that resends old trail points or journal entries adds nothing.
//! Sessions are not stored: they are worked out from the activity times when
//! asked for (a gap of more than [`SESSION_GAP`] seconds, a login or a logout
//! starts a new one), so they never go stale.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;
use wowtracker_protocol::{Character, JournalEntry, TrailPoint};

/// Silence longer than this ends a session.
pub const SESSION_GAP: i64 = 15 * 60;

pub struct History {
    db: Mutex<Connection>,
}

/// A point in time when level, XP and gold were known (each save, each live frame).
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub t: i64,
    pub level: Option<u32>,
    pub xp: Option<u64>,
    pub xp_max: Option<u64>,
    pub money: Option<u64>,
    pub played: Option<f64>,
    #[serde(rename = "mapID")]
    pub map_id: Option<u32>,
    pub zone: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub t: i64,
    #[serde(rename = "mapID")]
    pub map_id: u32,
    pub x: f64,
    pub y: f64,
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// Start time; also the session's ID.
    pub start: i64,
    pub end: i64,
    /// True when it ended with a logout entry.
    pub logged_out: bool,
    pub level_start: Option<u32>,
    pub level_end: Option<u32>,
    pub xp_gained: u64,
    pub money_start: Option<u64>,
    pub money_end: Option<u64>,
    pub quests: u32,
    pub kills: u32,
    pub deaths: u32,
    pub loot: u32,
    pub levels: u32,
    /// Zone names entered or saved during the session, in order.
    pub zones: Vec<String>,
    /// uiMapIDs the trail passed through.
    #[serde(rename = "mapIDs")]
    pub map_ids: Vec<u32>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryData {
    pub positions: Vec<Position>,
    pub events: Vec<serde_json::Value>,
    pub samples: Vec<Sample>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub session: Session,
    #[serde(flatten)]
    pub data: HistoryData,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS positions (
    char TEXT NOT NULL, t INTEGER NOT NULL, map_id INTEGER NOT NULL, x REAL NOT NULL, y REAL NOT NULL, source TEXT NOT NULL,
    PRIMARY KEY (char, t)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS events (
    char TEXT NOT NULL, t INTEGER NOT NULL, kind TEXT NOT NULL, json TEXT NOT NULL,
    PRIMARY KEY (char, t, kind, json)
);
CREATE TABLE IF NOT EXISTS samples (
    char TEXT NOT NULL, t INTEGER NOT NULL, level INTEGER, xp INTEGER, xp_max INTEGER, money INTEGER,
    played REAL, map_id INTEGER, zone TEXT,
    PRIMARY KEY (char, t)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS xp_curve (
    char TEXT NOT NULL, played REAL NOT NULL, level INTEGER NOT NULL, xp INTEGER NOT NULL, xp_max INTEGER NOT NULL,
    PRIMARY KEY (char, played)
) WITHOUT ROWID;
";

fn db_err(e: rusqlite::Error) -> String {
    e.to_string()
}

impl History {
    pub fn open(path: &Path) -> Result<History, String> {
        let conn = Connection::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn memory() -> History {
        Self::init(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn init(conn: Connection) -> Result<History, String> {
        conn.pragma_update(None, "journal_mode", "WAL").map_err(db_err)?;
        conn.pragma_update(None, "synchronous", "NORMAL").map_err(db_err)?;
        conn.execute_batch(SCHEMA).map_err(db_err)?;
        Ok(History { db: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Stores everything new in an addon save. Returns how many rows were added.
    pub fn record_character(&self, c: &Character) -> Result<usize, String> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(db_err)?;
        let mut added = 0;
        {
            let mut pos = tx
                .prepare_cached("INSERT OR IGNORE INTO positions VALUES (?1, ?2, ?3, ?4, ?5, 'addon')")
                .map_err(db_err)?;
            for p in &c.trail {
                added += pos.execute(params![c.key, p.t, p.map_id, p.x, p.y]).map_err(db_err)?;
            }
            let mut ev = tx.prepare_cached("INSERT OR IGNORE INTO events VALUES (?1, ?2, ?3, ?4)").map_err(db_err)?;
            for e in &c.journal {
                let json = serde_json::to_string(e).map_err(|e| e.to_string())?;
                added += ev.execute(params![c.key, e.t, e.kind, json]).map_err(db_err)?;
            }
            let mut xp = tx.prepare_cached("INSERT OR IGNORE INTO xp_curve VALUES (?1, ?2, ?3, ?4, ?5)").map_err(db_err)?;
            for s in &c.xp_samples {
                added += xp.execute(params![c.key, s.played, s.level, s.xp, s.xp_max]).map_err(db_err)?;
            }
        }
        if let Some(t) = c.saved_at {
            added += insert_sample(
                &tx,
                &c.key,
                &Sample {
                    t,
                    level: c.level,
                    xp: c.xp,
                    xp_max: c.xp_max,
                    money: c.money,
                    played: c.played,
                    map_id: c.map_id,
                    zone: c.zone.clone(),
                },
            )?;
        }
        tx.commit().map_err(db_err)?;
        Ok(added)
    }

    /// Stores positions read live (combat log, screen strip).
    pub fn record_positions(&self, char: &str, points: &[TrailPoint], source: &str) -> Result<usize, String> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(db_err)?;
        let mut added = 0;
        {
            let mut pos = tx.prepare_cached("INSERT OR IGNORE INTO positions VALUES (?1, ?2, ?3, ?4, ?5, ?6)").map_err(db_err)?;
            for p in points {
                added += pos.execute(params![char, p.t, p.map_id, p.x, p.y, source]).map_err(db_err)?;
            }
        }
        tx.commit().map_err(db_err)?;
        Ok(added)
    }

    pub fn record_sample(&self, char: &str, s: &Sample) -> Result<usize, String> {
        insert_sample(&self.conn(), char, s)
    }

    pub fn record_events(&self, char: &str, events: &[JournalEntry]) -> Result<usize, String> {
        let conn = self.conn();
        let mut ev = conn.prepare_cached("INSERT OR IGNORE INTO events VALUES (?1, ?2, ?3, ?4)").map_err(db_err)?;
        let mut added = 0;
        for e in events {
            let json = serde_json::to_string(e).map_err(|e| e.to_string())?;
            added += ev.execute(params![char, e.t, e.kind, json]).map_err(db_err)?;
        }
        Ok(added)
    }

    /// Everything for one character between `from` and `to` (inclusive).
    /// `kinds` limits the events; `limit` keeps the newest rows of each list.
    pub fn range(&self, char: &str, from: i64, to: i64, kinds: Option<&[String]>, limit: usize) -> Result<HistoryData, String> {
        let conn = self.conn();
        let limit = limit.max(1) as i64;

        let mut st = conn
            .prepare_cached(
                "SELECT t, map_id, x, y, source FROM (SELECT * FROM positions WHERE char = ?1 AND t BETWEEN ?2 AND ?3
                 ORDER BY t DESC LIMIT ?4) ORDER BY t",
            )
            .map_err(db_err)?;
        let positions = st
            .query_map(params![char, from, to, limit], |r| {
                Ok(Position { t: r.get(0)?, map_id: r.get(1)?, x: r.get(2)?, y: r.get(3)?, source: r.get(4)? })
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;

        let mut st = conn
            .prepare_cached(
                "SELECT kind, json FROM (SELECT * FROM events WHERE char = ?1 AND t BETWEEN ?2 AND ?3
                 ORDER BY t DESC LIMIT ?4) ORDER BY t",
            )
            .map_err(db_err)?;
        let events = st
            .query_map(params![char, from, to, if kinds.is_some() { i64::MAX } else { limit }], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        let mut events: Vec<serde_json::Value> = events
            .into_iter()
            .filter(|(k, _)| kinds.is_none_or(|ks| ks.iter().any(|x| x == k)))
            .filter_map(|(_, j)| serde_json::from_str(&j).ok())
            .collect();
        if events.len() > limit as usize {
            events.drain(..events.len() - limit as usize);
        }

        let samples = self.samples(&conn, char, from, to, Some(limit))?;
        Ok(HistoryData { positions, events, samples })
    }

    fn samples(&self, conn: &Connection, char: &str, from: i64, to: i64, limit: Option<i64>) -> Result<Vec<Sample>, String> {
        let mut st = conn
            .prepare_cached(
                "SELECT t, level, xp, xp_max, money, played, map_id, zone FROM (SELECT * FROM samples
                 WHERE char = ?1 AND t BETWEEN ?2 AND ?3 ORDER BY t DESC LIMIT ?4) ORDER BY t",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map(params![char, from, to, limit.unwrap_or(i64::MAX)], row_sample)
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// The session list for a character, newest first.
    pub fn sessions(&self, char: &str) -> Result<Vec<Session>, String> {
        let conn = self.conn();
        let mut st = conn
            .prepare_cached(
                // Same second: login first, logout last.
                "SELECT t, m FROM (
                    SELECT t, 0 AS m FROM positions WHERE char = ?1
                    UNION ALL SELECT t, CASE kind WHEN 'login' THEN 1 WHEN 'logout' THEN 2 ELSE 0 END FROM events WHERE char = ?1
                    UNION ALL SELECT t, 0 FROM samples WHERE char = ?1
                 ) ORDER BY t, CASE m WHEN 1 THEN 0 WHEN 0 THEN 1 ELSE 2 END",
            )
            .map_err(db_err)?;
        let marks = st
            .query_map(params![char], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        let spans = split_sessions(&marks);
        drop(st);

        let mut out = Vec::with_capacity(spans.len());
        for (start, end, logged_out) in spans.into_iter().rev() {
            out.push(self.summarize(&conn, char, start, end, logged_out)?);
        }
        Ok(out)
    }

    /// One session (by its start time) with everything recorded in it.
    pub fn session(&self, char: &str, start: i64) -> Result<Option<SessionDetail>, String> {
        let Some(session) = self.sessions(char)?.into_iter().find(|s| s.start == start) else { return Ok(None) };
        let data = self.range(char, session.start, session.end, None, 50_000)?;
        Ok(Some(SessionDetail { session, data }))
    }

    fn summarize(&self, conn: &Connection, char: &str, start: i64, end: i64, logged_out: bool) -> Result<Session, String> {
        let mut s = Session { start, end, logged_out, ..Default::default() };

        // Level, XP and gold: from the last sample before the session (the
        // state at login) through every sample in it.
        let before = conn
            .query_row(
                "SELECT t, level, xp, xp_max, money, played, map_id, zone FROM samples WHERE char = ?1 AND t < ?2 ORDER BY t DESC LIMIT 1",
                params![char, start],
                row_sample,
            )
            .optional()
            .map_err(db_err)?;
        let mut samples = self.samples(conn, char, start, end, None)?;
        if let Some(b) = before {
            samples.insert(0, b);
        }
        s.xp_gained = xp_gained(&samples);
        s.level_start = samples.iter().find_map(|x| x.level);
        s.level_end = samples.iter().rev().find_map(|x| x.level);
        s.money_start = samples.iter().find_map(|x| x.money);
        s.money_end = samples.iter().rev().find_map(|x| x.money);

        let mut zones: Vec<String> = Vec::new();
        let mut push_zone = |z: &str| {
            if !z.is_empty() && !zones.iter().any(|x| x == z) {
                zones.push(z.to_string());
            }
        };
        for x in samples.iter().filter(|x| x.t >= start) {
            if let Some(z) = &x.zone {
                push_zone(z);
            }
        }

        let mut st = conn
            .prepare_cached("SELECT kind, json FROM events WHERE char = ?1 AND t BETWEEN ?2 AND ?3 ORDER BY t")
            .map_err(db_err)?;
        let rows = st
            .query_map(params![char, start, end], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(db_err)?;
        for row in rows {
            let (kind, json) = row.map_err(db_err)?;
            match kind.as_str() {
                "questDone" => s.quests += 1,
                "kill" => s.kills += 1,
                "death" => s.deaths += 1,
                "loot" => s.loot += 1,
                "level" => s.levels += 1,
                "zone" | "login" => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                        if let Some(z) = v.get("zone").and_then(|z| z.as_str()) {
                            push_zone(z);
                        }
                    }
                }
                _ => {}
            }
        }

        let mut st = conn
            .prepare_cached("SELECT DISTINCT map_id FROM positions WHERE char = ?1 AND t BETWEEN ?2 AND ?3")
            .map_err(db_err)?;
        let ids: BTreeSet<u32> = st
            .query_map(params![char, start, end], |r| r.get(0))
            .map_err(db_err)?
            .collect::<Result<_, _>>()
            .map_err(db_err)?;
        s.map_ids = ids.into_iter().collect();
        s.zones = zones;
        Ok(s)
    }
}

fn insert_sample(conn: &Connection, char: &str, s: &Sample) -> Result<usize, String> {
    conn.prepare_cached("INSERT OR IGNORE INTO samples VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)")
        .map_err(db_err)?
        .execute(params![char, s.t, s.level, s.xp, s.xp_max, s.money, s.played, s.map_id, s.zone])
        .map_err(db_err)
}

fn row_sample(r: &rusqlite::Row) -> rusqlite::Result<Sample> {
    Ok(Sample {
        t: r.get(0)?,
        level: r.get(1)?,
        xp: r.get(2)?,
        xp_max: r.get(3)?,
        money: r.get(4)?,
        played: r.get(5)?,
        map_id: r.get(6)?,
        zone: r.get(7)?,
    })
}

/// Activity this close after a logout (the logout save itself) still
/// belongs to the session that ended.
const LOGOUT_GRACE: i64 = 10;

/// Splits activity times into sessions. `marks` are (time, mark) sorted by
/// time, with mark 1 = login, 2 = logout, 0 = anything else. Returns
/// (start, end, ended with a logout), oldest first.
fn split_sessions(marks: &[(i64, i64)]) -> Vec<(i64, i64, bool)> {
    let mut out: Vec<(i64, i64, bool)> = Vec::new();
    let mut cur: Option<(i64, i64)> = None;
    for &(t, mark) in marks {
        if cur.is_none() && mark != 1 {
            if let Some(last) = out.last_mut().filter(|l| l.2 && t - l.1 <= LOGOUT_GRACE) {
                last.1 = last.1.max(t);
                continue;
            }
        }
        if let Some((start, end)) = cur {
            if t - end > SESSION_GAP || (mark == 1 && t > start) {
                out.push((start, end, false));
                cur = None;
            }
        }
        let (start, end) = cur.unwrap_or((t, t));
        cur = Some((start, end.max(t)));
        if mark == 2 {
            out.push((start, end.max(t), true));
            cur = None;
        }
    }
    if let Some((start, end)) = cur {
        out.push((start, end, false));
    }
    out
}

/// XP gained across consecutive samples, counting level-ups. When several
/// levels passed between two samples, the levels in between are unknown and
/// only the start and end parts are counted.
fn xp_gained(samples: &[Sample]) -> u64 {
    let mut total = 0;
    for w in samples.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let (Some(la), Some(lb), Some(xa), Some(xb)) = (a.level, b.level, a.xp, b.xp) else { continue };
        total += match lb.cmp(&la) {
            std::cmp::Ordering::Equal => xb.saturating_sub(xa),
            std::cmp::Ordering::Greater => a.xp_max.unwrap_or(xa).saturating_sub(xa) + xb,
            std::cmp::Ordering::Less => 0,
        };
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(t: i64, kind: &str) -> JournalEntry {
        JournalEntry { t, kind: kind.into(), ..Default::default() }
    }

    fn sample(t: i64, level: u32, xp: u64, money: u64) -> Sample {
        Sample { t, level: Some(level), xp: Some(xp), xp_max: Some(1000), money: Some(money), ..Default::default() }
    }

    #[test]
    fn resent_saves_add_nothing() {
        let h = History::memory();
        let mut c = Character { key: "A-R".into(), saved_at: Some(100), level: Some(5), ..Default::default() };
        c.trail = vec![TrailPoint { t: 90, map_id: 1429, x: 0.1, y: 0.2 }];
        c.journal = vec![ev(95, "questDone"), ev(95, "kill")];
        assert_eq!(h.record_character(&c).unwrap(), 4);
        assert_eq!(h.record_character(&c).unwrap(), 0, "same save again");
        c.saved_at = Some(200);
        c.journal.push(ev(150, "death"));
        assert_eq!(h.record_character(&c).unwrap(), 2, "one event, one sample");
        let d = h.range("A-R", 0, 1000, None, 100).unwrap();
        assert_eq!((d.positions.len(), d.events.len(), d.samples.len()), (1, 3, 2));
        assert!(h.range("Other-R", 0, 1000, None, 100).unwrap().events.is_empty());
    }

    #[test]
    fn range_filters_and_limits() {
        let h = History::memory();
        h.record_events("A-R", &[ev(1, "kill"), ev(2, "loot"), ev(3, "kill"), ev(4, "kill")]).unwrap();
        let kills = h.range("A-R", 0, 10, Some(&["kill".to_string()]), 2).unwrap().events;
        let ts: Vec<i64> = kills.iter().map(|e| e["t"].as_i64().unwrap()).collect();
        assert_eq!(ts, vec![3, 4], "newest two kills, oldest first");
    }

    #[test]
    fn sessions_split_on_gaps_login_and_logout() {
        let marks = [(100, 1), (200, 0), (300, 2), (302, 0), (400, 0), (500, 0), (500 + SESSION_GAP + 1, 0), (2000, 1), (2100, 0)];
        assert_eq!(
            split_sessions(&marks),
            vec![(100, 302, true), (400, 500, false), (500 + SESSION_GAP + 1, 500 + SESSION_GAP + 1, false), (2000, 2100, false)],
            "the save right after a logout stays in that session"
        );
    }

    #[test]
    fn xp_across_level_up() {
        let s = [sample(0, 5, 900, 0), sample(10, 6, 200, 0), sample(20, 6, 300, 0)];
        assert_eq!(xp_gained(&s), 100 + 200 + 100);
    }

    #[test]
    fn session_summary() {
        let h = History::memory();
        let k = "A-R";
        // Previous session ended at level 5, 900 XP, 1g.
        h.record_sample(k, &sample(10, 5, 900, 10_000)).unwrap();
        h.record_events(k, &[ev(20, "logout")]).unwrap();
        // This session.
        let mut login = ev(5000, "login");
        login.zone = Some("Elwynn Forest".into());
        let mut zone = ev(5200, "zone");
        zone.zone = Some("Westfall".into());
        h.record_events(k, &[login, ev(5100, "kill"), ev(5150, "kill"), ev(5160, "questDone"), ev(5170, "level"), zone]).unwrap();
        h.record_positions(k, &[TrailPoint { t: 5050, map_id: 1429, x: 0.5, y: 0.5 }, TrailPoint { t: 5300, map_id: 1436, x: 0.5, y: 0.5 }], "strip")
            .unwrap();
        h.record_sample(k, &sample(5400, 6, 250, 12_500)).unwrap();

        let list = h.sessions(k).unwrap();
        assert_eq!(list.len(), 2);
        let s = &list[0];
        assert_eq!((s.start, s.end, s.logged_out), (5000, 5400, false));
        assert_eq!((s.level_start, s.level_end), (Some(5), Some(6)));
        assert_eq!(s.xp_gained, 100 + 250);
        assert_eq!((s.money_start, s.money_end), (Some(10_000), Some(12_500)));
        assert_eq!((s.kills, s.quests, s.levels, s.deaths), (2, 1, 1, 0));
        assert_eq!(s.zones, vec!["Elwynn Forest", "Westfall"]);
        assert_eq!(s.map_ids, vec![1429, 1436]);
        assert!(list[1].logged_out);

        let d = h.session(k, 5000).unwrap().unwrap();
        assert_eq!(d.data.positions.len(), 2);
        assert_eq!(d.data.events.len(), 6);
        assert!(h.session(k, 4999).unwrap().is_none());
    }
}
