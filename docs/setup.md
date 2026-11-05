# Setup

Three pieces, in this order: backend, addon, companion. Then the website, in one of two
ways: [self-hosted with Caddy](#4a-website-self-hosted-with-caddy) or
[on Vercel](#4b-website-on-vercel).

Example addresses below:
- `192.168.0.117` is the machine (or container) running the backend.
- `wowtracker.osrs.dk` is the website.

## 1. Backend

Build it (or copy a release binary) on the server:

```bash
cargo build --release -p wowtracker-backend
```

Create the config next to it and edit it:

```bash
./wowtracker-backend init
```

The settings that matter:

```toml
ingest_bind  = "0.0.0.0:1337"     # PRIVATE: the companion sends here
ingest_allow = ["127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "::1/128", "fc00::/7"]
ingest_token = "pick-a-long-random-string"
public_bind  = "0.0.0.0:8080"     # READ-ONLY: Caddy reads from here
```

- If Caddy runs on the same machine, use `public_bind = "127.0.0.1:8080"`.
- Run it with `./wowtracker-backend`, or as a service with
  [deploy/wowtracker-backend.service](../deploy/wowtracker-backend.service).
- Make sure your router does **not** forward port 1337.
- Check it works: `curl http://192.168.0.117:8080/api/state`.

## 2. Addon

1. Copy `addon/WowTracker` into the game's AddOns folder, e.g.
   `C:\Program Files (x86)\World of Warcraft\_forever_\Interface\AddOns\WowTracker`.
2. Log in and set the backend address and token:
   ```
   /wt endpoint 192.168.0.117:1337
   /wt token pick-a-long-random-string
   /wt sync
   ```
   `/wt config` opens the same settings in the game's Options → AddOns panel.

## 3. Companion (on the gaming PC)

Build it:

```bash
cargo build --release -p wowtracker-companion
```

Run it from the folder you want it in (`target/release/wowtracker-companion.exe` on
Windows). It looks for the game in the usual places, picks the game folder that has the
addon installed (Forever first), and reads the endpoint and token you set in game.

If it can't find the game, create a config file and set `wow_path`:

```bash
wowtracker-companion init
```

```toml
wow_path = 'C:\Program Files (x86)\World of Warcraft\_forever_'
```

On Linux, `wow_path` points inside the Wine/Lutris prefix, e.g.
`/home/me/Games/battlenet/drive_c/Program Files (x86)/World of Warcraft/_forever_`.
The companion checks common Lutris, Bottles, Wine and Steam/Proton locations on its own.

Useful flags:

| Flag | Does |
|---|---|
| `--dry-run` | Show what would be sent, and to where, without sending. |
| `--once` | Send once and exit (good for checking the token). |
| `--endpoint`, `--token`, `--wow-path` | Override the config and in-game settings. |

**Start it with Windows.** Press Win+R, type `shell:startup`, and put a shortcut to
`wowtracker-companion.exe` there.

**Start it with Linux.** Use the user service in
[deploy/wowtracker-companion.service](../deploy/wowtracker-companion.service).

**If the site always says Offline while you play,** the companion did not recognise the
game's process. Find the process name in Task Manager (or `ps`) and add it to
`process_names` in the companion config.

## 4a. Website, self-hosted with Caddy

Copy the `frontend` folder to the Caddy machine (e.g. `/srv/wowtracker/frontend`), then
use [deploy/Caddyfile.selfhosted](../deploy/Caddyfile.selfhosted):

```
wowtracker.osrs.dk {
	handle /api/* {
		reverse_proxy 192.168.0.117:8080
	}
	handle {
		root * /srv/wowtracker/frontend
		file_server
	}
}
```

Leave `apiBase: "api"` in `frontend/config.js`.

## 4b. Website on Vercel

Vercel hosts only the static site. The browser still needs to read `/api/state` from your
backend, so give the backend's **read-only** listener a public hostname. Caddy example:
[deploy/Caddyfile.api-for-vercel](../deploy/Caddyfile.api-for-vercel). It only lets
`GET /api/*` through.

1. Put that hostname in [frontend/vercel.json](../frontend/vercel.json):
   ```json
   "destination": "https://wowtracker-api.osrs.dk/api/:path*"
   ```
2. In Vercel, import the repository and set **Root Directory** to `frontend`. There is no
   build command or framework.
3. Leave `apiBase: "api"`. Vercel's rewrite forwards `/api/*` to your backend, so no CORS
   settings are needed.

To skip the rewrite and call the API host directly instead:
- set `apiBase: "https://wowtracker-api.osrs.dk/api"` in `config.js`;
- add `cors_origins = ["https://wowtracker.osrs.dk"]` to the backend config.

## Measuring what the game writes live (probe)

`wowtracker-companion probe` watches the game's `Logs`, `WTF`, `Screenshots` and `Errors`
folders while you play. It prints every file the game writes, and for the combat log how
many seconds old the lines are when they reach the disk. It only reads files; it never
touches the game.

1. Start it:
   ```bash
   wowtracker-companion probe
   ```
2. In game, turn on advanced combat logging, then start logging:
   ```
   /console advancedCombatLogging 1
   /combatlog
   ```
3. Play for 5–10 minutes. Walk, mount up, fight a few mobs, eat or drink, change zone,
   and stand still for a bit.

4. Try the developer log, the only known way an addon might write its own text to a file
   while you play. With the probe still running, type these one at a time, then watch
   for a new or growing file under `Logs`:
   ```
   /console developerLog 1
   /console developerLogWriteToFile 1
   /run C_Log.LogMessage("wt-probe "..date("%H:%M:%S"))
   ```
   Run the last line a few times, a few seconds apart.

The probe prints a summary every minute. The numbers that matter are the combat log's
"newest event … s old" values, which show how far behind the file is, and how many lines
carry a position. `--show-lines` prints every line instead of the first of each type.
Type `/combatlog` again to stop logging; the log files can get large. Leave combat
logging on or off as it is during fights; switching it to force a write would be working
around Blizzard's deliberate delay.

The full list of things worth measuring is in
[research/near-live-options.md](research/near-live-options.md#8-what-to-measure-in-game).

## Map images (optional)

Without images the site draws a schematic map: zone names at their real positions, the
current zone's outline, and an in-game style coordinate grid in zone view. To show the
real map art, see [frontend/maps/README.md](../frontend/maps/README.md).

## Trying it without the game

1. Generate a demo save file from the real addon code. It writes a play session around
   Elwynn and Westfall:

   ```bash
   mkdir -p "demo/World of Warcraft/_forever_/WTF/Account/DEMO/SavedVariables" "demo/World of Warcraft/_forever_/Interface/AddOns/WowTracker"
   ```

   ```bash
   WOWTRACKER_DEMO_OUT="demo/World of Warcraft/_forever_/WTF/Account/DEMO/SavedVariables/WowTracker.lua" WOWTRACKER_DEMO_ENDPOINT=127.0.0.1:1337 cargo test -p addon-harness -- --ignored demo
   ```

2. Start the backend with `frontend_dir = "frontend"` and `public_bind = "127.0.0.1:8080"`:

   ```bash
   cargo run -p wowtracker-backend
   ```

3. Send the demo data:

   ```bash
   cargo run -p wowtracker-companion -- --wow-path "demo/World of Warcraft" --once
   ```

4. Open http://127.0.0.1:8080.
