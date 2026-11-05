//! Last known state, kept in memory and saved to a JSON file.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use wowtracker_protocol::{Character, IngestPayload, LivePosition, PublicCharacter, PublicState, TrailPoint};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Store {
    pub characters: BTreeMap<String, Character>,
    pub companion_seen_at: Option<i64>,
    pub game_running: Option<bool>,
}

impl Store {
    pub fn load(path: &Path) -> Store {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("warning: ignoring unreadable {}: {e}", path.display());
                Store::default()
            }),
            Err(_) => Store::default(),
        }
    }

    /// Applies a payload. Returns true when character data changed.
    pub fn apply(&mut self, payload: IngestPayload, now: i64, trail_max: usize) -> bool {
        self.companion_seen_at = Some(now);
        if payload.game_running.is_some() {
            self.game_running = payload.game_running;
        }
        let mut changed = false;
        for mut c in payload.characters {
            if c.key.trim().is_empty() {
                continue;
            }
            // Never let an older save overwrite a newer one.
            if let Some(existing) = self.characters.get(&c.key) {
                if existing.saved_at.unwrap_or(0) > c.saved_at.unwrap_or(0) {
                    continue;
                }
                // Keep what the combat log added after this save's own data.
                c.live = existing.live.clone();
                let newest = c.trail.last().map(|p| p.t).unwrap_or(0).max(c.position_at.unwrap_or(0));
                c.trail.extend(existing.trail.iter().filter(|p| p.t > newest).copied());
            }
            cap(&mut c.trail, trail_max);
            self.characters.insert(c.key.clone(), c);
            changed = true;
        }
        if let Some(live) = payload.live {
            changed |= self.apply_live(live, trail_max);
        }
        changed
    }

    /// The character a live position belongs to: by GUID when the addon has
    /// recorded one, else the most recently saved character.
    pub fn live_target(&self, live: &LivePosition) -> Option<String> {
        self.characters
            .values()
            .find(|c| c.guid.is_some() && c.guid == live.guid)
            .or_else(|| self.characters.values().max_by_key(|c| c.saved_at.unwrap_or(0)))
            .map(|c| c.key.clone())
    }

    /// Attaches a combat log position to its character.
    fn apply_live(&mut self, mut live: LivePosition, trail_max: usize) -> bool {
        let key = self.live_target(&live);
        let Some(c) = key.and_then(|k| self.characters.get_mut(&k)) else { return false };
        if c.live.as_ref().is_some_and(|l| l.t >= live.t) {
            return false;
        }
        let last_t = c.trail.last().map(|p| p.t).unwrap_or(i64::MIN);
        let mut points = std::mem::take(&mut live.trail);
        points.push(TrailPoint { t: live.t, map_id: live.map_id, x: live.x, y: live.y });
        c.trail.extend(points.into_iter().filter(|p| p.t > last_t));
        cap(&mut c.trail, trail_max);
        c.live = Some(live);
        true
    }

    /// What the website sees. A character is online when it is the most
    /// recently saved one, its last save said "online" (a /reload, not a
    /// logout), the companion is still reporting and the game is running.
    pub fn public_state(&self, now: i64, offline_after: i64) -> PublicState {
        let companion_fresh = self.companion_seen_at.is_some_and(|t| now - t <= offline_after);
        let game_running = companion_fresh && self.game_running != Some(false);

        let mut chars: Vec<&Character> = self.characters.values().collect();
        chars.sort_by_key(|c| std::cmp::Reverse(c.saved_at.unwrap_or(0)));

        // Only the most recently saved character can be the one being played.
        let characters = chars
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let online = i == 0 && game_running && c.status.as_deref() == Some("online");
                let (character, source) = with_newest_position(c);
                PublicCharacter { character, online, position_source: source }
            })
            .collect();

        PublicState { server_time: now, companion_seen_at: self.companion_seen_at, game_running, characters }
    }
}

/// Uses the combat log position when it is newer than the addon's.
fn with_newest_position(c: &Character) -> (Character, Option<String>) {
    let mut out = c.clone();
    let addon_t = c.position_at.or(c.saved_at).unwrap_or(0);
    match &c.live {
        Some(l) if l.t > addon_t => {
            let moved = match (c.x, c.y) {
                (Some(x), Some(y)) => ((x - l.x).powi(2) + (y - l.y).powi(2)).sqrt() > 0.02,
                _ => true,
            };
            if c.map_id != Some(l.map_id) {
                // Moved to another map since the save.
                out.zone = l.map.clone().or(out.zone);
                out.instance = None;
            }
            if c.map_id != Some(l.map_id) || moved {
                // The log has no subzone, and the saved one may be wrong now.
                out.subzone = None;
            }
            out.map_id = Some(l.map_id);
            out.x = Some(l.x);
            out.y = Some(l.y);
            out.position_at = Some(l.t);
            out.has_position = Some(true);
            (out, Some("combatLog".into()))
        }
        _ => (out, c.map_id.map(|_| "addon".into())),
    }
}

fn cap(trail: &mut Vec<TrailPoint>, max: usize) {
    if trail.len() > max {
        trail.drain(..trail.len() - max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(key: &str, saved_at: i64, status: &str) -> Character {
        Character { key: key.into(), saved_at: Some(saved_at), status: Some(status.into()), ..Default::default() }
    }

    fn payload(chars: Vec<Character>, running: Option<bool>) -> IngestPayload {
        IngestPayload { game_running: running, characters: chars, ..Default::default() }
    }

    #[test]
    fn older_save_does_not_overwrite() {
        let mut s = Store::default();
        assert!(s.apply(payload(vec![ch("A-R", 200, "online")], Some(true)), 1000, 10));
        assert!(!s.apply(payload(vec![ch("A-R", 100, "offline")], Some(true)), 1001, 10));
        assert_eq!(s.characters["A-R"].saved_at, Some(200));
    }

    #[test]
    fn heartbeat_keeps_characters() {
        let mut s = Store::default();
        s.apply(payload(vec![ch("A-R", 200, "online")], Some(true)), 1000, 10);
        assert!(!s.apply(payload(vec![], Some(false)), 1010, 10));
        assert_eq!(s.characters.len(), 1);
        assert_eq!(s.game_running, Some(false));
    }

    #[test]
    fn online_rules() {
        let mut s = Store::default();
        s.apply(payload(vec![ch("Old-R", 100, "online"), ch("New-R", 200, "online")], Some(true)), 1000, 10);
        let st = s.public_state(1000, 120);
        assert_eq!(st.characters[0].character.key, "New-R");
        assert!(st.characters[0].online);
        assert!(!st.characters[1].online, "only the latest character can be online");

        // Companion silent for too long -> offline.
        assert!(!s.public_state(1200, 120).characters[0].online);

        // Game closed -> offline.
        s.apply(payload(vec![], Some(false)), 1001, 10);
        assert!(!s.public_state(1001, 120).characters[0].online);

        // Logged out -> offline even if the game is still open.
        s.apply(payload(vec![ch("New-R", 300, "offline")], Some(true)), 1002, 10);
        let st = s.public_state(1002, 120);
        assert!(st.characters.iter().all(|c| !c.online));
    }

    #[test]
    fn trail_is_capped() {
        let mut s = Store::default();
        let mut c = ch("A-R", 1, "online");
        c.trail = (0..50).map(|i| wowtracker_protocol::TrailPoint { t: i, map_id: 1429, x: 0.5, y: 0.5 }).collect();
        s.apply(payload(vec![c], None), 1, 10);
        let t = &s.characters["A-R"].trail;
        assert_eq!(t.len(), 10);
        assert_eq!(t[0].t, 40);
    }

    fn live(t: i64, guid: Option<&str>, x: f64) -> LivePosition {
        LivePosition {
            t,
            guid: guid.map(String::from),
            map_id: 1438,
            map: Some("Teldrassil".into()),
            x,
            y: 0.5,
            opponent: Some("Webwood Lurker".into()),
            trail: vec![TrailPoint { t: t - 10, map_id: 1438, x: x - 0.01, y: 0.5 }],
            ..Default::default()
        }
    }

    fn with_live(l: LivePosition) -> IngestPayload {
        IngestPayload { live: Some(l), ..Default::default() }
    }

    fn placed(key: &str, saved_at: i64, guid: Option<&str>) -> Character {
        let mut c = ch(key, saved_at, "online");
        c.guid = guid.map(String::from);
        c.map_id = Some(1429);
        c.zone = Some("Elwynn Forest".into());
        c.subzone = Some("Northshire".into());
        c.x = Some(0.4);
        c.y = Some(0.6);
        c.position_at = Some(saved_at);
        c.trail = vec![TrailPoint { t: saved_at, map_id: 1429, x: 0.4, y: 0.6 }];
        c
    }

    #[test]
    fn combat_log_position_wins_when_newer() {
        let mut s = Store::default();
        s.apply(payload(vec![placed("A-R", 100, None)], Some(true)), 100, 50);
        assert!(s.apply(with_live(live(200, Some("Player-1-X"), 0.6)), 200, 50));

        let c = &s.public_state(200, 120).characters[0];
        assert_eq!(c.position_source.as_deref(), Some("combatLog"));
        assert_eq!((c.character.map_id, c.character.x), (Some(1438), Some(0.6)));
        assert_eq!(c.character.zone.as_deref(), Some("Teldrassil"));
        assert_eq!(c.character.subzone, None, "old subzone dropped after a map change");

        // Same map, barely moved: the saved subzone still applies.
        let mut s2 = Store::default();
        s2.apply(payload(vec![placed("B-R", 100, None)], Some(true)), 100, 50);
        let mut near = live(200, None, 0.405);
        near.map_id = 1429;
        near.y = 0.6;
        s2.apply(with_live(near), 200, 50);
        assert_eq!(s2.public_state(200, 120).characters[0].character.subzone.as_deref(), Some("Northshire"));
        let mut far = live(300, None, 0.6);
        far.map_id = 1429;
        s2.apply(with_live(far), 300, 50);
        assert_eq!(s2.public_state(300, 120).characters[0].character.subzone, None, "moved away: subzone unknown");
        assert_eq!(c.character.position_at, Some(200));
        // Breadcrumb and position were added to the trail, in order.
        let t: Vec<i64> = s.characters["A-R"].trail.iter().map(|p| p.t).collect();
        assert_eq!(t, vec![100, 190, 200]);
        assert!(s.characters["A-R"].live.as_ref().unwrap().trail.is_empty(), "breadcrumbs are not stored twice");

        // An older or repeated update changes nothing.
        assert!(!s.apply(with_live(live(200, None, 0.7)), 201, 50));
        assert!(!s.apply(with_live(live(150, None, 0.7)), 201, 50));
    }

    #[test]
    fn newer_addon_save_wins_and_keeps_combat_log_trail() {
        let mut s = Store::default();
        s.apply(payload(vec![placed("A-R", 100, None)], Some(true)), 100, 50);
        s.apply(with_live(live(200, None, 0.6)), 200, 50);
        // A /reload at 150 (older than the combat log position) keeps the live one.
        s.apply(payload(vec![placed("A-R", 150, None)], Some(true)), 210, 50);
        let c = &s.public_state(210, 120).characters[0];
        assert_eq!(c.position_source.as_deref(), Some("combatLog"));
        let t: Vec<i64> = s.characters["A-R"].trail.iter().map(|p| p.t).collect();
        assert_eq!(t, vec![150, 190, 200], "combat log points after the save survive it");

        // A later save is shown instead.
        s.apply(payload(vec![placed("A-R", 300, None)], Some(true)), 300, 50);
        let c = &s.public_state(300, 120).characters[0];
        assert_eq!(c.position_source.as_deref(), Some("addon"));
        assert_eq!(c.character.map_id, Some(1429));
        assert_eq!(c.character.subzone.as_deref(), Some("Northshire"));
    }

    #[test]
    fn combat_log_matches_by_guid() {
        let mut s = Store::default();
        s.apply(
            payload(vec![placed("Main-R", 100, Some("Player-1-MAIN")), placed("Alt-R", 200, Some("Player-1-ALT"))], Some(true)),
            200,
            50,
        );
        s.apply(with_live(live(300, Some("Player-1-MAIN"), 0.6)), 300, 50);
        assert!(s.characters["Main-R"].live.is_some());
        assert!(s.characters["Alt-R"].live.is_none(), "not the most recent one: the GUID decides");

        // Unknown GUID (addon not updated yet): the most recently saved character.
        s.apply(with_live(live(400, Some("Player-1-OTHER"), 0.6)), 400, 50);
        assert_eq!(s.characters["Alt-R"].live.as_ref().map(|l| l.t), Some(400));
    }
}
