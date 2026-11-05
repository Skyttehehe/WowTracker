//! Turns the addon's SavedVariables file (`WowTracker.lua`) into settings and
//! characters.

use crate::lua::{parse_saved_variables, ParseError};
use serde_json::Value;
use wowtracker_protocol::Character;

/// The global the addon saves (`## SavedVariables: WowTrackerDB`).
pub const VARIABLE: &str = "WowTrackerDB";

#[derive(Debug, Default, Clone)]
pub struct AddonData {
    /// Backend address set in-game (`/wt endpoint`), if any.
    pub endpoint: Option<String>,
    /// Ingest token set in-game (`/wt token`), if any.
    pub token: Option<String>,
    pub characters: Vec<Character>,
    /// Characters that could not be read, with the reason.
    pub warnings: Vec<String>,
}

pub fn read_addon_data(src: &str) -> Result<AddonData, ParseError> {
    let vars = parse_saved_variables(src)?;
    let mut data = AddonData::default();
    let Some(Value::Object(db)) = vars.get(VARIABLE) else {
        return Ok(data);
    };

    if let Some(Value::Object(settings)) = db.get("settings") {
        data.endpoint = non_empty(settings.get("endpoint"));
        data.token = non_empty(settings.get("token"));
    }

    if let Some(Value::Object(chars)) = db.get("characters") {
        for (key, value) in chars {
            let mut value = value.clone();
            normalize_character(&mut value);
            match serde_json::from_value::<Character>(value) {
                Ok(mut c) => {
                    if c.key.is_empty() {
                        c.key = key.clone();
                    }
                    data.characters.push(c);
                }
                Err(e) => data.warnings.push(format!("{key}: {e}")),
            }
        }
    }
    Ok(data)
}

fn non_empty(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

/// An empty Lua table `{}` parses as `[]`; where the protocol expects an
/// object, treat it as missing.
fn normalize_character(v: &mut Value) {
    let Value::Object(map) = v else { return };
    for key in ["target", "lastFight"] {
        if map.get(key).is_some_and(is_empty_array) {
            map.remove(key);
        }
    }
    if let Some(Value::Object(fight)) = map.get_mut("lastFight") {
        if fight.get("target").is_some_and(is_empty_array) {
            fight.remove("target");
        }
    }
    // A sparse trail (should not happen) parses as an object; drop it.
    if map.get("trail").is_some_and(|t| !t.is_array()) {
        map.remove("trail");
    }
}

fn is_empty_array(v: &Value) -> bool {
    matches!(v, Value::Array(a) if a.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_settings_and_characters() {
        let src = r#"
WowTrackerDB = {
["settings"] = { ["endpoint"] = " 192.168.0.117:1337 ", ["token"] = "", ["trail"] = true, },
["characters"] = {
["Bob-Firemaw"] = {
["key"] = "Bob-Firemaw", ["name"] = "Bob", ["realm"] = "Firemaw",
["level"] = 23, ["xp"] = 1500, ["xpMax"] = 20000, ["rested"] = 0,
["mapID"] = 1429, ["x"] = 0.5, ["y"] = 0.25, ["hasPosition"] = true,
["inCombat"] = false, ["status"] = "online", ["savedAt"] = 1730000000,
["target"] = { ["name"] = "Kobold Miner", ["level"] = 3, ["hpPct"] = 40, },
["lastFight"] = { ["target"] = {}, ["startedAt"] = 1729999990, },
["trail"] = { { 1729999000, 1429, 0.4, 0.2 }, { 1729999100, 1429, 0.45, 0.22 }, },
},
["Broken-Firemaw"] = { ["level"] = "not a number", },
},
}
"#;
        let data = read_addon_data(src).unwrap();
        assert_eq!(data.endpoint.as_deref(), Some("192.168.0.117:1337"));
        assert_eq!(data.token, None);
        assert_eq!(data.characters.len(), 1);
        assert_eq!(data.warnings.len(), 1);
        let c = &data.characters[0];
        assert_eq!(c.level, Some(23));
        assert_eq!(c.map_id, Some(1429));
        assert_eq!(c.target.as_ref().unwrap().name.as_deref(), Some("Kobold Miner"));
        assert_eq!(c.target.as_ref().unwrap().hp_pct, Some(40.0));
        assert!(c.last_fight.as_ref().unwrap().target.is_none());
        assert_eq!(c.trail.len(), 2);
        assert_eq!(c.trail[1].x, 0.45);
    }

    #[test]
    fn missing_variable_is_empty() {
        let data = read_addon_data("SomethingElse = 1").unwrap();
        assert!(data.characters.is_empty());
        assert!(data.endpoint.is_none());
    }
}
