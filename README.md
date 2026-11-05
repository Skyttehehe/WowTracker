# WowTracker

Shows where your *World of Warcraft: Forever* character is on a map, with level, XP,
what you are fighting and your recent route.

```
 Gaming PC                                   Server / LAN                          Internet
┌──────────────────────────────┐          ┌──────────────────────────┐        ┌──────────────┐
│ WoW + WowTracker addon       │          │ wowtracker-backend       │        │ Browser      │
│   writes SavedVariables on   │          │                          │        │              │
│   /reload, /wt sync, logout  │          │  ingest  :1337  PRIVATE ◄┼─ LAN   │              │
│            │                 │  HTTP    │  (POST /ingest, token,   │        │              │
│            ▼                 │ ───────► │   LAN addresses only)    │        │              │
│ wowtracker-companion (Rust)  │          │                          │  GET   │              │
│   watches the file, sends it │          │  public  :8080 READ-ONLY ├───────►│ frontend     │
│   + "game running" heartbeat │          │  (GET /api/state)        │ Caddy/ │ (static site)│
└──────────────────────────────┘          └──────────────────────────┘ Vercel └──────────────┘
```

| Folder | What it is |
|---|---|
| [addon/WowTracker](addon/WowTracker) | The in-game addon (Lua). Collects the data, holds the settings. |
| [crates/companion](crates/companion) | Rust app on the gaming PC (Windows and Linux). Sends the addon's data to the backend. |
| [crates/backend](crates/backend) | Rust server. Private ingest listener plus a separate public, read-only API. |
| [frontend](frontend) | Static website (no build step). Works behind Caddy or on Vercel. |
| [crates/addon-harness](crates/addon-harness) | Tests that run the addon in real Lua 5.1 against a fake WoW API. |
| [docs/setup.md](docs/setup.md) | Step-by-step setup for both hosting styles. |
| [docs/design/live-tracking.md](docs/design/live-tracking.md) | How live the map can get without any ToS risk: options, recommendation, what to measure. |
| [docs/research](docs/research) | Research notes: Forever's addon API, ways to get data out, map maths. |

## How fresh is the data?

WoW addons cannot open network connections or write files. The only thing an addon can
save is its SavedVariables file, and **the game writes it only on `/reload`, logout or
exit**. So the website updates when you:

- type `/wt sync` (saves and reloads the UI, a short loading screen), or `/reload`
- log out or quit

Between saves the addon keeps recording a breadcrumb trail (every 10 seconds while you
move), so each save uploads the whole route since the last one. The companion also tells
the backend whether the game is running, so the site shows "Offline" soon after you close
the game, even after a crash.

**Between saves, the combat log fills in your position while you fight.** With
`/combatlog` and advanced combat logging on, the game writes a combat log that includes
your position on most fight lines. The game writes it in batches. In a busy zone that
was every 40–65 seconds when measured, so the map follows you within about a minute
while you fight. Walking without fighting adds nothing. The companion reads only the
lines about your own character, never other players' names or positions. This is the
same file Warcraft Logs reads, so it carries no Terms-of-Service risk.

A live (every second) version would need a screen-reading technique that is a grey area
under Blizzard's terms. This project deliberately does not do that; see
[the design notes](docs/design/live-tracking.md).

**Forever hides some combat data from addons.** During combat the game gives addons
"secret" values for an NPC's name, type and health. The addon records your target before
the pull and reads the outcome after the fight, so "Last fight" usually has a name, but a
mob that attacks you without being targeted first shows as unknown.

## Settings

In game (`/wt` lists them; `/wt config` opens a settings panel):

| Command | Does |
|---|---|
| `/wt endpoint 192.168.0.117:1337` | Where the companion sends the data. Also accepts a full URL such as `https://ingest.example.com`. |
| `/wt token <secret>` | Token the backend expects (`ingest_token`). `/wt token clear` removes it. |
| `/wt sync` | Save now so the website updates. |
| `/wt trail on\|off`, `/wt clear` | Record / delete the route. |
| `/wt status` | What is being tracked. |

The companion reads these from the SavedVariables file, so a change takes effect at your
next `/wt sync`, `/reload` or logout. The companion's own config file
(`wowtracker-companion.toml`) can set the same things; in-game values win unless
`prefer_config_endpoint = true`. Command-line flags and `WOWTRACKER_*` environment
variables beat both.

The backend's settings live in `wowtracker-backend.toml` (`wowtracker-backend init`
writes a commented example). The website's settings live in
[frontend/config.js](frontend/config.js).

## Security model

- **The ingest listener is private.** It only accepts data from `ingest_allow` (localhost
  and private LAN ranges by default), optionally requires `ingest_token`, and is a
  separate port from the public API. Do not port-forward or proxy it.
- **The public listener cannot change anything.** It has no write route at all
  (`POST` returns 405), so proxying it, even by mistake in full, exposes only what the
  website shows.
- **What becomes public** is exactly what the website shows: character name, realm,
  level, XP, zone, coordinates, route, target and last fight. The token and endpoint
  settings are never sent to the website.

## Build and test

Needs Rust (stable). The companion and backend build on Windows and Linux.

```bash
cargo build --release
```

```bash
cargo test
```

```bash
cargo test -p addon-harness
```

`cargo test -p addon-harness` compiles Lua 5.1 from C, so it needs a C compiler (MSVC on
Windows, gcc/clang on Linux). It runs the addon through login, movement, zone changes,
combat with hidden values, level-ups, slash commands, reloads and logouts, then parses
the file it would save with the companion's parser.

To try everything without the game, generate a demo save file, then point the companion
at a fake game folder; see [docs/setup.md](docs/setup.md#trying-it-without-the-game).

## Not verified in the real game yet

Everything here was tested against a mock of the WoW API, built from Blizzard's published
Forever API documentation. Check these in game:

- The addon loads with `## Interface: 16001`, and `/wt config` opens the settings panel.
- Whether `/reload` is detected as a reload, which keeps you "online". If a reload shows you
  as logged out, use `/wt sync`, which always works.
- Which NPC fields are really hidden out of combat.
- The game's install folder name (`_forever_` is assumed) and process name (`Wow.exe` is
  assumed). Both are configurable in the companion.
