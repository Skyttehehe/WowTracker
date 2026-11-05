//! Last known state, kept in memory and saved to a JSON file.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use wowtracker_protocol::{Character, IngestPayload, PublicCharacter, PublicState};

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
            }
            if c.trail.len() > trail_max {
                c.trail.drain(..c.trail.len() - trail_max);
            }
            self.characters.insert(c.key.clone(), c);
            changed = true;
        }
        changed
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
                PublicCharacter { character: c.clone(), online }
            })
            .collect();

        PublicState { server_time: now, companion_seen_at: self.companion_seen_at, game_running, characters }
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
}
