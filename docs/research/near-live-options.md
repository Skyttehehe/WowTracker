# Near-live position for the Forever tracker: zero-risk options

Researched 2026-11-05, the day after World of Warcraft: Forever launched (client 1.60.1, Interface 16001). This file follows on from section 4 of [wow-classic-live-tracker.md](wow-classic-live-tracker.md) and does not repeat it.

Constraint for this file: **zero Terms-of-Service risk.** Pixel or screen reading, memory reading, packet sniffing and input automation are already ruled out.

How claims are labelled:
- Every claim has a URL.
- **UNVERIFIED** means no primary source was found, or the claim has not been tested on the live Forever client.
- **Third-party observation** means a developer wrote down what they saw in their own public repo. That is useful evidence, but it is not a Blizzard statement.
- **Via search summary** means the page refused automated fetches (archon.gg, warcraftlogs.com and icy-veins.com returned HTTP 403), so the wording comes from a search engine's summary of the page.

---

## TL;DR

- **No zero-risk route is known to give ~1 s latency.** Every file the client writes during play is buffered or only written at logout. Blizzard deliberately added the combat-log delay in 10.2 to stop real-time external use ([Kaivax, 2023-08-21](https://us.forums.blizzard.com/en/wow/t/nerfing-combat-logs-will-it-happen-should-it-happen/1654695/11)). Plan for **minutes, not seconds**, unless a measurement below proves otherwise.
- **Best zero-risk data source today: tail the combat log with Advanced Combat Logging on.**
  - It carries the player's world x/y, `uiMapID`, HP and power on every player cast (`SPELL_CAST_SUCCESS`), on heals and energizes the player receives, on swings, and on environmental damage. This also applies outside combat, as long as something is logged.
  - It writes `ZONE_CHANGE` and `MAP_CHANGE` lines, including in the open world. A real Forever trace shows `MAP_CHANGE,1429,"Elwynn Forest",...` ([trace](https://github.com/dbspringer/nameplatesct-reforged/blob/main/spec/traces/warlock-open-world.combatlog.txt)).
  - **Latency:**
    - A Forever beta tester saw the file stay empty through a fight and fill about 4 minutes after it ([DOINK notes](https://github.com/ryan-flan/DOINK/blob/main/CLAUDE.md)).
    - Archon itself says WoW's log is "delayed a few minutes after ending combat" (via search summary of the [Archon FAQ](https://www.archon.gg/eso/articles/help/archon-app-help-and-faq)).
    - Warcraft Recorder waits up to **10 minutes** for silence on Retail and Forever logs, against 2 minutes on Classic ([LogHandler.ts](https://github.com/aza547/wow-recorder/blob/main/src/parsing/LogHandler.ts), [ForeverLogHandler.ts](https://github.com/aza547/wow-recorder/blob/main/src/parsing/ForeverLogHandler.ts)).
  - **No CVar for the flush interval was found.**
  - It has no XP and no character level. The advanced `level` field is item level for players.
- **New official route worth testing: Battle.net Profile API, "protected character".**
  - It returns `position` (zone, map, x, y, z) for the logged-in user's own characters ([response type](https://github.com/Pewtro/blizzard-api/blob/main/packages/wow/src/account-profile/types.ts), [2020 example](https://us.forums.blizzard.com/en/blizzard/t/wow-profile-api-character-position-zone-coordinates/13506)).
  - A community probe found `profile-forever-{region}` live and returning `protected_character` links on 2026-11-02 ([forum](https://us.forums.blizzard.com/en/blizzard/t/world-of-warcraft-forever-api-namespace/59774)).
  - ToS risk is zero (it is Blizzard's own API with the user's OAuth consent).
  - How often the position refreshes while you are logged in is **UNVERIFIED**. It is probably at logout, possibly on periodic server saves. Measure it.
- **Possible zero-risk channel: `C_Log.LogMessage` to the "developer log".**
  - Forever ships an unrestricted `C_Log.LogMessage(message)` ([LogDocumentation.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/LogDocumentation.lua)).
  - The CVars `developerLog` (default 0) and `developerLogWriteToFile` (default 1) exist ([CVar list](https://warcraft.wiki.gg/wiki/Console_variables)).
  - Whether this writes a file under `Logs\`, and how fast, is **UNVERIFIED**. If it does, it is the only route found where an addon writes its own text to disk during play. Test it first.
- **Not useful for live data:**
  - **WoWChatLog.txt:** written at exit ([DOINK beta test](https://github.com/ryan-flan/DOINK/blob/main/CLAUDE.md), [wowprogramming](https://wowprogramming.com/docs/api/LoggingChat.html)).
  - **Config.wtf:** written on logout or reload ([wiki](https://warcraft.wiki.gg/wiki/API_C_CVar.SetCVar)).
  - **SavedVariables:** written on logout or reload, with no force-save API in the Forever API docs.
  - **Battle.net friends presence:** in-game API only, no export.
  - **Discord:** Blizzard's integration is guild-chat only.
  - **WoW Companion app:** retired in 2024.
- **Overwolf is not zero-risk.**
  - Its mechanism is proprietary and undisclosed. There is no public Blizzard–Overwolf agreement for WoW, and a Taiwan player reports a 2024 suspension that he attributes to Overwolf.
  - Its "WoW Forever" GEP page offers **zone + level/XP only, no x/y** ([Overwolf](https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/wow-forever)).
- **Blizzard has never published a list of what third-party tools may read.**
  - What exists: the EULA's broad bans, the 2025 statement that client-modifying software violates the ToS ([Kaivax](https://us.forums.blizzard.com/en/wow/t/prohibitions-on-third-party-software/2142972)), and the 2023 statement that it would change how the Advanced Combat Log is populated to stop overlays.
  - Warcraft Logs, Raider.IO and TSM desktop are tolerated in practice. No Blizzard text naming them as allowed was found.

---

## 1. Combat log file (`Logs\WoWCombatLog-*.txt`)

### 1.1 How quickly the client flushes it

| Evidence | What it says | Source |
|---|---|---|
| Blizzard intent (2023) | Kaivax (Blizzard): "We intend to make changes to how the Advanced Combat Log is populated". This came ahead of Dragonflight Season 3, citing EULA 1-C, against overlays. | [Blizzard forum post #11, 2023-08-21](https://us.forums.blizzard.com/en/wow/t/nerfing-combat-logs-will-it-happen-should-it-happen/1654695/11) |
| Patch 10.2 behaviour | The game keeps combat-log data in memory and writes it to the file only once combat ends. A crash or reload mid-pull loses the pull. On the PTR the file had events out of order. | Wowhead's 10.2 PTR article, [wowhead.com](https://www.wowhead.com/news/combat-log-changes-with-patch-10-2-measures-to-prevent-out-of-game-overlays-335813) (body via search summary); [mein-mmo](https://mein-mmo.de/en/wow-has-introduced-a-nasty-trick-against-many-addons-and-programs,1057395) |
| 2025 developer report | "delays of several minutes before writing to the log file". The reply: the only option is reading the text file, and workarounds would violate the ToS. | [Blizzard forum, 2025-08-28](https://us.forums.blizzard.com/en/blizzard/t/wow-wow-combat-log-delay-%E2%80%93-seeking-api-or-workaround/55869) |
| Forever beta measurement | File "stayed empty through a fight and filled ~4 minutes after it". The author's note says to retest at launch. **Third-party observation, beta build.** | [DOINK CLAUDE.md, "Beta facts", 2026-10-01](https://github.com/ryan-flan/DOINK/blob/main/CLAUDE.md) |
| Warcraft Recorder | "log flush behaviour varies between games". It uses a 10-minute no-data timeout for Retail and for Forever, against 2 minutes for Classic and Era. | [LogHandler.ts](https://github.com/aza547/wow-recorder/blob/main/src/parsing/LogHandler.ts), [RetailLogHandler.ts](https://github.com/aza547/wow-recorder/blob/main/src/parsing/RetailLogHandler.ts), [ForeverLogHandler.ts](https://github.com/aza547/wow-recorder/blob/main/src/parsing/ForeverLogHandler.ts) |

**CVars.** Nothing in the CVar list controls file flushing. The relevant ones ([wiki CVar list](https://warcraft.wiki.gg/wiki/Console_variables)):
- `advancedCombatLogging`: turns on the advanced fields.
- `combatLogUniqueFilename` (default 1): "Write combat log file with a timestamped name per client launch".
- `combatLogRetentionTime` (default 300): in-memory retention of entries for the UI. It is not a file setting. That is my reading of its description.

The Forever UI source references no other combat-log CVar ([forever tree](https://github.com/Gethe/wow-ui-source/tree/forever)). **Conclusion: no supported way to change the buffering was found.**

**Open question: flushes outside combat.** Nothing found says when the client flushes when there is *no* combat: on a timer, at a size threshold, or on zone change. It must be measured (section 8).

### 1.2 What the uploaders and live loggers do about the delay

- **Warcraft Logs / Archon app.** "Live logging" reads the file while the game writes it and uploads changes after combat ends (via search summary of the [Archon uploader help](https://www.archon.gg/classic-mop/articles/help/uploader)).
  - Archon's FAQ says a real-time option "does not work for WoW any longer as the combat log is delayed a few minutes after ending combat" (via search summary of the [Archon FAQ](https://www.archon.gg/eso/articles/help/archon-app-help-and-faq)).
  - Archon's "Auto Logging" uses an **in-game addon that turns combat logging on** for you (via search summary of [Archon auto logging](https://www.archon.gg/wow/articles/news/archon-app-auto-logging)).
  - The uploader is closed source: [RPGLogs/Uploaders-archon](https://github.com/RPGLogs/Uploaders-archon) publishes only binaries (v9.6.260, 2026-11-04). I did not download or inspect it.
- **Warcraft Recorder (open source, added Forever on 2026-10-29).**
  - It watches the `Logs` folder with `fs.watch` and reads appended bytes ([CombatLogWatcher.ts](https://github.com/aza547/wow-recorder/blob/main/src/parsing/CombatLogWatcher.ts)).
  - Because lines arrive late, it keeps a rolling video buffer. When a start line finally arrives it "cut[s] back into the buffer" by `now − line timestamp` ([LogHandler.ts lines ~277–281](https://github.com/aza547/wow-recorder/blob/main/src/parsing/LogHandler.ts)). In other words, it accepts the delay and reconstructs the past rather than trying to beat it.
  - Forever support: [PR #921](https://github.com/aza547/wow-recorder/pull/921). The Forever client uses the retail executable name and is told apart by its `_forever_` folder ([rust-ps commit](https://github.com/aza547/rust-ps/commit/4297b5299121bba0996496e66e4fb28945c13991)).
- **WoWAnalyzer** reads reports through the Warcraft Logs API, not the raw file, so it is not relevant to latency. Not inspected further.
- **WowCoach** publishes a machine-readable format spec, v22, "verified_against_patch: 12.0+", last updated 2026-10-10 ([spec.yaml](https://wowcoach.gg/docs/combat-log/spec.yaml)). Its Forever analyser takes uploads after the fact ([forever.wowcoach.gg](https://forever.wowcoach.gg/)).

### 1.3 Did Midnight (12.0) / Forever change the file?

- **The file still exists and is still written.**
  - `COMBAT_LOG_EVENT[_UNFILTERED]` is removed for addons in 12.0 ([wiki](https://warcraft.wiki.gg/wiki/COMBAT_LOG_EVENT)).
  - The text file is unaffected. A real Forever header reads `COMBAT_LOG_VERSION,22,ADVANCED_LOG_ENABLED,1,BUILD_VERSION,1.60.1,PROJECT_ID,18` ([Forever trace](https://github.com/dbspringer/nameplatesct-reforged/blob/main/spec/traces/warlock-line-kind.combatlog.txt); also [wowlab §8](https://github.com/sandgraal/wowlab/blob/main/docs/LAB_FORMATS.md) and [DOINK](https://github.com/ryan-flan/DOINK/blob/main/CLAUDE.md)).
  - `BUILD_VERSION` holds the patch only, not the build.
  - One test fixture shows `PROJECT_ID,7` ([OpenLogs](https://github.com/Shaydera/OpenLogs/blob/main/crates/openlogs-cli/src/inspect.rs)). Real traces show 18, so treat 18 as correct.
- **Format version 22 (12.0+).** Per WowCoach:
  - The advanced block is now **19 fields**: two always-0 fields were inserted before `power_type`.
  - `COMBATANT_INFO` gained a field at offset 7 in 12.0.
  - The timestamp includes the year and a UTC offset (`M/D/YYYY H:MM:SS.fff-4`).
  - Source: [spec.yaml](https://wowcoach.gg/docs/combat-log/spec.yaml). A 2025 player also counted 19 advanced fields ([forum](https://us.forums.blizzard.com/en/wow/t/advanced-combat-log-documentation-requested/2042184)).
- **Fields present on a real Forever open-world line** (`SPELL_CAST_SUCCESS` by a player, [trace](https://github.com/dbspringer/nameplatesct-reforged/blob/main/spec/traces/warlock-open-world.combatlog.txt)):
  - `…,Player-…,0000000000000000,1092,1092,481,0,641,0,0,0,0,949,1050,80,-9203.05,206.64,1429,2.2001,22`
  - That is HP 1092/1092, power 949/1050, cost 80, **world** x/y −9203.05/206.64, `uiMapID` **1429** (Elwynn Forest), facing 2.2 rad, and 22 in the "level" slot.
  - Inside a dungeon the same character logged `uiMapID 0` and small local coordinates ([trace](https://github.com/dbspringer/nameplatesct-reforged/blob/main/spec/traces/warlock-line-kind.combatlog.txt)). Expect no usable map position in instances, which matches the addon API.
  - The last field is **item level for players, NPC level for NPCs** ([wiki](https://warcraft.wiki.gg/wiki/COMBAT_LOG_EVENT), [WowCoach](https://wowcoach.gg/docs/combat-log/spec.yaml)). It cannot replace `UnitLevel`.
- **Position is in world yards, not 0–1 map coordinates.** Convert with the `MAP_CHANGE` bounds line, or with UiMapAssignment as in the main doc.
- **GUID obfuscation.**
  - Forever beta 5 replaced "the server ID and spawn ID portions of creature, vehicle, pet and game object GUIDs … with obfuscated values to prevent comparison to determine layer" ([Patch 1.60.1/API changes](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes)).
  - The note sits under addon security. It does not say whether it applies to the file.
  - In the public Forever traces, file GUIDs (`Creature-0-4621-0-1104-278863-00015A5CA2`) and addon-side GUIDs (`Creature-0-4621-0-13-278863-4397C51F27`) look different ([trace](https://github.com/dbspringer/nameplatesct-reforged/blob/main/spec/traces/warlock-open-world.combatlog.txt), [addon trace](https://github.com/dbspringer/nameplatesct-reforged/blob/main/spec/traces/warlock-open-world.lua)). That hints the file is not obfuscated. **UNVERIFIED.**
  - Player GUIDs in those traces were anonymised by the repo author.
- **Removed events.** No source lists events removed from the *file* in 12.0 or Forever. **UNVERIFIED.**

### 1.4 Which events put the player's own position in the log

The advanced block describes one unit. Always match its `info_guid` against the player's GUID ([WowCoach](https://wowcoach.gg/docs/combat-log/spec.yaml)). The block appears on these events ([WowCoach](https://wowcoach.gg/docs/combat-log/spec.yaml), [wowarenalogs CombatAdvancedAction.ts](https://github.com/wowarenalogs/wowarenalogs/blob/main/packages/parser/src/actions/CombatAdvancedAction.ts)):

| Event | Block describes | Gives *your* position when… | Out of combat? |
|---|---|---|---|
| `SPELL_CAST_SUCCESS` | caster (player) — seen in Forever traces with `info_guid` = caster ([trace](https://github.com/dbspringer/nameplatesct-reforged/blob/main/spec/traces/warlock-open-world-pet.combatlog.txt)) | you finish any cast: mount, gathering (Herb Gathering / Mining), Hearthstone, buffs, crafting, eating/drinking start | **yes**, if the client logs non-combat casts. Forever traces show self-casts like "Summon Hawk". Whether a mount, gathering or crafting cast is logged is **UNVERIFIED**; measure it |
| `SPELL_HEAL`, `SPELL_PERIODIC_HEAL` | target | you are healed, including HoTs, Drain Life and possibly food ticks | food regen as periodic heal is **UNVERIFIED** |
| `SPELL_ENERGIZE`, `SPELL_PERIODIC_ENERGIZE` | target | you gain mana or rage from a spell (e.g. Blessing of Wisdom ticks in a Forever trace, [trace](https://github.com/dbspringer/nameplatesct-reforged/blob/main/spec/traces/warlock-stockade.combatlog.txt)) | yes when it ticks. Drink regen is **UNVERIFIED** |
| `ENVIRONMENTAL_DAMAGE` | target (you) | falling, drowning, fatigue, lava, fire | yes |
| `SPELL_DAMAGE`, `SPELL_PERIODIC_DAMAGE`, `RANGE_DAMAGE` | target | something damages *you* | combat |
| `SWING_DAMAGE` | source | *you* melee something | combat |
| `SWING_DAMAGE_LANDED` | target | something melees *you* | combat |

Events with **no** advanced block: `SPELL_CAST_START`, the `SPELL_AURA_*` events, `UNIT_DIED` ([WowCoach](https://wowcoach.gg/docs/combat-log/spec.yaml)). Simply walking or flying produces nothing.

The log also carries every nearby player's position. Filter to your own GUID before anything leaves the PC.

### 1.5 `ZONE_CHANGE` and `MAP_CHANGE`

- **Both are still written in Forever.**
  - The open-world trace starts with `ZONE_CHANGE,0,"Elwynn Forest",0` and then `MAP_CHANGE,1429,"Elwynn Forest",-7939.58,-10254.17,1535.42,-1935.42` at the same timestamp ([trace](https://github.com/dbspringer/nameplatesct-reforged/blob/main/spec/traces/warlock-open-world.combatlog.txt)).
  - Dungeon traces show the header repeated, followed by `ZONE_CHANGE`, after a loading screen ([trace](https://github.com/dbspringer/nameplatesct-reforged/blob/main/spec/traces/warlock-spells.combatlog.txt)).
- **Formats** ([WowCoach](https://wowcoach.gg/docs/combat-log/spec.yaml)):
  - `ZONE_CHANGE` = `instanceID, zoneName, difficultyID`.
  - `MAP_CHANGE` = `uiMapID, name, maxX, minX, maxY, minY`.
- **Not yet verified:**
  - Whether `MAP_CHANGE` fires on *every* open-world `uiMapID` change, such as walking from Elwynn into Westfall, or only on logging start and loading screens. **UNVERIFIED; measure.**
  - When these lines are flushed. **UNVERIFIED; measure.**

### 1.6 Can an addon cause harmless lines without protected calls?

- **No write API exists.**
  - Forever's `C_CombatLog` has filter and retention functions only ([CombatLogDocumentation.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/CombatLogDocumentation.lua)).
  - Casting is protected, so an addon cannot generate `SPELL_CAST_SUCCESS`.
- **`C_ChatInfo.SendAddonMessageLogged` does not help.** It is "logged" for GM review of user-generated content. Nothing says it reaches the combat log file ([wiki](https://warcraft.wiki.gg/wiki/API_C_ChatInfo.SendAddonMessageLogged), [ChatInfoDocumentation.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/ChatInfoDocumentation.lua)).
- **`LoggingCombat(false)` then `LoggingCombat(true)` is callable by addons, rate-limited to 5 calls per 10 s shared with `/combatlog`** ([wiki](https://warcraft.wiki.gg/wiki/API_LoggingCombat)). The toggle *probably*:
  - writes a fresh header plus `ZONE_CHANGE` and `MAP_CHANGE` (seen when logging starts, above);
  - **may** flush the buffer when logging stops.

  Both are **UNVERIFIED**.
- **ToS caveat (my assessment).** The 10.2 buffering is a deliberate Blizzard protection. Toggling logging to force flushes *during combat* would be defeating that protection, the same reasoning as not encoding secret values. That is not zero-risk. Out of combat the intent is benign, but Blizzard has not addressed it. If you use the toggle at all, use it only out of combat and at low frequency (≥ 30 s). Archon's own addon toggling logging is a precedent for addons calling `LoggingCombat` (via search summary of [Archon auto logging](https://www.archon.gg/wow/articles/news/archon-app-auto-logging)).

### 1.7 Route summary: combat-log tail

- **Latency.**
  - In combat: buffered until combat ends, then minutes. Expect 1–5 min after the pull ends ([DOINK](https://github.com/ryan-flan/DOINK/blob/main/CLAUDE.md), [Archon FAQ via search summary](https://www.archon.gg/eso/articles/help/archon-app-help-and-faq)).
  - Out of combat: unknown. Must measure.
- **Data.**
  - Your x/y (world), `uiMapID`, HP, power and facing whenever you cast, take environmental damage, or are healed or energized.
  - `MAP_CHANGE` and `ZONE_CHANGE`.
  - Target names, HP and NPC level in combat.
  - No XP or character level, and nothing while you simply move.
- **ToS risk.**
  - Passive tailing: **none known**. It is the Warcraft Logs, Warcraft Recorder and WowCoach model. Blizzard's 2023 change targeted real-time overlays and left post-combat parsing alone (per [Wowhead's 10.2 coverage](https://www.wowhead.com/news/combat-log-changes-with-patch-10-2-measures-to-prevent-out-of-game-overlays-335813), via search summary).
  - Forcing flushes: see 1.6.
- **Effort.** Medium:
  - tail and CSV-aware parse;
  - per-session file rotation (`WoWCombatLog-MMDDYY_HHMMSS.txt` in `_forever_\Logs\`, [wowlab](https://github.com/sandgraal/wowlab/blob/main/docs/LAB_FORMATS.md));
  - world-to-map coordinate conversion;
  - filtering out other players.

---

## 2. Other files the client writes during play

| File | When written | Carries | Source |
|---|---|---|---|
| `WTF\Account\…\SavedVariables\*.lua` | logout, `/reload`, exit only | anything the addon stores | [wiki Saved variables](https://warcraft.wiki.gg/wiki/Saved_variables); [wowlab §4](https://github.com/sandgraal/wowlab/blob/main/docs/LAB_FORMATS.md) |
| `WTF\Config.wtf`, `config-cache.wtf` | "not saved to Config.wtf until properly logging out or reloading the game". `config-cache.wtf` is synced to the server when `synchronizeConfig` is on | CVars | [wiki SetCVar](https://warcraft.wiki.gg/wiki/API_C_CVar.SetCVar), [wiki CVars](https://warcraft.wiki.gg/wiki/Console_variables) |
| `Logs\WoWChatLog.txt` | at exit (Forever beta: 0 bytes all session, filled on exit) | received chat, including whispers both ways | [DOINK](https://github.com/ryan-flan/DOINK/blob/main/CLAUDE.md) (third-party observation); [wowprogramming (old)](https://wowprogramming.com/docs/api/LoggingChat.html) |
| `Logs\WoWCombatLog-*.txt` | buffered (section 1) | see section 1 | above |
| `Screenshots\WoWScrnShot_*.jpg` | immediately on `Screenshot()`. Forever lists `Screenshot` with no restriction flag. Retail 11.x/12.x: callable from a timer without a hardware event | pixels only | [ClientDocumentation.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/ClientDocumentation.lua); [DOINK](https://github.com/ryan-flan/DOINK/blob/main/CLAUDE.md) (third-party observation) |
| `Logs\taint.log` | while `taintLog` > 0. Flush timing **UNVERIFIED** | taint and restricted-action events only | [wiki CVars](https://warcraft.wiki.gg/wiki/Console_variables) |
| `Logs\AddOnLoad.log` | at addon load, when `addonLoadDebugging` is on | load info only | wiki CVar list (via search summary of [Console variables](https://warcraft.wiki.gg/wiki/Console_variables)) |
| developer log (path unknown) | `developerLog` (0) / `developerLogWriteToFile` (1). The file, path and flush are **UNVERIFIED** | possibly `C_Log.LogMessage` text | [wiki CVars](https://warcraft.wiki.gg/wiki/Console_variables), [LogDocumentation.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/LogDocumentation.lua), [wiki C_Log.LogMessage (stub)](https://warcraft.wiki.gg/wiki/API_C_Log.LogMessage) |
| `macros-cache.txt`, `bindings-cache.wtf` | **UNVERIFIED**. No source found. Commonly understood to be written at logout | macros and keybinds | none found |

**Notes on these files.**

- **Screenshots** are a real during-play file write that an addon can trigger. DOINK already uses one as its realtime transport: it paints a 1-frame bit grid, calls `Screenshot()`, and its companion decodes the JPEG ([DOINK](https://github.com/ryan-flan/DOINK/blob/main/CLAUDE.md)).
  - It is still **pixel encoding read by an external program**, the same family the user ruled out as grey. The file is written by the game rather than captured from the screen, which is arguably milder, but Blizzard has not addressed it.
  - Not zero-risk. Listed for completeness.
- **`C_Log.LogMessage` is the most interesting unknown.**
  - It is an unrestricted public API. Its only restriction is that secret arguments are allowed only when untainted ([LogDocumentation.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/LogDocumentation.lua)). Blizzard's own glue UI uses it for diagnostics ([Blizzard_CharacterCreate.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_CharacterCreate/Mainline/Blizzard_CharacterCreate.lua)).
  - If enabling `developerLog` makes the client write those messages to a file under `Logs\` during play, an addon could write `mapID,x,y,level,xp` lines to disk every few seconds, with no pixels, no chat and no protected calls.
  - The ToS risk would be as low as reading the combat log: Blizzard's own logging, and a file the client writes. Everything here is **UNVERIFIED** until tested (section 8, test E).
  - **Tested 2026-11-05: it does not work.** `DeveloperLog.log` stayed empty during play and after `/reload` ([live-tracking.md](../design/live-tracking.md#developer-log)).
  - It must never carry secret values, and the API rejects tainted secrets anyway.
- **`taint.log`** can only be steered by deliberately tainting things. That is an abuse of a debugging feature. Not recommended.
- **Force-saving SavedVariables.**
  - No such API exists in the Forever API docs. `C_AddOns.SaveAddOns` saves addon enable state, not variables ([AddOnsDocumentation.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/AddOnsDocumentation.lua)).
  - The only triggers are `/reload` (`ReloadUI`/`C_UI.Reload`), logout and exit ([wiki](https://warcraft.wiki.gg/wiki/Saved_variables)).
  - No source says the modern client ever saves periodically. **UNVERIFIED; measure** by watching mtime through a long session with loading screens.

---

## 3. Chat-log route

- **Can an addon put text into WoWChatLog.txt without a hardware event?** Yes.
  - On the Forever beta, `SendChatMessage(…, "WHISPER")` to your own full name worked from a `C_Timer` callback, and the whisper appears in the chat log in both directions ([DOINK](https://github.com/ryan-flan/DOINK/blob/main/CLAUDE.md), third-party observation).
  - SAY, YELL, CHANNEL and, since Forever beta 4, EMOTE need a hardware event outdoors ([wiki SendChatMessage](https://warcraft.wiki.gg/wiki/API_SendChatMessage), [Patch 1.60.1/API changes](https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes)).
  - Forever adds a chat-messaging lockdown restriction ([RestrictedActionsConstantsDocumentation.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/RestrictedActionsConstantsDocumentation.lua)).
  - `BNSendWhisper`/`C_BattleNet.SendWhisper` is restricted and flagged `RestrictedForMacroChatMessages` ([BattleNetDocumentation.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/BattleNetDocumentation.lua)).
- **It does not help latency.** The file is written at exit (section 2), so this route is as slow as SavedVariables and worse in every other way.
- **Throttle and spam-flag risk.**
  - Chat throttle numbers are not published. The addon-message throttle (10 per prefix, regenerating 1/s, with whispers outside instances exempt) applies to `SendAddonMessage`, not to chat ([wiki](https://warcraft.wiki.gg/wiki/API_C_ChatInfo.SendAddonMessageLogged)).
  - Automated whispers to yourself are invisible to others, so nobody can report them. Still, a steady stream of addon-generated chat is the kind of pattern anti-spam systems look at. I found no Blizzard statement either way.
  - **Honest assessment: low but not zero risk, and no benefit. Do not use.**
- **Discord guild-chat bridge (official, Forever-supported).**
  - Blizzard relays guild chat to a linked Discord channel in "modern World of Warcraft" and "World of Warcraft: Forever" ([Blizzard, Nov 4](https://worldofwarcraft.blizzard.com/en-us/news/24296228/curse-of-ulatek-link-your-battlenet-account-to-discord)).
  - A Discord bot could read position lines posted to a private one-person guild within seconds.
  - **But** on retail 12.1.5, `SendChatMessage(…, "GUILD")` from a timer fails with "Interface action failed because of an addon"; it only works from a key press ([DOINK](https://github.com/ryan-flan/DOINK/blob/main/CLAUDE.md), third-party observation, 2026-11-02). Forever is untested.
  - Used as a **manual "check in" keybind** (one key press = one message), this is zero-risk: it is ordinary chat. It is not live.
  - Any trick to piggy-back the hardware-event requirement on other key presses would be circumventing a protection. Don't.

---

## 4. Official / first-party presence

### 4.1 Battle.net friends rich presence

- **What the client exposes.** The Forever client exposes, for friends only, `areaName`, `characterLevel`, `richPresence`, `wowProjectID` and more via `C_BattleNet.GetFriendGameAccountInfo` ([BattleNetDocumentation.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/BattleNetDocumentation.lua)). Forever's friends frame shows rich presence for other game modes ([FriendsFrame.lua (Camelot)](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_FriendsFrame/Camelot/FriendsFrame.lua)).
- **No export.** That data is only available *inside a game client*. It would need a second account logged in, plus the same export problem.
- **No public Battle.net social or friends web API was found.** The published WoW web APIs are game data and profile only ([co-discord-bot #48](https://github.com/plohkoon/co-discord-bot/issues/48), [Pewtro/blizzard-api](https://github.com/Pewtro/blizzard-api)). Reading presence out of the Battle.net desktop app's internals would be unauthorised.
- **Verdict: no zero-risk route.**

### 4.2 Battle.net Web API for Forever

- **Status.**
  - Pre-launch probe (2026-10-26): every guessed namespace returned 403 ([co-discord-bot #48](https://github.com/plohkoon/co-discord-bot/issues/48)).
  - Community probe (2026-11-02): `profile-forever-{region}`, `dynamic-forever-{region}` and versioned `static-1.60.1_70399-forever-{region}` respond in US, EU and TW, not KR. `/profile/user/wow` returns Forever characters with a new `surname` field and `protected_character` links. Character endpoints were not yet routing ([forum post by vman-2993](https://us.forums.blizzard.com/en/blizzard/t/world-of-warcraft-forever-api-namespace/59774)).
  - No Blizzard staff reply in either forum thread ([59774](https://us.forums.blizzard.com/en/blizzard/t/world-of-warcraft-forever-api-namespace/59774), [59595](https://us.forums.blizzard.com/en/blizzard/t/when-will-we-gain-api-access-to-forever-apis/59595)). **Post-launch status UNVERIFIED.**
- **Protected Character Profile Summary.**
  - Returns `position` and `bind_position` (zone, map, x, y, z, facing), `money` and `protected_stats` ([response type](https://github.com/Pewtro/blizzard-api/blob/main/packages/wow/src/account-profile/types.ts), [example payload, 2020](https://us.forums.blizzard.com/en/blizzard/t/wow-profile-api-character-position-zone-coordinates/13506)).
  - Requires the account owner's OAuth token (`wow.profile`).
  - **Latency: UNVERIFIED.** Profile data is generally committed at logout ([forum on last_login_timestamp](https://us.forums.blizzard.com/en/blizzard/t/wow-last-login-timestamp-and-api-performance/9028)), but whether `position` also changes on periodic server-side saves while online has not been checked.
- **Data.** Position (world x/y/z plus zone and map), and from the character profile, level and (probably) XP. No target, no combat state.
- **ToS risk.** None: an official API used with the player's consent.
- **Effort.** Low-medium: an OAuth flow on the website, then polling within the rate limits.

### 4.3 Discord rich presence

- **Blizzard's integration covers chat only.** The Battle.net↔Discord link covers guild-chat relay, display name and linking ([Blizzard](https://worldofwarcraft.blizzard.com/en-us/news/24296228/curse-of-ulatek-link-your-battlenet-account-to-discord), [Blizzard 12.1 announcement](https://worldofwarcraft.blizzard.com/en-us/news/24286284)). BlizzardWatch notes the link consent screen shares "Your online and in-game status" ([BlizzardWatch](https://blizzardwatch.com/2026/08/18/wow-patch-12-1-will-let-attach-warcraft-guild-discord-server-integration/)), with no detail.
- **No zone presence was found.** No source shows WoW pushing zone or level as Discord Rich Presence. The Forever API has a `C_Discord` namespace for guild linking only ([DiscordDocumentation.lua](https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/DiscordDocumentation.lua)). **UNVERIFIED** whether "in-game status" includes zone. If it ever did, a Discord bot with the presence intent could read it, but that is speculation.
- **CraftPresence is not a zero-risk alternative.** It is the community addon that shows WoW zone in Discord and lists Forever support ([CurseForge](https://www.curseforge.com/wow/addons/craftpresence)). Its author describes the pixel generation and reading as "not verified by Blizzard" (via search summary of [legacy-wow](https://legacy-wow.com/tbc-burning-crusade-classic-wow-addons/craftpresence-tbc/)), so it is the pixel route.

### 4.4 WoW Companion app

Retired with The War Within pre-patch, July 2024 ([Blizzard news](https://worldofwarcraft.com/en-gb/news/24061010), [Blizzard support](https://us.support.blizzard.com/en/article/85524)). It never had live position anyway.

---

## 5. Overwolf GEP

- **Blizzard arrangement.**
  - No public Blizzard–Overwolf agreement for WoW was found.
  - Overwolf's generic claim is that it "work[s] closely with game publishers" ([app page](https://www.overwolf.com/app/rastudio-wow_forever_map)).
  - The only explicit Activision Blizzard approval found concerns Call of Duty, from a secondary source (via search summary of [earlygame](https://earlygame.com/call-of-duty/overwolf-banned-warzone)).
- **How it gets data.**
  - Undisclosed. An Overwolf staffer called it proprietary and said they work directly with developers (via search summary of [discuss.overwolf.com](https://discuss.overwolf.com/t/how-does-overwolf-grab-game-data/1605); that site did not resolve for me).
  - Overwolf says events can be disabled at "request by the relevant game studio" ([GEP intro](https://dev.overwolf.com/ow-native/live-game-data-gep/live-game-data-gep-intro)).
  - For New World, a developer attributed a breakage to a game update moving the location value in memory (via search summary of [Overwolf forum](https://discuss.overwolf.com/t/events-getinfo-cb-for-new-world-updates-no-location-data-anymore/2967)). That suggests memory reading for at least some games. **UNVERIFIED for WoW.**
- **Forever support.**
  - Yes, as a separate game page. It offers `location.zone` (main and sub zone IDs) and `player_info` (level, XP, max XP, money, name, professions), and **no x/y** ([WoW Forever GEP](https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/wow-forever)).
  - The retail WoW page does list a `location` event with x/y and map ([WoW GEP](https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/world-of-warcraft)).
  - An Overwolf app, "WoW Forever Map" by RAStudio (v4.4.7, listed 2026-11-05), advertises "Live position tracking". Its mechanism is not stated ([app page](https://www.overwolf.com/app/rastudio-wow_forever_map)).
  - From 2026-11-01 Overwolf only approves apps with Overwolf monetization ([Overwolf docs banner](https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/world-of-warcraft)).
- **Is it safe?**
  - No Blizzard statement either way.
  - A Taiwan player reports a March 2024 suspension he attributes to Overwolf. The reply, from Vrakthris (apparently Blizzard support; not marked staff in my fetch), was "you'll need to take it up with" the TW region ([forum](https://us.forums.blizzard.com/en/wow/t/overwolf-is-not-safe/1844656)).
  - **Not zero-risk. Excluded.**

---

## 6. What Blizzard has said about third-party tools

- **EULA §1.C** prohibits, among other things:
  - "any unauthorized process or software that intercepts, collects, reads, or 'mines' information" from the games;
  - connections using third-party programs "not expressly authorized by Blizzard".

  Source: [Blizzard EULA](https://www.blizzard.com/en-us/legal/fba4d00f-c7e4-4883-b8b9-1b4500a402ea/blizzard-end-user-license-agreement).
- **Kaivax, 2025-08-05:** "The use of third-party software that modifies the World of Warcraft game client is against our Terms of Service", and "We expect players to remove third-party software that bypasses the security of the game client" ([forum](https://us.forums.blizzard.com/en/wow/t/prohibitions-on-third-party-software/2142972)). No tools were named. Questions about TSM and Auctionator in that thread got no staff answer.
- **Kaivax, 2023-08-21:** Blizzard would change how the Advanced Combat Log is populated to stop overlays, citing §1-C ([forum](https://us.forums.blizzard.com/en/wow/t/nerfing-combat-logs-will-it-happen-should-it-happen/1654695/11)). Coverage says post-combat parsers such as Warcraft Logs were unaffected (via search summary of [Wowhead](https://www.wowhead.com/news/combat-log-changes-with-patch-10-2-measures-to-prevent-out-of-game-overlays-335813)). Blizzard's own words do not name Warcraft Logs.
- **UI Add-On Development Policy:** add-ons must be free and fully visible, and Blizzard can disable add-on functionality ([forum](https://us.forums.blizzard.com/en/wow/t/ui-add-on-development-policy/24534)). It says nothing about companion apps reading files.
- **Midnight announcements** ([combat philosophy](https://news.blizzard.com/en-us/article/24246290/combat-philosophy-and-addon-disarmament-in-midnight), [addon impact](https://news.blizzard.com/en-us/article/24244638/how-midnights-upcoming-game-changes-will-impact-combat-addons), [UI updates](https://news.blizzard.com/en-us/article/24223311/midnight-get-up-to-speed-with-user-interface-updates)) do not mention the combat-log file, Warcraft Logs or companion apps.
- **Warcraft Logs, Raider.IO desktop client and TSM desktop app.** No Blizzard statement explicitly authorising any of them was found.
  - Raider.IO's own policy says its desktop client collects character and combat-log data (via search summary of [Raider.IO privacy policy](https://raider.io/privacy-policy)).
  - TSM's app manages SavedVariables and AppHelper data ([TSM support](https://support.tradeskillmaster.com/addon/how-do-i-fix-an-error-about-appdatalua-being-empty)).
  - They operate openly at large scale, which is strong evidence of tolerance, but not authorisation.
- **Practical reading** (my assessment): the risk Blizzard acts on is real-time competitive advantage, automation, client modification and memory access. Reading files the client writes for the user is the long-standing tolerated pattern. Making it real-time is exactly what Blizzard pushed back on in 2023.

---

## 7. Ranked recommendation: zero-risk routes only

1. **SavedVariables + watcher (keep it).**
   - Latency: `/reload`/logout. Data: everything non-secret (position, level, XP, target snapshot).
   - Risk: none known. Effort: done.
2. **Combat-log tail with ACL on (add it).**
   - Latency: minutes (measure the out-of-combat flush). Data: position, uiMapID and map bounds whenever you cast, take environmental damage, or get healed or energized; zone and map changes; combat targets.
   - Risk: none known for passive reading. Do **not** toggle logging in combat to force flushes.
   - Effort: medium.
   - Feed the "last seen" map with the newest timestamped player position from whichever source is freshest.
3. **Battle.net Profile API, protected character (test it).**
   - Latency: unknown, logout at worst. Data: position, level, money.
   - Risk: none (official, OAuth). Effort: low-medium, server-side only.
   - If position refreshes while online, it is the cleanest "near-live" source available.
4. **`C_Log.LogMessage` + developer log (test it; potentially the best).**
   - If the client writes it to a file during play with short buffering, it gives seconds-level latency for any non-secret field, written by the addon itself.
   - Risk: none known (a Blizzard logging API plus reading a client log file). **Entirely UNVERIFIED.**
5. **Manual check-in to a Discord-linked guild (optional).**
   - One key press posts a position line to guild chat, and Blizzard relays it to Discord in seconds.
   - Risk: none (ordinary chat typed by the player). Not live.

Not zero-risk, excluded:
- pixel strip;
- screenshot grid (DOINK pattern);
- Overwolf;
- forcing combat-log flushes in combat;
- taint-log tricks;
- anything that listens to the game's text-to-speech or other OS hooks.

---

## 8. What to measure in game

Run with a companion-side logger that records `(wall clock ms, file, size, mtime)` every 100 ms for everything under `_forever_\Logs\`, `_forever_\Screenshots\` and `_forever_\WTF\`. Note the in-game wall time of each action (`/run print(date("%H:%M:%S"), GetTime())`).

**A. Combat log** (`/combatlog` on, Advanced Combat Logging on in System → Network):
1. **Start.** Run `/combatlog`. Is a new `WoWCombatLog-MMDDYY_HHMMSS.txt` created? When do the header, `ZONE_CHANGE` and `MAP_CHANGE` lines appear on disk?
2. **Out-of-combat self actions.** Note each action time and when its line hits disk:
   - a self-buff;
   - mounting;
   - Herb Gathering / Mining;
   - Hearthstone;
   - eating, and drinking;
   - jumping off something for falling damage (`ENVIRONMENTAL_DAMAGE`).

   Also record which of these produce a line at all, and with what `info_guid`.
3. **Combat.** Pull one mob and note the `PLAYER_REGEN_DISABLED` and `PLAYER_REGEN_ENABLED` times. Record the delay from combat end to the lines appearing. Repeat 5 times, plus once with a 3-minute fight.
4. **Idle.** Stay out of combat for 20 minutes, doing one self-buff every 2 minutes. Does the file ever grow without combat? Is there a periodic or size-based flush?
5. **Zone borders.** Walk Elwynn → Westfall → Duskwood, and a subzone inside a city. Is `MAP_CHANGE` written on each open-world uiMap change, or only at logging start and loading screens? Measure the lag.
6. **Logging toggle.** `/combatlog` off: does the buffer flush immediately? Then on again: header plus zone and map lines? Do this **out of combat only.**
7. **Exit paths.** `/reload`, logout to character select, exit: does each flush?
8. **GUIDs.** Compare the `UnitGUID("target")` printed by the addon with the same mob's GUID in the file. Same spawn portion means the file is not obfuscated.
9. **Last advanced field.** Record it for your character at two different levels and gear sets: is it item level or character level?
10. **Instances.** Record `uiMapID` and x/y in a dungeon (expect 0 and local coordinates).

**B. Chat log:**
- `/chatlog` on, whisper yourself.
- Record file size after the whisper, after `/reload`, after logout, and after exit.

**C. SavedVariables:**
- Play 60 minutes with several loading screens (instance in and out, boat or zeppelin) and no `/reload`.
- Does `WowTracker.lua` mtime ever change before logout?

**D. Config:**
- Change a setting in Options and run `/run C_CVar.SetCVar("screenshotQuality","4")`.
- Does `Config.wtf` or `config-cache.wtf` mtime change before reload or logout?

**E. Developer log (highest priority):**
1. List `Logs\` before the test.
2. Run `/console developerLog 1` (and confirm `developerLogWriteToFile` is 1).
3. Run `/run C_Log.LogMessage("wt-probe "..date("%H:%M:%S"))`.
4. List `Logs\` again and check:
   - Is there a new or grown file?
   - What lag?
   - What does each line look like?
   - Does it survive `/reload`?
5. Repeat every 5 s for 2 minutes to see the buffering.
6. Also try `C_Log.LogMessageWithPriority`. **Check whether the CVar is protected or reset at login.**

**F. Taint log (informational):**
- Run `/console taintLog 1`, `/reload`.
- Does `Logs\taint.log` grow during play or only at exit?

**G. Battle.net API:**
1. Authorise `wow.profile`, then call `GET /profile/user/wow?namespace=profile-forever-eu` (or your region).
2. Follow `protected_character`.
3. While staying logged in, move to a new zone and poll every 60 s for 30 minutes. When does `position` change: never, periodically (note the interval), or only after logout?
4. Also check that `level` and XP update.

**H. Discord bridge (only if you want check-ins):**
- `/run C_Timer.After(1,function() C_ChatInfo.SendChatMessage("test","GUILD") end)`: blocked or not in Forever?
- Then measure in-game-to-Discord lag for a typed guild line.

**I. Paths:**
- Confirm `World of Warcraft\_forever_\Logs\` and `...\_forever_\WTF\Account\<ACCOUNT>\SavedVariables\`.
- Confirm the `_forever_` folder per [wow-recorder's rust-ps change](https://github.com/aza547/rust-ps/commit/4297b5299121bba0996496e66e4fb28945c13991).

---

## Sources

**Blizzard (first-party)**
- Kaivax on Advanced Combat Log changes (2023-08-21): https://us.forums.blizzard.com/en/wow/t/nerfing-combat-logs-will-it-happen-should-it-happen/1654695/11
- Kaivax, Prohibitions on Third-party Software (2025-08-05): https://us.forums.blizzard.com/en/wow/t/prohibitions-on-third-party-software/2142972
- Combat log delay thread (2025): https://us.forums.blizzard.com/en/blizzard/t/wow-wow-combat-log-delay-%E2%80%93-seeking-api-or-workaround/55869
- Forever API namespace threads: https://us.forums.blizzard.com/en/blizzard/t/world-of-warcraft-forever-api-namespace/59774, https://us.forums.blizzard.com/en/blizzard/t/when-will-we-gain-api-access-to-forever-apis/59595
- Profile API position example (2020): https://us.forums.blizzard.com/en/blizzard/t/wow-profile-api-character-position-zone-coordinates/13506
- last_login_timestamp thread: https://us.forums.blizzard.com/en/blizzard/t/wow-last-login-timestamp-and-api-performance/9028
- Advanced Combat Log documentation thread (2025): https://us.forums.blizzard.com/en/wow/t/advanced-combat-log-documentation-requested/2042184
- Discord linking (Forever included): https://worldofwarcraft.blizzard.com/en-us/news/24296228/curse-of-ulatek-link-your-battlenet-account-to-discord, https://worldofwarcraft.blizzard.com/en-us/news/24286284
- Midnight addon articles: https://news.blizzard.com/en-us/article/24246290/combat-philosophy-and-addon-disarmament-in-midnight, https://news.blizzard.com/en-us/article/24244638/how-midnights-upcoming-game-changes-will-impact-combat-addons, https://news.blizzard.com/en-us/article/24223311/midnight-get-up-to-speed-with-user-interface-updates
- Companion app retirement: https://worldofwarcraft.com/en-gb/news/24061010, https://us.support.blizzard.com/en/article/85524
- EULA: https://www.blizzard.com/en-us/legal/fba4d00f-c7e4-4883-b8b9-1b4500a402ea/blizzard-end-user-license-agreement
- UI Add-On Development Policy: https://us.forums.blizzard.com/en/wow/t/ui-add-on-development-policy/24534
- Overwolf/Taiwan suspension thread: https://us.forums.blizzard.com/en/wow/t/overwolf-is-not-safe/1844656

**Blizzard UI source (Gethe/wow-ui-source, branch `forever`, 1.60.1)**
- LogDocumentation.lua (C_Log): https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/LogDocumentation.lua
- ClientDocumentation.lua (Screenshot): https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/ClientDocumentation.lua
- CombatLogDocumentation.lua: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/CombatLogDocumentation.lua
- ChatInfoDocumentation.lua: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/ChatInfoDocumentation.lua
- BattleNetDocumentation.lua: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/BattleNetDocumentation.lua
- DiscordDocumentation.lua: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/DiscordDocumentation.lua
- AddOnsDocumentation.lua: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/AddOnsDocumentation.lua
- RestrictedActionsConstantsDocumentation.lua: https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_APIDocumentationGenerated/RestrictedActionsConstantsDocumentation.lua
- FriendsFrame (Camelot): https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_FriendsFrame/Camelot/FriendsFrame.lua
- Blizzard_CharacterCreate (C_Log usage): https://github.com/Gethe/wow-ui-source/blob/forever/Interface/AddOns/Blizzard_CharacterCreate/Mainline/Blizzard_CharacterCreate.lua

**warcraft.wiki.gg**
- COMBAT_LOG_EVENT: https://warcraft.wiki.gg/wiki/COMBAT_LOG_EVENT
- LoggingCombat: https://warcraft.wiki.gg/wiki/API_LoggingCombat
- SendChatMessage: https://warcraft.wiki.gg/wiki/API_SendChatMessage
- SendAddonMessage(Logged): https://warcraft.wiki.gg/wiki/API_C_ChatInfo.SendAddonMessageLogged
- C_CVar.SetCVar: https://warcraft.wiki.gg/wiki/API_C_CVar.SetCVar
- Console variables: https://warcraft.wiki.gg/wiki/Console_variables
- C_Log.LogMessage: https://warcraft.wiki.gg/wiki/API_C_Log.LogMessage
- Saved variables: https://warcraft.wiki.gg/wiki/Saved_variables
- Patch 1.60.1/API changes: https://warcraft.wiki.gg/wiki/Patch_1.60.1/API_changes
- Patch 12.0.0/API changes: https://warcraft.wiki.gg/wiki/Patch_12.0.0/API_changes

**Tool source code and data**
- Warcraft Recorder: https://github.com/aza547/wow-recorder/blob/main/src/parsing/LogHandler.ts, https://github.com/aza547/wow-recorder/blob/main/src/parsing/CombatLogWatcher.ts, https://github.com/aza547/wow-recorder/blob/main/src/parsing/RetailLogHandler.ts, https://github.com/aza547/wow-recorder/blob/main/src/parsing/ForeverLogHandler.ts, https://github.com/aza547/wow-recorder/pull/921, https://github.com/aza547/rust-ps/commit/4297b5299121bba0996496e66e4fb28945c13991
- WowCoach combat log spec: https://wowcoach.gg/docs/combat-log/spec.yaml, https://forever.wowcoach.gg/
- wowarenalogs parser: https://github.com/wowarenalogs/wowarenalogs/blob/main/packages/parser/src/actions/CombatAdvancedAction.ts
- Archon uploader binaries: https://github.com/RPGLogs/Uploaders-archon
- Archon help (403 to fetch; via search summaries): https://www.archon.gg/classic-mop/articles/help/uploader, https://www.archon.gg/eso/articles/help/archon-app-help-and-faq, https://www.archon.gg/wow/articles/news/archon-app-auto-logging
- Real Forever combat-log traces: https://github.com/dbspringer/nameplatesct-reforged/tree/main/spec/traces
- DOINK (Forever beta and retail observations): https://github.com/ryan-flan/DOINK/blob/main/CLAUDE.md
- wowlab format notes: https://github.com/sandgraal/wowlab/blob/main/docs/LAB_FORMATS.md
- OpenLogs fixture: https://github.com/Shaydera/OpenLogs/blob/main/crates/openlogs-cli/src/inspect.rs
- Blizzard API typings (protected character): https://github.com/Pewtro/blizzard-api/blob/main/packages/wow/src/account-profile/types.ts
- Forever namespace probe: https://github.com/plohkoon/co-discord-bot/issues/48

**Overwolf / Discord / other**
- Overwolf GEP WoW Forever: https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/wow-forever
- Overwolf GEP WoW: https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/world-of-warcraft
- Overwolf GEP intro: https://dev.overwolf.com/ow-native/live-game-data-gep/live-game-data-gep-intro
- WoW Forever Map app: https://www.overwolf.com/app/rastudio-wow_forever_map
- Overwolf forum (via search summary): https://discuss.overwolf.com/t/how-does-overwolf-grab-game-data/1605, https://discuss.overwolf.com/t/events-getinfo-cb-for-new-world-updates-no-location-data-anymore/2967
- Overwolf and Call of Duty (via search summary): https://earlygame.com/call-of-duty/overwolf-banned-warzone
- CraftPresence: https://www.curseforge.com/wow/addons/craftpresence, https://legacy-wow.com/tbc-burning-crusade-classic-wow-addons/craftpresence-tbc/
- BlizzardWatch on Discord link: https://blizzardwatch.com/2026/08/18/wow-patch-12-1-will-let-attach-warcraft-guild-discord-server-integration/
- Wowhead 10.2 combat log (body via search summary): https://www.wowhead.com/news/combat-log-changes-with-patch-10-2-measures-to-prevent-out-of-game-overlays-335813
- mein-mmo 10.2 summary: https://mein-mmo.de/en/wow-has-introduced-a-nasty-trick-against-many-addons-and-programs,1057395
- Warcraft Tavern on Kaivax 2023: https://warcrafttavern.com/wow/news/blizzard-is-changing-how-the-advanced-combat-log-functions-in-dragonflight-for-season-3
- wowprogramming LoggingChat (old): https://wowprogramming.com/docs/api/LoggingChat.html
- TSM support: https://support.tradeskillmaster.com/addon/how-do-i-fix-an-error-about-appdatalua-being-empty
- Raider.IO privacy policy (via search summary): https://raider.io/privacy-policy
