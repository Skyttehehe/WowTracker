# Live character tracker for "WoW Classic Forever": research notes

Researched 2026-11-05, the day after World of Warcraft: Forever launched. Every claim cites a URL. The UI source facts come from Blizzard's exported interface code, mirrored at `Gethe/wow-ui-source`, branch `forever`, build 1.60.1.70790, commit dated 2026-11-04. The DB2 facts come from wago.tools for that same build. Anything not checked against this client is labelled **UNVERIFIED**.

---

## TL;DR

- **The product.** "WoW Classic Forever" is almost certainly **World of Warcraft: Forever**. Blizzard announced it at BlizzCon 2026 as its "Classic+" and launched it on 2026-11-04/05. It is a separate game branch alongside retail and Classic. It is not a rename of the Classic Era or Anniversary realms ([wiki](https://warcraft.wiki.gg/wiki/Patch_1.60)).
  - Client: **1.60.1** (live build 70790), product code `wowf` ([wow-ui-source](https://github.com/Gethe/wow-ui-source/tree/forever), [wago.tools builds](https://wago.tools/api/builds)).
  - **TOC `## Interface: 16001`** ([wiki API changes](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes)).
- **It runs on the modern (Midnight/retail) client, not the 1.15 Classic Era client.** Its game type is `forever`/`camelot` in the `Mainline` family. **Blizzard confirmed that Midnight's "addon disarmament" applies, including secret values** ([wiki, quoting the 2026-09-15 Blizzard post](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes)). Classic Era addon knowledge partly does not apply.
- **Available as plain values (no secret flag in this build's API docs):**
  - Zone, map ID and x/y. These return nil in instances, battlegrounds and arenas.
  - Level, XP, max XP and rested XP.
  - In-combat flag.
  - Target level and classification.
- **Not reliably available to an addon in combat:**
  - Target name, creature type, health and max health. These are secret in combat, and for NPCs possibly more often.
  - `COMBAT_LOG_EVENT_UNFILTERED`, which is removed for addons.

  The "what am I fighting" fields will often be blank from the addon side.
- **Addons cannot open sockets, do HTTP or write files.** Their only disk output is SavedVariables, written on `/reload`, logout or exit ([wiki](https://warcraft.wiki.gg/wiki/Lua_functions), [wiki](https://warcraft.wiki.gg/wiki/Saved_variables)). Something outside the game has to pick the data up.
- **Combat log file (`/combatlog` + Advanced Combat Logging):**
  - Still written in the Midnight model, and it carries the combat details addons lose: names, HP, level, position, uiMapID.
  - It is only written for combat events, so it does not cover travel.
  - The client buffers writes. Reports range from seconds to "several minutes" ([forum](https://us.forums.blizzard.com/en/blizzard/t/wow-wow-combat-log-delay-%E2%80%93-seeking-api-or-workaround/55869)). Measure it yourself.
- **No first-party live data route exists.** As of 2026-10-26 there is no documented Battle.net API namespace for Forever. Even where profile APIs exist, they update at logout and have no position.
- **Recommended architecture (if you accept the ToS grey area):**
  - The addon gathers the non-secret fields and **draws them as a small strip of coloured pixels**.
  - A companion app captures only that screen region, decodes it, and POSTs it over HTTPS to wowtracker.osrs.dk.
  - The server fans it out to browsers over SSE or WebSocket. A Leaflet `CRS.Simple` map projects it with UiMapAssignment bounds.
  - Optionally, tail the combat log to fill in "what am I fighting".
- **Main risk: Blizzard's EULA.** It prohibits "any unauthorized process or software that intercepts, collects, reads, or 'mines' information" from the game ([EULA](https://www.blizzard.com/en-us/legal/fba4d00f-c7e4-4883-b8b9-1b4500a402ea/blizzard-end-user-license-agreement)).
  - Pixel export is not automation and does not touch game memory.
  - However, it is the same channel pixel bots use, and Blizzard has not blessed it.
  - **Never encode secret values** (for example via colour curves). That would be deliberately circumventing a protection Blizzard just added.
- **Zero-risk fallback:** SavedVariables plus a file watcher. This gives "last seen" data on each `/reload` or logout, not a live map.

---

## 1. What "WoW Classic Forever" is

| Fact | Value | Source |
|---|---|---|
| Official name | *World of Warcraft: Forever*. Community name "Classic+", internal codename "Camelot" | [warcraft.wiki.gg Patch 1.60](https://warcraft.wiki.gg/wiki/Patch_1.60) |
| Launch | 2026-11-04 (NA evening) / 11-05, 23:00 UTC | [warcraft.wiki.gg Patch 1.60](https://warcraft.wiki.gg/wiki/Patch_1.60) |
| Relationship | "a standalone branch of the MMO, running concurrently with both retail and Classic". A level-60-capped game with new and revamped zones, not the Classic Era or Anniversary realms | [wiki](https://warcraft.wiki.gg/wiki/Patch_1.60), [Windows Central](https://www.windowscentral.com/gaming/blizzard/world-of-warcraft-forever-is-blizzards-take-on-classic-revamping-vanilla-azeroth) |
| Client version | **1.60.1.70790** (wow-ui-source `forever` branch, commit "1.60.1 (70790)", 2026-11-04) | [Gethe/wow-ui-source@forever](https://github.com/Gethe/wow-ui-source/tree/forever), [version.txt](https://raw.githubusercontent.com/Gethe/wow-ui-source/forever/version.txt) |
| Battle.net product code | `wowf` (wago.tools lists `wowf` at 1.60.1.70790) | [wago.tools/api/builds](https://wago.tools/api/builds) |
| TOC Interface | **16001**. The API-changes page lists TOC 16001 and detects Forever with `version > 16000 and version < 20000`. The TOC_format table lists 16001 for "Forever Beta/Test" and had no value yet for live | [Patch 1.60.1/API changes](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes), [TOC format](https://warcraft.wiki.gg/wiki/TOC_format) |
| Game type / family | Game type `forever` (alias of `camelot`, added in build 70291). Family `Mainline`, shared with retail Midnight. `WOW_PROJECT_ID == WOW_PROJECT_CAMELOT` (18) | [Patch 1.60.1/API changes](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes), [TOC format](https://warcraft.wiki.gg/wiki/TOC_format) |
| Code base | Modern UI. The `forever` branch contains Blizzard_DamageMeter, Blizzard_CooldownViewer, `Secret*` API docs, etc. It is diffed against 12.1.5 | [forever tree](https://github.com/Gethe/wow-ui-source/tree/forever/Interface/AddOns), [Blizzard_DamageMeter.toc](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_DamageMeter/Blizzard_DamageMeter.toc) |
| Addon restrictions | "The addon disarmament changes introduced in Midnight, including secrets, will also be active in Forever." Blizzard said this on the WoW UI Discord on 2026-09-15, as quoted by the wiki and reported by Wowhead | [Patch 1.60.1/API changes](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes), [Wowhead report](https://www.wowhead.com/news/wow-forever-will-have-addon-changes-from-midnight-382921) (via [Warperia](https://warperia.com/wow-forever-inherits-midnights-addon-changes/)) |
| Install folder | `World of Warcraft\_forever_\` (**UNVERIFIED**: from a fan guide only. Check your disk) | [forevermastery.com](https://forevermastery.com/en/guides/install-addons) |

**If you actually play Classic Era or Anniversary instead:**
- Classic Era is 1.15.9 with Interface 11509.
- Anniversary is currently the TBC 2.5.6 client with Interface 20506 ([TOC format](https://warcraft.wiki.gg/wiki/TOC_format), [wow-ui-source classic_era version](https://raw.githubusercontent.com/Gethe/wow-ui-source/classic_era/version.txt)).
- Those clients have **no secret values**: the `classic_era` UnitDocumentation has no `SecretReturns` annotations ([classic_era UnitDocumentation](https://github.com/Gethe/wow-ui-source/blob/classic_era/Interface/AddOns/Blizzard_APIDocumentationGenerated/UnitDocumentation.lua)).
- Their `C_CombatLog.GetCurrentEventInfo` is still in the public namespace ([classic_era CombatLogDocumentation](https://github.com/Gethe/wow-ui-source/blob/classic_era/Interface/AddOns/Blizzard_APIDocumentationGenerated/CombatLogDocumentation.lua)).
- So the target and combat fields would be freely readable there. Everything else in this document applies to both, except the UiMapIDs and the restrictions sections.

---

## 2. Addon basics for this client

### Folder layout

```
World of Warcraft\_forever_\Interface\AddOns\WowTracker\
    WowTracker.toc          (or WowTracker_Forever.toc / WowTracker_Camelot.toc for a Forever-only file)
    WowTracker.lua
```

Saved data lands in these files ([wiki](https://warcraft.wiki.gg/wiki/Saved_variables)):
- `WTF\Account\<ACCOUNT>\SavedVariables\WowTracker.lua` (account-wide).
- `WTF\Account\<ACCOUNT>\<Realm>\<Character>\SavedVariables\WowTracker.lua` (per character).

### TOC rules (per [TOC format](https://warcraft.wiki.gg/wiki/TOC_format))

- **Interface numbers.** `## Interface:` accepts a comma list, e.g. `## Interface: 16001, 11509, 20506`.
- **Per-client TOC files.** Suffixes pick a TOC per client: `_Forever` or `_Camelot` for Forever, `_Vanilla` for Classic Era, `_TBC`, `_Mists`, `_Standard` for retail. Family suffixes are `_Mainline`, which **includes Forever**, and `_Classic`, which covers vanilla through mists. A game-type suffix beats a family suffix. **Pitfall:** a `_Mainline.toc` will also be picked up by Forever.
- **Load conditions.** `## AllowLoadGameType: forever` restricts loading. The wiki warns that unrecognised values are treated as satisfied ([TOC format](https://warcraft.wiki.gg/wiki/TOC_format), [Patch 1.60.1/API changes](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes)).
- **Saved data.** `## SavedVariables:` is account-wide and `## SavedVariablesPerCharacter:` is per character.
- **Metadata.** `## Title`, `## Notes`, `## Category` and `## IconTexture` are optional metadata.
- **Language and structure.** Lua 5.1 plus optional XML; files are listed in load order in the TOC.
  - Forever renamed `require` to `import`. That is a Blizzard-internal module system, not Lua `require` ([Patch 1.60.1/API changes](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes)).
  - An addon does not need XML. `CreateFrame` from Lua is enough.

### Events, OnUpdate and SavedVariables lifecycle

- **SavedVariables.** They are loaded after the addon's code runs, and then `ADDON_LOADED(addonName)` fires. They are written **only on logout, disconnect, quit or `/reload`**. `PLAYER_LOGOUT` is the last chance to modify them. Only strings, numbers, booleans and tables persist ([wiki](https://warcraft.wiki.gg/wiki/Saved_variables)).
- **Events.** Register with `frame:RegisterEvent("...")` and handle them in `OnEvent`. For polling, use `frame:SetScript("OnUpdate", fn(self, elapsed))` and throttle it yourself, for example to 1 Hz.
- **Restriction state.** Forever has `ADDON_RESTRICTION_STATE_CHANGED` and `C_RestrictedActions.IsAddOnRestrictionActive(Enum.AddOnRestrictionType.Combat|Encounter|ChallengeMode|PvPMatch|Map|Chat)`. Use them to know when values will be secret ([RestrictedActionsConstantsDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/RestrictedActionsConstantsDocumentation.lua), [RestrictedActionsDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/RestrictedActionsDocumentation.lua)).

### Debugging

- **Built-in slash commands.** `/reload`, `/dump <expr>`, `/etrace` (event trace), `/fstack`, `/tinspect` and `/api` all exist in the Forever UI code ([SlashCommands.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_ChatFrameBase/Shared/SlashCommands.lua), [SlashCommandsOverrides.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_ChatFrameBase/Mainline/SlashCommandsOverrides.lua)).
- **Lua error display.** BugSack's TOC already lists 16001: `## Interface: 11508, 11509, 16001, ...` ([BugSack.toc](https://github.com/funkydude/BugSack/blob/master/BugSack.toc)). It needs BugGrabber; I did not check BugGrabber's TOC. Alternatively, turn on `/console scriptErrors 1`.
- **Secret values.** Test with `issecretvalue(v)` before doing arithmetic or comparisons. Doing either on a secret raises an immediate Lua error ([Secret Values](https://warcraft.wiki.gg/wiki/Secret_Values)).

### Getting Blizzard's UI source for this branch

- **Online.** https://github.com/Gethe/wow-ui-source/tree/forever. The README branch list includes `forever`, `classic_era`, `classic_anniversary`, `classic` and `live`. API signatures are in `Interface/AddOns/Blizzard_APIDocumentationGenerated/*.lua`.
- **Locally.** Launch the client with `-console`. At the login or character screen, open the console and run `ExportInterfaceFiles code`. Output goes to `BlizzardInterfaceCode\` ([wiki](https://warcraft.wiki.gg/wiki/Viewing_Blizzard%27s_interface_code)).

### Minimal addon (gathers every requested field; untested in-game)

`WowTracker.toc`
```
## Interface: 16001
## Title: WoW Tracker
## Notes: Collects position / XP / combat state for wowtracker.osrs.dk
## Version: 0.1.0
## SavedVariables: WowTrackerDB
WowTracker.lua
```

`WowTracker.lua`
```lua
local ADDON = ...
local S = {}                                   -- current snapshot
local f = CreateFrame("Frame")

-- Midnight/Forever: secret values cannot be compared, used in arithmetic or saved meaningfully.
local function plain(v)
  if issecretvalue and issecretvalue(v) then return nil end
  return v
end

local function updatePosition()
  local mapID = C_Map.GetBestMapForUnit("player")          -- nil in some places
  S.mapID, S.x, S.y, S.contID, S.wx, S.wy = mapID, nil, nil, nil, nil, nil
  if mapID then
    local pos = C_Map.GetPlayerMapPosition(mapID, "player") -- nil inside instances/BGs/arenas
    if pos then
      S.x, S.y = pos:GetXY()                                -- 0..1 within the uiMap
      local contID, world = C_Map.GetWorldPosFromMapPos(mapID, pos)
      if world then S.contID = contID; S.wx, S.wy = world:GetXY() end
    end
  end
  S.zone, S.subzone = GetRealZoneText(), GetSubZoneText()
end

local function updateXP()
  S.level  = UnitLevel("player")
  S.xp, S.xpMax = UnitXP("player"), UnitXPMax("player")
  S.rested = GetXPExhaustion() or 0                         -- nil when no rested XP
end

local function updateTarget()
  if not UnitExists("target") then S.target = nil; return end
  local hp, hpMax = plain(UnitHealth("target")), plain(UnitHealthMax("target"))
  S.target = {
    name  = plain(UnitName("target")),                      -- secret for NPCs in combat (see section 3)
    level = plain(UnitLevel("target")),                     -- -1 = skull/boss
    class = plain(UnitClassification("target")),            -- "normal","elite","rare","rareelite","worldboss","trivial","minus"
    ctype = plain(UnitCreatureType("target")),
    hpPct = (hp and hpMax and hpMax > 0) and math.floor(hp / hpMax * 100 + 0.5) or nil,
  }
end

local function updateCombat() S.inCombat = UnitAffectingCombat("player") and true or false end

f:SetScript("OnEvent", function(self, event, arg1)
  if event == "ADDON_LOADED" and arg1 == ADDON then
    WowTrackerDB = WowTrackerDB or {}
  elseif event == "PLAYER_ENTERING_WORLD" then
    updatePosition(); updateXP(); updateTarget(); updateCombat()
  elseif event == "ZONE_CHANGED_NEW_AREA" or event == "ZONE_CHANGED" or event == "ZONE_CHANGED_INDOORS" then
    updatePosition()
  elseif event == "PLAYER_XP_UPDATE" or event == "PLAYER_LEVEL_UP" or event == "UPDATE_EXHAUSTION" then
    updateXP()
  elseif event == "PLAYER_REGEN_DISABLED" or event == "PLAYER_REGEN_ENABLED" then
    updateCombat(); updateTarget()
  elseif event == "PLAYER_TARGET_CHANGED" or (event == "UNIT_HEALTH" and arg1 == "target") then
    updateTarget()
  elseif event == "PLAYER_LOGOUT" then
    S.savedAt = time()
    WowTrackerDB.last = S                                    -- only reaches disk now (or on /reload)
  end
end)

for _, e in ipairs({ "ADDON_LOADED","PLAYER_ENTERING_WORLD","ZONE_CHANGED_NEW_AREA","ZONE_CHANGED",
  "ZONE_CHANGED_INDOORS","PLAYER_XP_UPDATE","PLAYER_LEVEL_UP","UPDATE_EXHAUSTION",
  "PLAYER_REGEN_DISABLED","PLAYER_REGEN_ENABLED","PLAYER_TARGET_CHANGED","UNIT_HEALTH","PLAYER_LOGOUT" }) do
  f:RegisterEvent(e)
end

-- Position has no "moved" event: poll at 1 Hz.
local acc = 0
f:SetScript("OnUpdate", function(self, elapsed)
  acc = acc + elapsed
  if acc < 1 then return end
  acc = 0
  updatePosition()
  -- TODO: hand S to the export route (pixel strip, see section 4.4)
end)
```

Notes on the example:
- **Verified against Forever's API docs.** Every function and event name used here was checked; see section 3.
- **`UNIT_HEALTH` registration.** It could be narrowed with `f:RegisterUnitEvent("UNIT_HEALTH","target")`.
- **NOT verified in a running client:**
  - Whether `issecretvalue` exists as a global in Forever. It is documented for Midnight ([Secret Values](https://warcraft.wiki.gg/wiki/Secret_Values)).
  - The exact secrecy of target fields out of combat.
  - Whether storing a secret into SavedVariables errors or silently drops. The wiki is silent on this; the example avoids it via `plain()`.

---

## 3. API reference (Forever 1.60.1)

"Doc" means the function or event is present in the `forever` branch's `Blizzard_APIDocumentationGenerated`. Secret flags were read from the same files.

| API | Returns | In Forever? | Secret / restriction notes | Source |
|---|---|---|---|---|
| `C_Map.GetBestMapForUnit(unit)` | uiMapID or nil | **Y** (doc) | "Only works for the player and party members." No secret-return flag | [MapDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/MapDocumentation.lua) |
| `C_Map.GetPlayerMapPosition(uiMapID, unit)` | Vector2D (0–1) or nil | **Y** (doc + wiki lists forever 1.60.1) | **nil in instances, battlegrounds and arenas** (since 7.1.0). Player and party only | [MapDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/MapDocumentation.lua), [wiki](https://warcraft.wiki.gg/wiki/API_C_Map.GetPlayerMapPosition) |
| `C_Map.GetMapInfo(uiMapID)` | UiMapDetails {mapID, name, mapType, parentMapID, …} | **Y** (doc) | none | [MapDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/MapDocumentation.lua) |
| `C_Map.GetWorldPosFromMapPos(uiMapID, vec2)` | continentID (InstanceID), Vector2D world yards | **Y** (doc + wiki) | none | [wiki](https://warcraft.wiki.gg/wiki/API_C_Map.GetWorldPosFromMapPos) |
| `C_Map.GetMapPosFromWorldPos`, `C_Map.GetMapWorldSize` | inverse transform; map size in yards | **Y** (doc) | none | [MapDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/MapDocumentation.lua) |
| `UnitPosition(unit)` | posA, posB, 0, instanceID (world yards) | **Y** (doc + wiki) | nil in instances. Player, party and raid only. The wiki says the 1st return is labelled "Y" (axis order confusing; verify) | [UnitDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/UnitDocumentation.lua), [wiki](https://warcraft.wiki.gg/wiki/API_UnitPosition) |
| `GetZoneText()`, `GetSubZoneText()`, `GetRealZoneText()`, `GetMinimapZoneText()` | localized strings | **Y** (doc) | none | [ZoneScriptDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/ZoneScriptDocumentation.lua) |
| `UnitLevel(unit)` | number (-1 = ??) | **Y** | no secret flag | [UnitDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/UnitDocumentation.lua) |
| `UnitXP("player")`, `UnitXPMax("player")` | numbers | **Y** | no secret flag | same |
| `GetXPExhaustion()` | rested XP or nil | **Y** | none | [PlayerScriptDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/PlayerScriptDocumentation.lua) |
| `IsResting()`, `GetRestState()` | bool; state | **Y** | none | same |
| `UnitAffectingCombat(unit)` | bool | **Y** | no secret flag | [UnitDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/UnitDocumentation.lua) |
| `UnitExists(unit)` | bool | **Y** | none | same |
| `UnitName("target")` | name, realm | **Y** | `SecretWhenUnitNameIdentityRestricted`: secret when the unit "isn't player-controlled or in the party/raid" (except enemy players in PvP). The wiki says secret for non-player units **in combat**. **Exact out-of-combat behaviour for NPCs: UNVERIFIED** | [UnitDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/UnitDocumentation.lua), [SecretPredicatesDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/SecretPredicatesDocumentation.lua), [Secret Values](https://warcraft.wiki.gg/wiki/Secret_Values) |
| `UnitHealth("target")` | number | **Y** | `SecretReturns = true`. Secret when combat restrictions are active ("Combat API functions may now return secret values … for example UnitHealth") | [UnitDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/UnitDocumentation.lua), [Secret Values](https://warcraft.wiki.gg/wiki/Secret_Values) |
| `UnitHealthMax("target")` | number | **Y** | `SecretWhenUnitHealthMaxRestricted`: "when the unit isn't player-controlled" | same, [SecretPredicates](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/SecretPredicatesDocumentation.lua) |
| `UnitHealthPercent(unit[, usePredicted, curve])` | float or curve result | **Y** (new API) | `SecretReturns`, `SecretWhenCurveSecret`. Display-only in combat | [UnitDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/UnitDocumentation.lua) |
| `UnitClassification("target")` | "normal", "elite", "rare", "rareelite", "worldboss", "trivial", "minus" | **Y** | no secret flag | same |
| `UnitCreatureType("target")` | localized string | **Y** | `SecretWhenUnitIdentityRestricted` (NPCs) | same |
| `UnitGUID("target")` | GUID | **Y** | `SecretWhenUnitIdentityRestricted`. Creature GUID server/spawn parts are obfuscated in Forever beta 5 | same, [Patch 1.60.1/API changes](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes) |
| `CombatLogGetCurrentEventInfo()` | — | **N (effectively)** | Only a deprecation shim mapping to `C_CombatLog.GetCurrentEventInfo`, which is **not** in the public `C_CombatLog` docs. `GetCurrentEventInfo` exists only in `C_CombatLogSecure` (`HasRestrictions`) and `C_CombatLogInternal`. In Midnight, `CombatLogGetCurrentEventInfo` was removed and CLEU errors on registration | [Deprecated_CombatLog.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_DeprecatedCombatLog/Deprecated_CombatLog.lua), [CombatLogDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/CombatLogDocumentation.lua), [Patch 12.0.0 API changes](https://warcraft.wiki.gg/wiki/Patch_12.0.0/API_changes) |
| `LoggingCombat([bool])` | isLogging | **Y** (wiki lists forever 1.60.1. Used by `/combatlog`) | Rate limit of 5 calls per 10 s, shared across addons. `C_ChatInfo.IsLoggingCombat()` returns (enabled, advanced) | [wiki](https://warcraft.wiki.gg/wiki/API_LoggingCombat), [SlashCommands.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_ChatFrameBase/Shared/SlashCommands.lua), [ChatInfoDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/ChatInfoDocumentation.lua) |
| `CopyToClipboard` | — | Y but **protected** ("can only be called from secure code") | not usable by addons | [wiki](https://warcraft.wiki.gg/wiki/API_CopyToClipboard) |

**Events.** All of these are present in the Forever docs: `PLAYER_ENTERING_WORLD`, `PLAYER_LOGIN`, `PLAYER_LOGOUT`, `ADDON_LOADED`, `ZONE_CHANGED_NEW_AREA`, `ZONE_CHANGED`, `ZONE_CHANGED_INDOORS`, `PLAYER_XP_UPDATE`, `PLAYER_LEVEL_UP`, `UPDATE_EXHAUSTION`, `PLAYER_UPDATE_RESTING`, `PLAYER_REGEN_DISABLED`, `PLAYER_REGEN_ENABLED`, `PLAYER_TARGET_CHANGED`, `UNIT_HEALTH`, `UNIT_MAXHEALTH`, `UNIT_LEVEL`, `UNIT_CLASSIFICATION_CHANGED`, `UNIT_FLAGS`, `ADDON_RESTRICTION_STATE_CHANGED` ([SystemDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/SystemDocumentation.lua), [MapDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/MapDocumentation.lua), [UnitDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/UnitDocumentation.lua)).
- `COMBAT_LOG_EVENT_UNFILTERED` is still declared, with `HasRestrictions = true`. That flag also appears in classic_era, so it is not proof on its own. However, Blizzard's statement that the Midnight changes apply, plus the Midnight note that registering CLEU errors, mean you should **assume CLEU is unavailable to addons in Forever** (**verify in-game**).
- There is no "player moved" event; poll position in `OnUpdate`.

**Secret value rules that matter here** ([Secret Values](https://warcraft.wiki.gg/wiki/Secret_Values), [SecretPredicatesDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/SecretPredicatesDocumentation.lua)):
- **Not allowed on secrets (tainted code):** arithmetic, comparisons, `#`, using them as table keys. Any of these is an **immediate Lua error**.
- **Allowed on secrets:** storing them, passing them to `FontString:SetText` / `StatusBar:SetValue`, `string.format` / `concat`.
- **When restrictions turn on:**
  - In combat (`Combat`).
  - During encounters, in challenge mode and in PvP matches.
  - On restricted maps (dungeons and raids, `SecretOnRestrictedMaps`, 12.0.5).
- **How restrictions are reported:** `C_RestrictedActions.GetAddOnRestrictionState` / `ADDON_RESTRICTION_STATE_CHANGED`.
- **Practical consequence:** the addon can reliably export position (open world), level, XP, rested and in-combat state. For the target, it can reliably export level and classification. Name, creature type and health% are best-effort, captured out of combat when not secret.

---

## 4. Getting data out of the game

**Baseline facts.**
- The WoW Lua environment has no OS or file I/O libraries: "Notably, operating system and file I/O libraries are not present" ([Lua functions](https://warcraft.wiki.gg/wiki/Lua_functions)). There is no socket, HTTP or `io`.
- The only built-in disk output an addon controls is SavedVariables ([wiki](https://warcraft.wiki.gg/wiki/Saved_variables)).
- The clipboard is protected ([wiki](https://warcraft.wiki.gg/wiki/API_CopyToClipboard)).
- Since build 70291, trial accounts cannot send addon messages ([Patch 1.60.1/API changes](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes)).

### 4.1 SavedVariables + file watcher

- **How it works.** The addon fills `WowTrackerDB`. The client serialises it to `WTF\Account\<ACCOUNT>\SavedVariables\WowTracker.lua` **only on logout, disconnect, quit or `/reload`** ([wiki](https://warcraft.wiki.gg/wiki/Saved_variables)). A companion app watches the file, parses the Lua table (it is plain Lua table syntax; use any Lua-table parser), and POSTs it.
- **Latency.** Minutes to hours. It is live only if you `/reload` regularly, and `/reload` is a loading screen.
- **Reliability.** High. A crash or Alt-F4 may lose the last write; that is my inference, based on the write only happening at logout/reload.
- **Effort.** Low.
- **ToS risk.** Very low. This is how mainstream companion apps work: for example the TSM desktop app, which manages TSM addon files and data in SavedVariables and AppHelper ([TSM support](https://support.tradeskillmaster.com/addon/how-do-i-fix-an-error-about-appdatalua-being-empty)).
- **Track record.** A developer who tried SavedVariables for real-time export reported it was "too slow" ([forum](https://us.forums.blizzard.com/en/blizzard/t/wow-wow-combat-log-delay-%E2%80%93-seeking-api-or-workaround/55869)).
- **Use it for:** a "last seen" record and session summaries.

### 4.2 Combat log file (`/combatlog`, `LoggingCombat(true)`)

- **What it is.** `LoggingCombat(true)` or `/combatlog` writes combat log text to `Logs\WoWCombatLog*.txt`. Addons may call it, subject to the 5-per-10 s limit ([wiki](https://warcraft.wiki.gg/wiki/API_LoggingCombat)). The `advancedCombatLogging` CVar is still exposed in Forever's settings ([Network.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_SettingsDefinitions_Shared/Network.lua)).
- **Advanced fields** (17 parameters after the prefix) ([COMBAT_LOG_EVENT](https://warcraft.wiki.gg/wiki/COMBAT_LOG_EVENT)):
  - `infoGUID, ownerGUID, currentHP, maxHP, attackPower, spellPower, armor, absorb, powerType, currentPower, maxPower, powerCost, positionX, positionY, uiMapID, facing, level`. Here `level` is NPC level, or item level for players.
  - They appear on damage, heal and cast events such as `SPELL_DAMAGE`, `SPELL_CAST_SUCCESS` and `SWING_DAMAGE`. On `SWING_DAMAGE_LANDED` (advanced-only) the info unit is the **destination**, which gives the target's HP and level.
  - The log also writes `ZONE_CHANGE` (instanceID, zoneName, difficulty) and `MAP_CHANGE` (uiMapID, name, x0, x1, y0, y1) lines.
- **What it covers.** Exactly the "what am I fighting" data Forever's addon API hides: names, HP and level. Position and uiMapID come along on the player's own actions.
- **What it does not cover.** No events means no data, so walking or flying around produces nothing. There are no XP lines. Coverage is limited to events within logging range.
- **Still allowed in the Midnight model.** The secret-values changes did not touch the combat log file. Desktop analysers still read it ([WowCoach](https://wowcoach.gg/blog/how-to-enable-combat-logging-wow)), and a Forever-specific log analyser exists ([forever.wowcoach.gg](https://forever.wowcoach.gg/)).
- **Real-time-ness.** The client buffers writes. One developer reports delays of "several minutes" in some cases ([forum, 2025](https://us.forums.blizzard.com/en/blizzard/t/wow-wow-combat-log-delay-%E2%80%93-seeking-api-or-workaround/55869)).
  - The Warcraft Logs uploader's "live logging" reportedly reads the file as it grows but uploads after combat ends (via search summary of [archon.gg uploader docs](https://www.archon.gg/wow/articles/help/uploader); page blocked to my fetch).
  - Old docs claim the file is not updated until logout ([wowprogramming](https://wowprogramming.com/docs/api/LoggingCombat.html)), which modern live logging contradicts.
  - **Measure the flush interval in Forever yourself.**
- **Can an addon write custom lines?** I found no API that writes arbitrary text to the combat log, and none in Forever's `C_CombatLog` docs ([CombatLogDocumentation](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/CombatLogDocumentation.lua)). Treat the answer as **no**.
- **Effort.** Medium: a tail and parse loop. **ToS risk:** very low. This is the standard Warcraft Logs and WowCoach pattern.
- **Open questions for Forever:**
  - Whether the log uses obfuscated creature GUIDs like the API does. **UNVERIFIED.**
  - The path, likely `_forever_\Logs\`. **UNVERIFIED.**

### 4.3 Chat log (`/chatlog`, `WoWChatLog.txt`)

- **Write timing.** `LoggingChat(true)` writes "text received via the chat system (but not necessarily all text displayed in chat windows)". Old documentation says the file is only written at logout ([wowprogramming LoggingChat](https://wowprogramming.com/docs/api/LoggingChat.html)), and users report an empty file until logoff ([gist](https://gist.github.com/RagedUnicorn/818d01320af349b244feeaef5167f148)).
- **Getting addon data into it.** `DEFAULT_CHAT_FRAME:AddMessage` is local display, not "received via the chat system", so it is presumably not logged (**UNVERIFIED**). Sending real chat messages is a poor fit:
  - It is visible or spammy.
  - SAY, YELL, CHANNEL and (since 12.1.7 / Forever beta 4) EMOTE need a hardware event outdoors ([SendChatMessage](https://warcraft.wiki.gg/wiki/API_SendChatMessage), [Patch 1.60.1/API changes](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes)).
  - Forever adds chat lockdown states (`Chat` restriction type).
- **Verdict.** Slow, buffered and awkward. **Not recommended.**

### 4.4 Screen / pixel encoding

- **How it works.**
  - The addon draws a strip of solid-colour squares, each carrying 3 bytes in RGB: a header (magic plus length plus checksum), then the payload. Pack it compactly, e.g. mapID u16, x and y as u16 fixed-point, level u8, xp u32, xpMax u32, rested u32, flags, target level and classification.
  - A companion app grabs only that rectangle of the WoW window, using Windows Graphics Capture or DXGI Desktop Duplication, a few times per second. It validates the checksum and forwards the result.
- **Existing projects.**
  - [efskap/WoWCerealize](https://github.com/efskap/WoWCerealize): Classic. It sends packets (packet number, checksum, UTF-8) through a pixel strip. The author explicitly suggests "showing a character's position on a world map on a second screen" and RGB peripherals, and argues it is one-way, with no injection or memory reads.
  - [alex-berliner/LibSerpix](https://github.com/alex-berliner/LibSerpix) + [libserpix_rs](https://rust-digger.code-maven.com/crates/libserpix_rs): 1-px columns, an anchor colour, a header with size and checksum, and a CBOR payload. It defaults to 512 pixels (about 1.5 KB per frame by my arithmetic), "Realtime data transmission".
  - StatCast (addon plus Python, drives LIFX lights; per search results, repo not inspected).
- **Latency.** About one frame plus capture interval, so **sub-second**. **Effort:** medium-high. The hard parts:
  - UI scale and pixel snapping (use `PixelUtil` / an `UIParent` scale of 1 / larger blocks).
  - Colour management: HDR, gamma, night-light filters.
  - Windowed or borderless mode for capture.
  - The strip must stay on screen and unobstructed.
- **What it can carry.** Only the non-secret values. Secret values cannot even be processed: arithmetic or comparison on them is a Lua error. Getting secrets out would mean routing them through display primitives such as colour curves into pixels. That is deliberately defeating Blizzard's protection; **don't**.
- **ToS / Warden risk (honest assessment):**
  - The EULA prohibits "Use any unauthorized process or software that intercepts, collects, reads, or 'mines' information" and "any connection using third-party programs or tools not expressly authorized by Blizzard" ([EULA](https://www.blizzard.com/en-us/legal/fba4d00f-c7e4-4883-b8b9-1b4500a402ea/blizzard-end-user-license-agreement)). A screen-reading exporter arguably falls under "reads … information".
  - **Pixel-reading is the standard technique of rotation and grind bots.** Community discussion describes pixel bots as reading predefined screen pixels ([wowlazymacros](https://wowlazymacros.com/t/hekili-x-gse-kinda/57954)).
  - Blizzard's enforcement statements target **automation**, i.e. software that plays or presses keys ([2015 Bashiok summary](https://thelegalgeeks.com/2015/05/18/blizzard-bans-bots-100000-accounts-suspended-for-violating-terms-of-use-in-wow)).
  - I found **no Blizzard statement that specifically permits or bans one-way pixel export**.
  - Warden scans the client process. A separate app doing desktop capture, like OBS, is not visible as memory tampering. Detection would come from behaviour, i.e. automation.
  - My assessment is **low-to-moderate risk**, provided the companion never sends input to the game, never reads memory, and only encodes non-secret, non-combat-decision data. It is not zero. Blizzard can disable addon functionality at will ([UI Add-On Development Policy](https://us.forums.blizzard.com/en/wow/t/ui-add-on-development-policy/24534)), and the policy requires add-on code to be free and fully visible.

### 4.5 First-party options

- **Battle.net Web API.**
  - Classic Era uses `profile-classic1x-{region}`, and progression Classic uses `profile-classic-{region}` ([Blizzard forum](https://us.forums.blizzard.com/en/blizzard/t/wow-classic-era-realm-apis/16812), [403 fix thread](https://us.forums.blizzard.com/en/wow/t/wow-classic-era-api-character-api-returns-403-for-existing-characters-on-profile/1989372)).
  - The character profile summary includes `level`, `experience` and `last_login_timestamp`. One developer says that field is really the *logout* time, when data is committed ([forum 2020](https://us.forums.blizzard.com/en/blizzard/t/wow-last-login-timestamp-and-api-performance/9028)).
  - There is no location, no live state and no combat data. A replier in the 2025 thread confirms the web API offers no real-time data ([forum](https://us.forums.blizzard.com/en/blizzard/t/wow-wow-combat-log-delay-%E2%80%93-seeking-api-or-workaround/55869)).
- **Forever API status.** A community probe on 2026-10-26 got 403 for every guessed Forever namespace ([co-discord-bot #48](https://github.com/plohkoon/co-discord-bot/issues/48)). The Blizzard "When will we gain API access to Forever APIs?" thread has no staff answer ([forum](https://us.forums.blizzard.com/en/blizzard/t/when-will-we-gain-api-access-to-forever-apis/59595)). The official docs portal is JS-rendered and I could not read it ([develop.battle.net](https://develop.battle.net/documentation/world-of-warcraft-classic/profile-apis)). **Status after launch: UNVERIFIED.**
- **How to use it.** At best, show "level / last logout" when the live feed is off.
- **Official telemetry or streaming hooks.** None found. Forever's UI has a built-in damage meter, but that is in-game UI, not an export.

### 4.6 Other things people use

- **Overwolf GEP.** Overwolf's Game Events Provider for WoW advertises a `location` event (x, y, map, fired "when the player moves") and `player_info` (level, exp, exp_max) ([Overwolf docs](https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/world-of-warcraft)).
  - It does not say how it obtains this data.
  - Forever support is **UNVERIFIED**.
  - It ties you to the Overwolf platform. Worth a look as an "officially tolerated" alternative to DIY pixel capture, but verify Forever support first.
- **WeakAuras / WeakAuras Companion.** Not an option. WeakAuras ended development before Midnight and "won't return" ([Warperia](https://warperia.com/wow-forever-inherits-midnights-addon-changes/)). The Companion only synced auras through SavedVariables anyway.
- **CurseForge, WowUp, TSM and Raider.IO clients.** These are addon managers or SavedVariables-based companions (see 4.1). None provides a live export channel.
- **TomTom, Questie, RestedXP.** In-game only. Their data reaches disk only via SavedVariables. Questie ships a `Questie_Camelot.toc` (per [search summary](https://wowforeveraddons.com), not inspected).
- **Clipboard.** Protected ([wiki](https://warcraft.wiki.gg/wiki/API_CopyToClipboard)).
- **OBS.** OBS can capture the game, but any "WoW overlay" plugin still needs one of the routes above for data. I found none with its own channel.
- **Memory reading or packet sniffing.** This clearly violates the EULA ("intercepts … reads, or 'mines' information", "hacks") and is what Warden exists to detect. **Ruled out.**

### 4.7 Comparison

| Route | Latency | Covers travel / position | Covers XP / level | Covers "what am I fighting" in combat | Effort | ToS / ban risk |
|---|---|---|---|---|---|---|
| SavedVariables + watcher | /reload or logout only | yes (snapshot) | yes | out-of-combat snapshot only | low | very low |
| Combat log tail (ACL on) | buffered, seconds to minutes (measure) | only during combat | no (level only) | **yes** (names, HP, level) | medium | very low |
| Chat log | logout / buffered | via spammy messages only | same | no | medium | low, but impractical |
| Pixel strip + capture app | **< 1 s** | **yes** (open world; nil in instances) | **yes** | only non-secret fields | medium-high | **low-moderate (EULA grey area)** |
| Battle.net API | logout-ish | no | level / XP | no | low | none (Forever namespace unconfirmed) |
| Overwolf GEP | real-time (claimed) | yes (claimed) | yes (claimed) | no | low-medium | unknown mechanism; Forever unverified |
| Memory reading | — | — | — | — | — | **ban-worthy; excluded** |

### 4.8 Recommendation

1. **Ship the zero-risk path first.** Use the addon (section 2) plus SavedVariables and a watcher, so the site shows "last seen at <zone> (x, y), level N, XP a/b" after every `/reload` or logout.
2. **Add live mode as an opt-in.**
   - Add a pixel strip to the same addon, carrying only the fields in section 3 that are not secret.
   - Add a small Windows companion app that captures that strip at 2–5 Hz and POSTs deltas over HTTPS.
   - Document clearly that it is one-way, does no input and reads no memory.
3. **Optional: combat enrichment.** Tail `WoWCombatLog*.txt` with Advanced Combat Logging on to get target names, HP, level and combat position. Accept that it lags by the client's flush interval.
4. **Watch for later options.** Re-check Overwolf GEP and the Battle.net API for Forever support. If either appears, it can replace or augment the pixel channel.

---

## 5. Map rendering on the website

**Coordinates.**
- `C_Map.GetPlayerMapPosition` gives normalised (0–1) coordinates per uiMapID.
- `C_Map.GetWorldPosFromMapPos(uiMapID, pos)` converts them to world yards plus continent instanceID ([wiki](https://warcraft.wiki.gg/wiki/API_C_Map.GetWorldPosFromMapPos)).
- The same transform can be done server-side from the **UiMapAssignment** DB2. Columns `Region_0..2` are the world min (x, y, z), `Region_3..5` the world max, and `UiMin`/`UiMax` the normalised sub-rectangle.

**Forever-specific UiMapIDs.** I pulled the tables for **build 1.60.1.70790** ([UiMap csv](https://wago.tools/db2/UiMap/csv?build=1.60.1.70790), [UiMapAssignment csv](https://wago.tools/db2/UiMapAssignment/csv?build=1.60.1.70790)). Forever uses the **Classic-era IDs**, not retail's. The table has only 60 UiMap rows.

| UiMap | ID | Parent | World bounds (min x,y → max x,y) |
|---|---|---|---|
| Azeroth | 947 | 0 | composite |
| Kalimdor | 1414 | 947 | (-11733.3, -19733.2) → (12799.9, 17066.6), MapID 1 |
| Eastern Kingdoms | 1415 | 947 | (-16000, -19199.9) → (7466.6, 16000), MapID 0 |
| Elwynn Forest | 1429 | 1415 | (-10254.17, -1935.42) → (-7939.58, 1535.42) |
| Durotar | 1411 | 1414 | (-1716.67, -7250.0) → (1808.33, -1962.5) |
| New Forever zones | Mount Hyjal 2482, Zephras Isle 2521/2665 (own MapID 2991), Riverglades 2548, Shen'dralas 2652 | | from same csv |

**Axis convention** (checked against the wiki's Elwynn example, where map (0,0) gives world (-7939.58, 1535.42) and (1,1) gives (-10254.2, -1935.42) ([wiki](https://warcraft.wiki.gg/wiki/API_C_Map.GetWorldPosFromMapPos))):

```
worldX = maxX - v * (maxX - minX)   // v = map vertical (down)
worldY = maxY - u * (maxY - minY)   // u = map horizontal (right)
```

To place a zone point on a **continent** image, convert zone (u, v) to world (x, y), then invert with the continent's bounds: `u_c = (maxY_c - worldY)/(maxY_c - minY_c)`, `v_c = (maxX_c - worldX)/(maxX_c - minX_c)`. Doing this server-side from the CSVs means the addon only needs to send `(uiMapID, u, v)`.

**Map images.**
- **[wow.export](https://github.com/Kruithne/wow.export)** (MIT) exports zone maps, textures and DB2s from a local install or the CDN.
- **[wago.tools](https://wago.tools/maps/worldmap/1429)** renders world maps by UiMapID ([UiMapID wiki](https://warcraft.wiki.gg/wiki/UiMapID)).
- Whether either already handles the `wowf` product is **UNVERIFIED**; wago.tools does index `wowf` builds.
- Wowhead's map images are their hosted copies. Don't hotlink or scrape them. Wowhead's terms were not checked.

**Copyright.**
- The map art is Blizzard's. Blizzard's Legal FAQ allows fan-site use "within certain limits", "for home, noncommercial and personal use only". It requires keeping copyright notices, says the license can be revoked "at any time, for any reason", and forbids altering or reverse-engineering downloaded content ([Legal FAQ](https://www.blizzard.com/en-us/legal/28d5ebbf-c245-4408-8ba9-043dd5f056bf/legal-faq)).
- Extracting textures from game files sits awkwardly with the "reverse engineer" clause, even though community tools do it routinely.
- **Practical advice:**
  - Keep the site non-commercial.
  - Credit "© Blizzard Entertainment".
  - Prefer low-resolution continent images.
  - Or draw your own stylised outline map from the UiMapAssignment rectangles.

**Web map library.** Use Leaflet with `L.CRS.Simple` and an `L.imageOverlay` sized to the image's pixel dimensions ([Leaflet CRS.Simple tutorial](https://leafletjs.com/examples/crs-simple/crs-simple.html)). In Leaflet, `[lat, lng]` = `[-v_c*H, u_c*W]`, or flip the image with bounds `[[0,0],[H,W]]` and use `[H - v_c*H, u_c*W]`. Keep one overlay per continent, switch when `contID` changes, and use a marker plus a short trail polyline.

---

## 6. Server / transport sketch

```
WoW (addon: pixel strip / SavedVariables / combat log)
   │  screen capture, file watch
   ▼
Companion app (Windows; e.g. .NET, Rust, Python)
   │  HTTPS POST /api/ingest   {charId, ts, mapID, u, v, level, xp, xpMax, rested, inCombat, target{...}}
   │  Authorization: Bearer <per-user device token>   (≤ 2–5 req/s, send deltas, heartbeat every 10 s)
   ▼
wowtracker.osrs.dk server
   ├─ validate + rate-limit, convert (mapID,u,v) → continent coords using the UiMapAssignment CSV
   ├─ keep last state in memory/Redis; mark "offline" after ~30 s without heartbeat
   └─ fan-out: Server-Sent Events  GET /api/stream/:charId   (or WebSocket)
   ▼
Browser (Leaflet CRS.Simple) — EventSource → move marker
```

- **SSE versus WebSocket.** SSE is enough because data flows one way. Use WebSocket only if you add browser-to-companion controls.
- **Privacy.** Live location is personal data. Make the stream page opt-in and private by default, and consider a delay option for streamers.

---

## 7. Open questions / verify in-game

1. `/dump select(4, GetBuildInfo())` should give **16001**. `/dump WOW_PROJECT_ID` should give 18.
2. The actual install path (`_forever_`?), plus the SavedVariables and `Logs\` locations.
3. Whether `issecretvalue` exists, and what `UnitName("target")`, `UnitHealth("target")` and `UnitCreatureType("target")` return for an NPC **out of combat** versus **in combat**. Check with `/dump issecretvalue(UnitName("target"))`.
4. Whether registering `COMBAT_LOG_EVENT_UNFILTERED` raises `ADDON_ACTION_FORBIDDEN` in Forever.
5. What happens when a secret value is stored in SavedVariables: error, dropped or saved?
6. The combat log flush latency in Forever with ACL on. Does `/combatlog` produce `WoWCombatLog-<date>.txt`? Are creature GUIDs obfuscated in the file?
7. Whether `C_Map.GetPlayerMapPosition` is ever secret or nil in the open world during combat. No secret flag is documented.
8. Whether a Forever Battle.net API namespace exists after launch, and its name.
9. Whether Overwolf GEP supports the `wowf` product.
10. Whether wow.export can open the `wowf` product, for map art extraction.
11. The BugGrabber TOC includes 16001. Only BugSack's was checked.

---

## Sources

**Blizzard / first-party**
- Blizzard EULA (rev. 2024-03-21): https://www.blizzard.com/en-us/legal/fba4d00f-c7e4-4883-b8b9-1b4500a402ea/blizzard-end-user-license-agreement
- Blizzard Legal FAQ: https://www.blizzard.com/en-us/legal/28d5ebbf-c245-4408-8ba9-043dd5f056bf/legal-faq
- UI Add-On Development Policy: https://us.forums.blizzard.com/en/wow/t/ui-add-on-development-policy/24534
- Forum, combat log delay: https://us.forums.blizzard.com/en/blizzard/t/wow-wow-combat-log-delay-%E2%80%93-seeking-api-or-workaround/55869
- Forum, Forever API access: https://us.forums.blizzard.com/en/blizzard/t/when-will-we-gain-api-access-to-forever-apis/59595
- Forum, Forever shares retail restrictions (player post): https://us.forums.blizzard.com/en/wow/t/wow-forever-will-share-the-addon-and-api-restrictions-of-retail/2350682
- Forum, Classic Era API namespaces: https://us.forums.blizzard.com/en/blizzard/t/wow-classic-era-realm-apis/16812
- Forum, profile-classic1x 403 fix: https://us.forums.blizzard.com/en/wow/t/wow-classic-era-api-character-api-returns-403-for-existing-characters-on-profile/1989372
- Forum, last_login_timestamp: https://us.forums.blizzard.com/en/blizzard/t/wow-last-login-timestamp-and-api-performance/9028
- develop.battle.net Classic profile docs (JS-rendered, not readable by fetch): https://develop.battle.net/documentation/world-of-warcraft-classic/profile-apis

**Blizzard UI source (Gethe/wow-ui-source, branch `forever`, 1.60.1.70790)**
- Branch / version: https://github.com/Gethe/wow-ui-source/tree/forever, https://raw.githubusercontent.com/Gethe/wow-ui-source/forever/version.txt
- MapDocumentation: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/MapDocumentation.lua
- UnitDocumentation: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/UnitDocumentation.lua
- PlayerScriptDocumentation: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/PlayerScriptDocumentation.lua
- ZoneScriptDocumentation: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/ZoneScriptDocumentation.lua
- SystemDocumentation: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/SystemDocumentation.lua
- CombatLogDocumentation: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/CombatLogDocumentation.lua
- SecretPredicatesDocumentation: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/SecretPredicatesDocumentation.lua
- RestrictedActions(Constants)Documentation: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/RestrictedActionsConstantsDocumentation.lua, https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/RestrictedActionsDocumentation.lua
- ChatInfoDocumentation: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/ChatInfoDocumentation.lua
- Deprecated_CombatLog.lua: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_DeprecatedCombatLog/Deprecated_CombatLog.lua
- Blizzard_DamageMeter.toc: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_DamageMeter/Blizzard_DamageMeter.toc
- SlashCommands.lua: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_ChatFrameBase/Shared/SlashCommands.lua
- SlashCommandsOverrides.lua: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_ChatFrameBase/Mainline/SlashCommandsOverrides.lua
- Settings Network.lua: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_SettingsDefinitions_Shared/Network.lua
- classic_era comparisons: https://github.com/Gethe/wow-ui-source/blob/classic_era/Interface/AddOns/Blizzard_APIDocumentationGenerated/UnitDocumentation.lua, https://github.com/Gethe/wow-ui-source/blob/classic_era/Interface/AddOns/Blizzard_APIDocumentationGenerated/CombatLogDocumentation.lua

**warcraft.wiki.gg**
- Patch 1.60 (Forever): https://warcraft.wiki.gg/wiki/Patch_1.60
- Patch 1.60.1/API changes: https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes
- Patch 12.0.0/API changes: https://warcraft.wiki.gg/wiki/Patch_12.0.0/API_changes
- Secret Values: https://warcraft.wiki.gg/wiki/Secret_Values
- TOC format: https://warcraft.wiki.gg/wiki/TOC_format
- Saved variables: https://warcraft.wiki.gg/wiki/Saved_variables
- Lua functions: https://warcraft.wiki.gg/wiki/Lua_functions
- Viewing Blizzard's interface code: https://warcraft.wiki.gg/wiki/Viewing_Blizzard%27s_interface_code
- C_Map.GetPlayerMapPosition: https://warcraft.wiki.gg/wiki/API_C_Map.GetPlayerMapPosition
- C_Map.GetWorldPosFromMapPos: https://warcraft.wiki.gg/wiki/API_C_Map.GetWorldPosFromMapPos
- UnitPosition: https://warcraft.wiki.gg/wiki/API_UnitPosition
- UiMapID: https://warcraft.wiki.gg/wiki/UiMapID
- COMBAT_LOG_EVENT: https://warcraft.wiki.gg/wiki/COMBAT_LOG_EVENT
- LoggingCombat: https://warcraft.wiki.gg/wiki/API_LoggingCombat
- SendChatMessage: https://warcraft.wiki.gg/wiki/API_SendChatMessage
- CopyToClipboard: https://warcraft.wiki.gg/wiki/API_CopyToClipboard

**Data / tools**
- wago.tools builds API: https://wago.tools/api/builds
- UiMap (Forever build): https://wago.tools/db2/UiMap/csv?build=1.60.1.70790
- UiMapAssignment (Forever build): https://wago.tools/db2/UiMapAssignment/csv?build=1.60.1.70790
- wago.tools world map viewer: https://wago.tools/maps/worldmap/1429
- wow.export: https://github.com/Kruithne/wow.export
- BugSack TOC: https://github.com/funkydude/BugSack/blob/master/BugSack.toc
- WoWCerealize: https://github.com/efskap/WoWCerealize
- LibSerpix: https://github.com/alex-berliner/LibSerpix, libserpix_rs: https://rust-digger.code-maven.com/crates/libserpix_rs
- Overwolf GEP WoW: https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/world-of-warcraft
- Leaflet CRS.Simple: https://leafletjs.com/examples/crs-simple/crs-simple.html
- wowprogramming LoggingCombat / LoggingChat (old): https://wowprogramming.com/docs/api/LoggingCombat.html, https://wowprogramming.com/docs/api/LoggingChat.html
- Archon/WCL uploader (blocked to fetch; cited via search summary): https://www.archon.gg/wow/articles/help/uploader
- TSM support (SavedVariables companion example): https://support.tradeskillmaster.com/addon/how-do-i-fix-an-error-about-appdatalua-being-empty

**Secondary / news (used for context only)**
- Windows Central: https://www.windowscentral.com/gaming/blizzard/world-of-warcraft-forever-is-blizzards-take-on-classic-revamping-vanilla-azeroth
- Warperia (relays the Wowhead report): https://warperia.com/wow-forever-inherits-midnights-addon-changes/
- Wowhead report: https://www.wowhead.com/news/wow-forever-will-have-addon-changes-from-midnight-382921
- forevermastery.com install guide: https://forevermastery.com/en/guides/install-addons
- WowCoach combat logging / Forever analyser: https://wowcoach.gg/blog/how-to-enable-combat-logging-wow, https://forever.wowcoach.gg/
- co-discord-bot Forever namespace probe: https://github.com/plohkoon/co-discord-bot/issues/48
- Pixel-bot community discussion: https://wowlazymacros.com/t/hekili-x-gse-kinda/57954
- 2015 ban wave / Bashiok summary: https://thelegalgeeks.com/2015/05/18/blizzard-bans-bots-100000-accounts-suspended-for-violating-terms-of-use-in-wow
- Chat log empty until logout (user report): https://gist.github.com/RagedUnicorn/818d01320af349b244feeaef5167f148
