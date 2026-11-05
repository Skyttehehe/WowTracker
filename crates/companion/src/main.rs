use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use wowtracker_companion::config::{self, Args, Config};
use wowtracker_companion::savedvars::{read_addon_data, AddonData};
use wowtracker_companion::{discover, process, upload};
use wowtracker_protocol::IngestPayload;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args = match config::parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        print!("{}", config::USAGE);
        return ExitCode::SUCCESS;
    }
    if args.version {
        println!("wowtracker-companion {VERSION}");
        return ExitCode::SUCCESS;
    }
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log(&format!("error: {e}"));
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    let config_path = args.config.clone().unwrap_or_else(config::default_config_path);

    if args.init {
        if config_path.exists() {
            return Err(format!("{} already exists", config_path.display()));
        }
        std::fs::write(&config_path, config::EXAMPLE_CONFIG)
            .map_err(|e| format!("cannot write {}: {e}", config_path.display()))?;
        println!("Wrote {}", config_path.display());
        return Ok(());
    }

    let mut cfg = if config_path.exists() {
        log(&format!("config: {}", config_path.display()));
        Config::load(&config_path)?
    } else {
        log(&format!("no config file ({}), using defaults. Run `wowtracker-companion init` to create one.", config_path.display()));
        Config::default()
    };
    if let Some(p) = args.wow_path.clone() {
        cfg.wow_path = Some(p);
    }

    let game_dir = match &cfg.wow_path {
        Some(p) => discover::resolve_game_dir(p)
            .ok_or_else(|| format!("no WTF folder found in {} (set wow_path to the folder that contains WTF)", p.display()))?,
        None => discover::auto_detect_game_dir()
            .ok_or("could not find World of Warcraft; set wow_path in the config or pass --wow-path")?,
    };
    log(&format!("game folder: {}", game_dir.display()));

    let mut companion = Companion::new(cfg, args, game_dir);
    companion.run()
}

struct Target {
    url: String,
    token: Option<String>,
}

struct FileState {
    sent_mtime: Option<SystemTime>,
    retry_at: Option<Instant>,
    failures: u32,
}

struct Companion {
    cfg: Config,
    args: Args,
    game_dir: PathBuf,
    files: HashMap<PathBuf, FileState>,
    /// Where heartbeats go: every backend we have sent data to.
    targets: Vec<Target>,
    last_heartbeat: Option<Instant>,
    last_game_running: Option<bool>,
    warned_no_files: bool,
}

impl Companion {
    fn new(cfg: Config, args: Args, game_dir: PathBuf) -> Self {
        Companion {
            cfg,
            args,
            game_dir,
            files: HashMap::new(),
            targets: Vec::new(),
            last_heartbeat: None,
            last_game_running: None,
            warned_no_files: false,
        }
    }

    fn run(&mut self) -> Result<(), String> {
        if !(self.args.once || self.args.dry_run) {
            log("watching for changes (Ctrl+C to stop). In game: /wt sync or /reload to send an update.");
        }
        loop {
            let game_running = process::game_running(&self.cfg.process_names);
            let sent_any = self.check_files(game_running)?;

            if self.args.once || self.args.dry_run {
                if !sent_any && !self.args.dry_run {
                    return Err("nothing was sent".into());
                }
                return Ok(());
            }

            let heartbeat_due = self
                .last_heartbeat
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(self.cfg.heartbeat_seconds));
            if heartbeat_due || game_running != self.last_game_running {
                if game_running != self.last_game_running {
                    if let Some(r) = game_running {
                        log(if r { "game is running" } else { "game is not running" });
                    }
                }
                self.heartbeat(game_running);
                self.last_game_running = game_running;
            }

            std::thread::sleep(Duration::from_secs(self.cfg.poll_seconds));
        }
    }

    /// Sends any SavedVariables file that changed. Returns whether something was sent.
    fn check_files(&mut self, game_running: Option<bool>) -> Result<bool, String> {
        let files = discover::saved_variable_files(&self.game_dir);
        if files.is_empty() {
            if !self.warned_no_files {
                log(&format!(
                    "no WowTracker.lua under {}\\WTF\\Account\\*\\SavedVariables yet. \
                     Install the addon, log in, then /reload or log out.",
                    self.game_dir.display()
                ));
                self.warned_no_files = true;
            }
            return Ok(false);
        }

        let mut sent_any = false;
        for file in files {
            let Ok(mtime) = std::fs::metadata(&file).and_then(|m| m.modified()) else { continue };
            let state = self.files.entry(file.clone()).or_insert(FileState { sent_mtime: None, retry_at: None, failures: 0 });
            if state.sent_mtime == Some(mtime) {
                continue;
            }
            if state.retry_at.is_some_and(|t| Instant::now() < t) {
                continue;
            }
            match self.send_file(&file, game_running) {
                Ok(()) => {
                    let state = self.files.get_mut(&file).unwrap();
                    state.sent_mtime = Some(mtime);
                    state.retry_at = None;
                    state.failures = 0;
                    sent_any = true;
                }
                Err(e) => {
                    let state = self.files.get_mut(&file).unwrap();
                    state.failures += 1;
                    let wait = Duration::from_secs(2u64.pow(state.failures.min(5))).min(Duration::from_secs(60));
                    state.retry_at = Some(Instant::now() + wait);
                    if self.args.once {
                        return Err(format!("{}: {e}", short(&file)));
                    }
                    log(&format!("{}: {e} (retrying in {}s)", short(&file), wait.as_secs()));
                }
            }
        }
        Ok(sent_any)
    }

    fn send_file(&mut self, file: &Path, game_running: Option<bool>) -> Result<(), String> {
        let text = std::fs::read(file).map_err(|e| format!("cannot read: {e}"))?;
        let text = String::from_utf8_lossy(&text);
        let data = read_addon_data(&text).map_err(|e| format!("cannot parse (the game may still be writing it): {e}"))?;
        for w in &data.warnings {
            log(&format!("{}: skipped character {w}", short(file)));
        }

        let (endpoint, token, source) = self.resolve_target(&data)?;
        let url = upload::ingest_url(&endpoint)?;
        let payload = IngestPayload {
            companion_version: VERSION.to_string(),
            sent_at: unix_now(),
            game_running,
            characters: data.characters.clone(),
        };

        if self.args.dry_run {
            println!("Would send to {url} ({source}):");
            println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
            return Ok(());
        }

        upload::send(&url, token.as_deref(), &payload)?;
        let summary: Vec<String> = data
            .characters
            .iter()
            .map(|c| {
                format!(
                    "{} lvl {} in {}",
                    c.key,
                    c.level.map(|l| l.to_string()).unwrap_or("?".into()),
                    c.zone.as_deref().unwrap_or("?")
                )
            })
            .collect();
        log(&format!("sent {} character(s) to {url}: {}", data.characters.len(), summary.join(", ")));

        if !self.targets.iter().any(|t| t.url == url) {
            self.targets.push(Target { url, token });
        } else if let Some(t) = self.targets.iter_mut().find(|t| t.url == url) {
            t.token = token;
        }
        Ok(())
    }

    /// Precedence: command line / environment, then in-game (`/wt endpoint`),
    /// then the config file. `prefer_config_endpoint` swaps the last two.
    fn resolve_target(&self, data: &AddonData) -> Result<(String, Option<String>, &'static str), String> {
        let (first, second) = if self.cfg.prefer_config_endpoint {
            ((self.cfg.endpoint.clone(), self.cfg.token.clone(), "config file"), (data.endpoint.clone(), data.token.clone(), "in-game setting"))
        } else {
            ((data.endpoint.clone(), data.token.clone(), "in-game setting"), (self.cfg.endpoint.clone(), self.cfg.token.clone(), "config file"))
        };
        let token_fallback = || self.args.token.clone().or(first.1.clone()).or(second.1.clone());

        if let Some(e) = &self.args.endpoint {
            return Ok((e.clone(), token_fallback(), "command line"));
        }
        if let Some(e) = first.0 {
            return Ok((e, self.args.token.clone().or(first.1).or(second.1), first.2));
        }
        if let Some(e) = second.0 {
            return Ok((e, self.args.token.clone().or(second.1).or(first.1), second.2));
        }
        Err("no backend address. Set it in game with /wt endpoint <ip:port>, or endpoint in the config file".into())
    }

    fn heartbeat(&mut self, game_running: Option<bool>) {
        self.last_heartbeat = Some(Instant::now());
        if self.targets.is_empty() {
            // Nothing sent yet; fall back to an explicitly configured address.
            if let Some(e) = self.args.endpoint.clone().or(self.cfg.endpoint.clone()) {
                if let Ok(url) = upload::ingest_url(&e) {
                    self.targets.push(Target { url, token: self.args.token.clone().or(self.cfg.token.clone()) });
                }
            }
        }
        let payload = IngestPayload {
            companion_version: VERSION.to_string(),
            sent_at: unix_now(),
            game_running,
            characters: Vec::new(),
        };
        let mut failed = BTreeSet::new();
        for t in &self.targets {
            if let Err(e) = upload::send(&t.url, t.token.as_deref(), &payload) {
                failed.insert(format!("heartbeat: {e}"));
            }
        }
        for f in failed {
            log(&f);
        }
    }
}

fn short(p: &Path) -> String {
    let parts: Vec<_> = p.components().rev().take(4).collect();
    parts.into_iter().rev().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn log(msg: &str) {
    let s = unix_now().rem_euclid(86_400);
    println!("[{:02}:{:02}:{:02} UTC] {msg}", s / 3600, (s / 60) % 60, s % 60);
}
