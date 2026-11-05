# Design research: how live can the tracker be?

Status: research done; first in-game measurements in (2026-11-05), quiet-area test pending

## Problem

The map only updates when the game saves the addon's data, which happens on `/reload`,
`/wt sync`, logout or exit. In normal play that means the dot can be minutes or hours
behind. The goal is updates every second or so, or at least every few minutes, without
having to reload.

## Constraint: zero Terms-of-Service risk

Only routes that Blizzard provides, or that read files the game writes for players, are
acceptable. These are excluded even though they would work:

| Excluded route | Why |
|---|---|
| Pixel strip read off the screen (incl. screenshot tricks) | Grey area under the EULA's "reads or mines information" clause. Bots use the same technique. |
| Reading game memory, sniffing network traffic | Clear EULA violations; what Warden detects. |
| Overwolf | Not Blizzard-approved, unknown data source, one reported suspension. Its Forever feed has no x/y anyway. |
| Toggling combat logging to force writes | Works around a delay Blizzard added on purpose. |

## Why addons can't just send data

- **No way out of the game.** Addons have no network access and cannot write files. Their
  only disk output is SavedVariables, which the game writes on reload, logout or exit.
- **No force-save.** There is no API to write SavedVariables while playing.
- **The combat log is delayed on purpose.** Since Dragonflight season 3, Blizzard holds the
  combat log back so outside tools can't use it in real time
  ([Blizzard announcement](https://us.forums.blizzard.com/en/wow/t/nerfing-combat-logs-will-it-happen-should-it-happen/1654695?page=7),
  [coverage](https://warcrafttavern.com/wow/news/blizzard-is-changing-how-the-advanced-combat-log-functions-in-dragonflight-for-season-3)).

So **no known risk-free route reaches 1 second.** The realistic target is somewhere between
one and a few minutes.

## Options evaluated

| # | Route | Latency | Data | Risk | Status |
|---|---|---|---|---|---|
| 0 | SavedVariables + file watcher (**current**) | At `/reload`, `/wt sync`, logout | Everything the addon collects | None | Built |
| 1 | Battle.net API, protected character | Unknown: logout only, or periodic while online | Position (world x/y, zone, map), level | None (official API, your consent) | Needs testing |
| 2 | Combat log tail (advanced logging on) | **Measured: 40–65 s in a busy zone** (written in ~32 KB batches); slower when quiet | Your position whenever you fight, zone changes with map bounds, enemy names, HP | Very low (how Warcraft Logs works) | Measured; best candidate |
| 3 | Developer log (`C_Log.LogMessage`) | Unknown | Whatever the addon writes | None if it works | Needs testing |
| 4 | Chat log | Written at exit | — | Low | Rejected |
| 5 | Config.wtf / other WTF files | Written at logout or reload | — | None | Rejected |
| 6 | Battle.net friends presence | Live, but in-game only | Zone, level | None | Rejected: no export |
| 7 | Discord guild bridge | Manual key press only | Free text | None | Rejected: not automatic |

### 1. Battle.net API: protected character

- **What it returns.** Blizzard's profile API returns `position` (zone, map, world x/y/z,
  facing) for your own characters
  ([example response](https://us.forums.blizzard.com/en/blizzard/t/wow-profile-api-character-position-zone-coordinates/13506)).
- **What it needs.** A one-time Battle.net login granting the `wow.profile` scope, and a
  free API client from develop.battle.net.
- **Forever support.** A Forever namespace (`profile-forever-{region}`) was seen responding
  on 2026-11-02; character routes were not working yet then.
- **The open question is freshness.** If `position` updates while you are online, polling
  it every minute gives a risk-free, reload-free map. If it only updates at logout, it is
  no better than what we have.
- **Fit with what's built.** World x/y can already be drawn: the website converts world
  coordinates with the game's own map tables.

### 2. Combat log tail

- **What gets logged.** With advanced combat logging on, every spell you cast logs your
  world position and map ID, even out of combat. That includes mounting, gathering and
  eating. Zone and map changes are logged too.
- **Gaps.** Walking without casting logs nothing. There is no XP or character level in
  the log.
- **Latency.** Minutes, by Blizzard's design. One Forever beta report saw about 4 minutes;
  Warcraft Recorder allows up to 10 minutes of silence for retail and Forever logs.
- **Bonus.** It records who you fought, by name, which the addon cannot see during
  combat.
- **Format.** Combat log version 22. The advanced block is 19 fields, and position is in
  world yards. Inside dungeons the map ID is 0.

### 3. Developer log

- **The idea.** `C_Log.LogMessage` is available to addons in Forever with no
  restrictions. With the `developerLog` setting on, it may write to a file under `Logs`.
- **Why it matters.** If that file is written promptly, the addon could write its own
  status line every few seconds. That would be the best risk-free channel.
- **Status.** Completely untested.

## Measurements (2026-11-05)

Taken with `wowtracker-companion probe` on the Forever client (build 1.60.1), a warrior
in Teldrassil at launch, with many other players nearby.

### Files written during play

Over 43 minutes of play before combat logging was turned on, the only files written
without a reload were `Logs/Hotfix.log` (server hotfix data, about every 7 minutes),
`Logs/gx.log` (graphics, window focus) and `Logs/QuestCache.log` (quest completion
checks, 10–40 s late). None contains character data. SavedVariables and the WTF caches
were written only at login and `/reload`, as expected.

### Combat log

| Write | Time | Bytes added | Lines | Newest line age | Oldest line age |
|---|---|---|---|---|---|
| 1 | 18:20:00 | 47,906 (incl. header) | 170 | 0.9 s | 41.6 s |
| 2 | 18:20:42 | 33,075 | 111 | 1.1 s | 39.6 s |
| 3 | 18:21:47 | 33,434 | 115 | 1.0 s | 64.8 s |

- **Written in batches when a ~32 KB buffer fills, not on a timer.** The newest line in
  each batch is about 1 s old, so the delay is how long the buffer takes to fill. In a
  crowded zone that was 40–65 s. Alone or out of combat it will be longer, which likely
  explains the 4-minute beta report. Not yet measured.
- **The file starts at once.** `/combatlog` created the file immediately, with a header,
  `ZONE_CHANGE` and `MAP_CHANGE` timestamped at the moment logging started.
- **Header:** `COMBAT_LOG_VERSION,22,ADVANCED_LOG_ENABLED,1,BUILD_VERSION,1.60.1,PROJECT_ID,18`.
- **Most lines are about other people.** Of the first 281 lines, 213 carried a position:
  41 about the player and 172 about nearby players and mobs. The tracker must keep only
  lines whose advanced-info GUID is the player's own; the player is the unit flagged
  `0x511` ("mine"). It must never forward other players' names or positions.
- **The player's position appeared only in combat lines:** `SWING_DAMAGE` (13),
  `SWING_DAMAGE_LANDED` (23) and `SPELL_PERIODIC_DAMAGE` (5). Walking without fighting
  wrote nothing about the player.
- **Map coordinates need no lookup table.** `MAP_CHANGE` gives the uiMapID and its world
  bounds, e.g. `MAP_CHANGE,1438,"Teldrassil",11831.25,8437.5,3814.58,-1277.08`
  (= X top, X bottom, Y left, Y right). For a world position (X, Y):
  - map x = (Y left − Y) / (Y left − Y right)
  - map y = (X top − X) / (X top − X bottom)

  Checked against the night elf start at Aldrassil (world 10311, 832), which comes out
  at 58.6, 44.8 as expected.

### Developer log

`Logs/DeveloperLog.log` stayed at 0 bytes for the whole session. If the `developerLog`
commands and `C_Log.LogMessage` were run, this route does not write to a file in Forever.
Unconfirmed whether they were run.

### Still to measure

- Combat log delay in a quiet zone, alone, and while walking without fighting.
- Whether `/reload` or logout writes out the remaining buffer.
- The developer log, if it was not tested.

## Prior art: wow.skysoft.dk

[wow.skysoft.dk](https://wow.skysoft.dk/) is another person's Forever tracker that does
update every second. It was examined on 2026-11-05 using only what it serves to any
visitor: the page, `app.js`, `/api/state`, `/api/history` and `/api/stream`.

**Observed:**

- **One snapshot a second.** The page keeps a server-sent event stream open
  (`/api/stream`, event `live`). Each event carries a full snapshot: map ID, x/y to 4
  decimals, level, XP, gold, combat, dead/ghost, zone and subzone. Separate `update`
  events trigger a full reload of state and history.
- **Live in-game changes.** Over 12 seconds of polling while the character fought, XP rose
  with each kill (1544 → 1563 → 1582 → 1601), combat switched on and off, and the
  position moved. `updatedAt` was never more than 1 second behind the server clock.
- **The source is an addon.** Current health and power are always `null`, matching
  Forever's secret values. The page itself says "Live data from an in-game addon". Memory
  reading would have the real numbers.
- **Stack.** A Python backend ("ForeverTracker/0.1") behind Caddy. The upload endpoint is
  not public, and no source code was found.

**Inferred (not proven):** the data leaves the game every second, but addons can't make
network requests or write files during play, and the combat log is minutes behind with
no XP in it. The remaining explanation is screen reading: the addon draws the data as
pixels or text, and a program on the PC captures it. That is the pixel route this design
excludes, so this site does not show a new risk-free route.

**Features worth borrowing** (all work with SavedVariables):

- quest log with objectives;
- journal of quests, levels, zones, logins and deaths;
- XP per hour and time-to-next-level estimate;
- levelling-curve chart by played time;
- "Live" / "Online" / "Offline" pill with "last update N ago".

## Recommendation

1. **Keep option 0** as the baseline. It is complete and reliable.
2. **Measure before building.** Run `wowtracker-companion probe` during a normal session
   (see [setup.md](../setup.md#measuring-what-the-game-writes-live-probe)). It answers
   options 2 and 3:
   - how old combat log lines are when they reach disk;
   - how many lines carry your position;
   - whether the developer log writes a file at all.
3. **Test option 1**, using a small checker that logs in once and polls your position every
   minute for 30 minutes while you move between zones.
4. **Build in this order, depending on results:**
   - If the API position updates while online, add it to the backend as the main live
     source. The backend polls; nothing changes in game.
   - If the developer log writes promptly, add it to the addon and companion.
   - Otherwise, add the combat log tail to the companion. It gives the trail and fight
     details within minutes. Also add an "updated N min ago" note on the map.

   **Update after measuring:** the combat log tail is now the leading option. In a busy
   zone it lands within about a minute and carries the player's position during fights.
   It can be built now, filtered to the player's own lines; the Battle.net API test can
   follow. See [Measurements](#measurements-2026-11-05).

## Measurements needed

| What | How | Decides |
|---|---|---|
| Combat log delay | Probe: "newest event … s old" values | Whether option 2 is worth building |
| Position coverage in the combat log | Probe: lines "with position" while walking, mounting, fighting | How smooth the trail would be |
| Developer log | Probe + `/console developerLog 1`, `/console developerLogWriteToFile 1`, `/run C_Log.LogMessage("wt-probe "..date("%H:%M:%S"))` | Whether option 3 exists at all |
| API position freshness | Checker polling every 60 s while you play | Whether option 1 beats option 0 |
| Game folder and process name | Probe output, Task Manager | Companion defaults (`_forever_`, `Wow.exe` assumed) |

## Sources

- Full research with all sources: [research/near-live-options.md](../research/near-live-options.md)
- Earlier research (addon API, secret values, map maths):
  [research/wow-classic-live-tracker.md](../research/wow-classic-live-tracker.md)
