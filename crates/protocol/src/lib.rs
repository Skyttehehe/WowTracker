//! Data sent from the companion to the backend, and from the backend to the
//! website. Field names match the keys the addon writes into SavedVariables
//! (camelCase), so a parsed SavedVariables table deserializes straight into
//! [`Character`].

use serde::{Deserialize, Serialize};

/// One character, as recorded by the addon.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Character {
    /// `Name-Realm`.
    pub key: String,
    pub name: String,
    pub realm: String,
    /// `Player-<server>-<id>`, used to match combat log lines.
    pub guid: Option<String>,
    pub class: Option<String>,
    /// Locale-independent class token, e.g. `WARRIOR`.
    pub class_file: Option<String>,
    pub race: Option<String>,
    pub faction: Option<String>,

    pub level: Option<u32>,
    pub xp: Option<u64>,
    pub xp_max: Option<u64>,
    pub rested: Option<u64>,
    pub resting: Option<bool>,
    pub xp_gained_session: Option<u64>,

    /// Last known uiMapID and normalized (0..1) position on that map.
    #[serde(rename = "mapID")]
    pub map_id: Option<u32>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// False when the game hides the position (dungeons, battlegrounds).
    pub has_position: Option<bool>,
    pub position_at: Option<i64>,
    pub zone: Option<String>,
    pub subzone: Option<String>,
    /// Instance type ("party", "raid", "pvp", ...) when inside one.
    pub instance: Option<String>,

    pub in_combat: Option<bool>,
    pub target: Option<Unit>,
    pub last_fight: Option<Fight>,

    /// "online" or "offline", as of the last save.
    pub status: Option<String>,
    pub login_at: Option<i64>,
    pub saved_at: Option<i64>,

    pub trail: Vec<TrailPoint>,

    /// Latest position read from the combat log. Set by the backend, not the addon.
    pub live: Option<LivePosition>,
}

/// The player's position from the game's combat log (advanced logging on).
/// The companion only ever reads the player's own lines from the log.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LivePosition {
    /// When the logged event happened (Unix seconds).
    pub t: i64,
    /// The player's GUID as written in the log.
    pub guid: Option<String>,
    /// `Name-Realm` as written in the log.
    pub name: Option<String>,
    #[serde(rename = "mapID")]
    pub map_id: u32,
    /// Map name from the log's MAP_CHANGE line.
    pub map: Option<String>,
    /// Normalized (0..1) position on `map_id`.
    pub x: f64,
    pub y: f64,
    /// Name of the creature in that line (never another player).
    pub opponent: Option<String>,
    /// Earlier positions since the previous update, oldest first. The backend
    /// moves these into the character's trail and stores this list empty.
    pub trail: Vec<TrailPoint>,
}

/// A target / opponent snapshot. Name, creature type and health can be
/// missing because the game hides them from addons during combat.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Unit {
    pub name: Option<String>,
    /// -1 means "??" (skull / boss).
    pub level: Option<i32>,
    pub classification: Option<String>,
    pub creature_type: Option<String>,
    pub reaction: Option<u8>,
    pub is_player: Option<bool>,
    pub dead: Option<bool>,
    pub hp_pct: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Fight {
    pub target: Option<Unit>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

/// One breadcrumb. The addon stores these as `{ time, mapID, x, y }` arrays,
/// which serde accepts for a struct; it is sent on as an object.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrailPoint {
    pub t: i64,
    #[serde(rename = "mapID")]
    pub map_id: u32,
    pub x: f64,
    pub y: f64,
}

/// Body of `POST /ingest`. An empty `characters` list is a heartbeat.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IngestPayload {
    pub companion_version: String,
    pub sent_at: i64,
    /// Whether a WoW process is running on the player's PC.
    pub game_running: Option<bool>,
    pub characters: Vec<Character>,
    /// New position from the combat log, if any.
    pub live: Option<LivePosition>,
}

/// Body of the public `GET /api/state`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PublicState {
    pub server_time: i64,
    /// Last time the companion was heard from.
    pub companion_seen_at: Option<i64>,
    pub game_running: bool,
    /// Most recently saved first.
    pub characters: Vec<PublicCharacter>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PublicCharacter {
    #[serde(flatten)]
    pub character: Character,
    /// Logged in right now, as far as the backend can tell.
    pub online: bool,
    /// Where `mapID`/`x`/`y`/`positionAt` came from: "addon" or "combatLog".
    pub position_source: Option<String>,
}
