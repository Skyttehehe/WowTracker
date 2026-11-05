//! Runs addon/WowTracker/WowTracker.lua in Lua 5.1 (the version WoW uses)
//! against a mocked WoW API, then reads the SavedVariables it would write with
//! the companion's parser.

use mlua::Lua;
use wowtracker_companion::savedvars::{read_addon_data, AddonData};

const MOCK: &str = include_str!("../lua/mock_wow.lua");
const ADDON: &str = include_str!("../../../addon/WowTracker/WowTracker.lua");

fn load_addon() -> Lua {
    let lua = Lua::new();
    lua.load(MOCK).set_name("mock_wow.lua").exec().expect("mock loads");
    let ns = lua.create_table().unwrap();
    lua.load(ADDON)
        .set_name("WowTracker.lua")
        .call::<()>(("WowTracker", ns))
        .expect("addon loads");
    lua
}

fn run(lua: &Lua, script: &str) {
    if let Err(e) = lua.load(script).set_name("scenario").exec() {
        panic!("Lua error: {e}");
    }
}

fn saved(lua: &Lua) -> (String, AddonData) {
    let text: String = lua.load("return Mock.saved_variables()").eval().expect("serialize");
    let data = read_addon_data(&text).unwrap_or_else(|e| panic!("companion cannot parse:\n{text}\n{e}"));
    assert!(data.warnings.is_empty(), "warnings: {:?}\n{text}", data.warnings);
    (text, data)
}

const LOGIN: &str = r#"
    Mock.fire("ADDON_LOADED", "WowTracker")
    Mock.fire("PLAYER_LOGIN")
    Mock.fire("PLAYER_ENTERING_WORLD", true, false)
"#;

#[test]
fn full_session_with_reload() {
    let lua = load_addon();
    run(&lua, LOGIN);
    run(
        &lua,
        r#"
        -- walk around Elwynn, then into Westfall
        for i = 1, 5 do Mock.map.x = Mock.map.x + 0.01; Mock.update(11) end
        Mock.map.id, Mock.map.zone, Mock.map.subzone = 1436, "Westfall", "Sentinel Hill"
        Mock.map.x, Mock.map.y = 0.56, 0.47
        Mock.fire("ZONE_CHANGED_NEW_AREA")
        Mock.update(11)

        -- target a mob before the pull, then fight it
        Mock.target = { name = "Defias Thug", level = 11, classification = "normal",
                        creatureType = "Humanoid", reaction = 2, hp = 150, hpMax = 200 }
        Mock.fire("PLAYER_TARGET_CHANGED")
        Mock.inCombat = true
        Mock.fire("PLAYER_REGEN_DISABLED")
        Mock.target.hp = 40
        Mock.fire("UNIT_HEALTH", "target")   -- health is secret now; must not error
        Mock.target.hp = 0
        Mock.inCombat = false
        Mock.fire("PLAYER_REGEN_ENABLED")

        -- XP and a level up
        Mock.player.xp = 6900; Mock.fire("PLAYER_XP_UPDATE", "player")
        Mock.player.level, Mock.player.xp, Mock.player.xpMax = 11, 200, 7500
        Mock.fire("PLAYER_LEVEL_UP", 11); Mock.fire("PLAYER_XP_UPDATE", "player")

        -- settings from chat
        SlashCmdList.WOWTRACKER("endpoint   192.168.0.117:1337  ")
        SlashCmdList.WOWTRACKER("token s3cret")
        SlashCmdList.WOWTRACKER("status")
        SlashCmdList.WOWTRACKER("config")
        SlashCmdList.WOWTRACKER("")

        -- /wt sync -> reload -> PLAYER_LOGOUT
        SlashCmdList.WOWTRACKER("sync")
        assert(Mock.reloadRequested, "sync should reload the UI")
        Mock.fire("PLAYER_LOGOUT")
        "#,
    );

    let (text, data) = saved(&lua);
    assert_eq!(data.endpoint.as_deref(), Some("192.168.0.117:1337"));
    assert_eq!(data.token.as_deref(), Some("s3cret"));
    assert_eq!(data.characters.len(), 1, "{text}");

    let c = &data.characters[0];
    assert_eq!(c.key, "Testchar-Firemaw");
    assert_eq!(c.class_file.as_deref(), Some("WARRIOR"));
    assert_eq!(c.guid.as_deref(), Some("Player-6940-00AF5B76"), "GUID for matching combat log lines");
    assert_eq!(c.level, Some(11));
    assert_eq!(c.xp, Some(200));
    assert_eq!(c.xp_max, Some(7500));
    assert_eq!(c.rested, Some(1200));
    assert_eq!(c.xp_gained_session, Some(6400 + 100 + 200));
    assert_eq!(c.map_id, Some(1436));
    assert_eq!(c.zone.as_deref(), Some("Westfall"));
    assert_eq!(c.x, Some(0.56));
    assert_eq!(c.has_position, Some(true));
    assert_eq!(c.status.as_deref(), Some("online"), "a reload must not mark the character offline");
    assert_eq!(c.in_combat, Some(false));
    assert!(c.saved_at.is_some());

    let fight = c.last_fight.as_ref().expect("lastFight");
    let foe = fight.target.as_ref().expect("fight target");
    assert_eq!(foe.name.as_deref(), Some("Defias Thug"), "name captured before the pull");
    assert_eq!(foe.level, Some(11));
    assert_eq!(foe.dead, Some(true), "outcome read after combat");
    assert!(fight.started_at.is_some() && fight.ended_at.is_some());

    let target = c.target.as_ref().expect("target");
    assert_eq!(target.dead, Some(true));
    assert_eq!(target.hp_pct, Some(0.0));

    assert!(c.trail.len() >= 5, "trail: {:?}", c.trail);
    assert_eq!(c.trail.last().unwrap().map_id, 1436);
    // Coordinates are rounded to 4 decimals.
    assert_eq!(c.trail[0].x, 0.4123);
}

#[test]
fn logout_marks_offline() {
    let lua = load_addon();
    run(&lua, LOGIN);
    run(&lua, r#"Mock.fire("PLAYER_LOGOUT")"#);
    let (_, data) = saved(&lua);
    assert_eq!(data.characters[0].status.as_deref(), Some("offline"));
    assert_eq!(data.endpoint, None);
}

#[test]
fn plain_reload_counts_as_online() {
    let lua = load_addon();
    run(&lua, LOGIN);
    run(&lua, r#"C_UI.Reload(); Mock.fire("PLAYER_LOGOUT")"#);
    let (_, data) = saved(&lua);
    assert_eq!(data.characters[0].status.as_deref(), Some("online"));
}

#[test]
fn instance_keeps_last_position() {
    let lua = load_addon();
    run(&lua, LOGIN);
    run(
        &lua,
        r#"
        Mock.map.instance = "party"; Mock.map.zone = "The Deadmines"; Mock.map.subzone = ""
        Mock.fire("ZONE_CHANGED_NEW_AREA")
        Mock.update(11)
        Mock.fire("PLAYER_LOGOUT")
        "#,
    );
    let (_, data) = saved(&lua);
    let c = &data.characters[0];
    assert_eq!(c.has_position, Some(false));
    assert_eq!(c.instance.as_deref(), Some("party"));
    assert_eq!(c.zone.as_deref(), Some("The Deadmines"));
    assert_eq!(c.map_id, Some(1429), "last open-world map is kept");
}

#[test]
fn target_picked_up_in_combat_has_no_secrets() {
    let lua = load_addon();
    run(&lua, LOGIN);
    run(
        &lua,
        r#"
        Mock.inCombat = true
        Mock.fire("PLAYER_REGEN_DISABLED")
        Mock.target = { name = "Hogger", level = 11, classification = "elite",
                        creatureType = "Humanoid", reaction = 2, hp = 500, hpMax = 600 }
        Mock.fire("PLAYER_TARGET_CHANGED")
        Mock.fire("UNIT_HEALTH", "target")
        Mock.fire("PLAYER_LOGOUT")   -- logging out mid-fight
        "#,
    );
    let (_, data) = saved(&lua);
    let c = &data.characters[0];
    assert_eq!(c.in_combat, Some(true));
    let t = c.target.as_ref().unwrap();
    assert_eq!(t.name, None, "hidden in combat");
    assert_eq!(t.hp_pct, None, "hidden in combat");
    assert_eq!(t.level, Some(11));
    assert_eq!(t.classification.as_deref(), Some("elite"));
}

/// Writes a demo SavedVariables file for trying the companion, backend and
/// website without the game:
///   WOWTRACKER_DEMO_OUT=<file> cargo test -p addon-harness -- --ignored demo
/// Optional: WOWTRACKER_DEMO_ENDPOINT / WOWTRACKER_DEMO_TOKEN set the in-game settings.
#[test]
#[ignore]
fn demo_saved_variables() {
    let out = std::env::var("WOWTRACKER_DEMO_OUT").expect("set WOWTRACKER_DEMO_OUT");
    let endpoint = std::env::var("WOWTRACKER_DEMO_ENDPOINT").unwrap_or_default();
    let token = std::env::var("WOWTRACKER_DEMO_TOKEN").unwrap_or_default();
    let lua = load_addon();
    lua.globals().set("DEMO_ENDPOINT", endpoint).unwrap();
    lua.globals().set("DEMO_TOKEN", token).unwrap();
    run(&lua, "Mock.clock = os.time() - 3600");
    run(&lua, LOGIN);
    run(
        &lua,
        r#"
        if DEMO_ENDPOINT ~= "" then SlashCmdList.WOWTRACKER("endpoint " .. DEMO_ENDPOINT) end
        if DEMO_TOKEN ~= "" then SlashCmdList.WOWTRACKER("token " .. DEMO_TOKEN) end
        -- Goldshire, south-west down the road and over the bridge into Westfall
        local route = {
          {1429, 0.42, 0.65}, {1429, 0.40, 0.68}, {1429, 0.36, 0.72}, {1429, 0.31, 0.75},
          {1429, 0.26, 0.78}, {1429, 0.21, 0.80}, {1429, 0.16, 0.81}, {1429, 0.10, 0.82},
          {1429, 0.05, 0.82}, {1436, 0.69, 0.48}, {1436, 0.66, 0.50}, {1436, 0.62, 0.52},
          {1436, 0.59, 0.51}, {1436, 0.57, 0.48}, {1436, 0.56, 0.47},
        }
        for _, p in ipairs(route) do
          Mock.map.id, Mock.map.x, Mock.map.y = p[1], p[2], p[3]
          if p[1] == 1436 then Mock.map.zone, Mock.map.subzone = "Westfall", "Sentinel Hill" end
          Mock.update(120)
        end
        Mock.player.level, Mock.player.xp, Mock.player.xpMax = 11, 2950, 7500
        Mock.fire("PLAYER_XP_UPDATE", "player")
        Mock.target = { name = "Defias Trapper", level = 12, classification = "normal",
                        creatureType = "Humanoid", reaction = 2, hp = 180, hpMax = 210 }
        Mock.fire("PLAYER_TARGET_CHANGED")
        Mock.inCombat = true; Mock.fire("PLAYER_REGEN_DISABLED")
        Mock.update(14)
        Mock.target.hp = 0; Mock.inCombat = false; Mock.fire("PLAYER_REGEN_ENABLED")
        Mock.update(30)
        Mock.target = { name = "Harvest Golem", level = 13, classification = "elite",
                        creatureType = "Mechanical", reaction = 2, hp = 640, hpMax = 900 }
        Mock.fire("PLAYER_TARGET_CHANGED")
        Mock.clock = os.time()
        C_UI.Reload()
        Mock.fire("PLAYER_LOGOUT")
        "#,
    );
    let (text, _) = saved(&lua);
    std::fs::write(&out, text).unwrap();
    println!("wrote {out}");
}

#[test]
fn second_login_keeps_settings_and_trail() {
    let lua = load_addon();
    run(&lua, LOGIN);
    run(
        &lua,
        r#"
        SlashCmdList.WOWTRACKER("endpoint 10.0.0.5:1337")
        for i = 1, 3 do Mock.map.x = Mock.map.x + 0.02; Mock.update(11) end
        Mock.fire("PLAYER_LOGOUT")
        "#,
    );
    let (text, first) = saved(&lua);
    let points = first.characters[0].trail.len();

    // New session: SavedVariables are loaded from the file before ADDON_LOADED.
    let lua = Lua::new();
    lua.load(MOCK).exec().unwrap();
    lua.load(&text).exec().expect("the saved file is valid Lua");
    lua.load(ADDON).call::<()>(("WowTracker", lua.create_table().unwrap())).unwrap();
    run(&lua, LOGIN);
    run(&lua, r#"Mock.fire("PLAYER_LOGOUT")"#);
    let (_, second) = saved(&lua);
    assert_eq!(second.endpoint.as_deref(), Some("10.0.0.5:1337"));
    assert!(second.characters[0].trail.len() >= points);
    assert_eq!(second.characters[0].xp_gained_session, Some(0), "session counter resets on login");
}
