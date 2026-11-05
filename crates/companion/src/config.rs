//! Companion settings: config file, command line and environment.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CONFIG_FILE_NAME: &str = "wowtracker-companion.toml";

pub const EXAMPLE_CONFIG: &str = r#"# WowTracker companion settings.
#
# The backend address and token can also be set in-game with
#   /wt endpoint 192.168.0.117:1337
#   /wt token <secret>
# In-game values win over this file unless prefer_config_endpoint = true.

# Folder that contains WTF\ for the game you play, for example
#   C:\Program Files (x86)\World of Warcraft\_forever_
#   /home/me/Games/battlenet/drive_c/Program Files (x86)/World of Warcraft/_forever_
# The "World of Warcraft" folder itself also works. Leave empty to auto-detect.
wow_path = ""

# Backend ingest address: "host:port" on your LAN, or a full URL.
endpoint = ""

# Must match ingest_token in the backend's config. Empty = no token.
token = ""

# Use endpoint/token from this file even when they are set in-game.
prefer_config_endpoint = false

# How often to check the SavedVariables file for changes.
poll_seconds = 2

# How often to tell the backend whether the game is running.
heartbeat_seconds = 30

# Process names that count as "the game is running".
process_names = ["Wow.exe", "WowForever.exe", "WowF.exe", "WowT.exe", "WowB.exe", "WowClassic.exe"]
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub wow_path: Option<PathBuf>,
    pub endpoint: Option<String>,
    pub token: Option<String>,
    pub prefer_config_endpoint: bool,
    pub poll_seconds: u64,
    pub heartbeat_seconds: u64,
    pub process_names: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            wow_path: None,
            endpoint: None,
            token: None,
            prefer_config_endpoint: false,
            poll_seconds: 2,
            heartbeat_seconds: 30,
            process_names: ["Wow.exe", "WowForever.exe", "WowF.exe", "WowT.exe", "WowB.exe", "WowClassic.exe"]
                .into_iter()
                .map(String::from)
                .collect(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut cfg: Config = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        cfg.clean();
        Ok(cfg)
    }

    /// Treats empty strings as "not set".
    pub fn clean(&mut self) {
        if self.wow_path.as_ref().is_some_and(|p| p.as_os_str().is_empty()) {
            self.wow_path = None;
        }
        for v in [&mut self.endpoint, &mut self.token] {
            if v.as_ref().is_some_and(|s| s.trim().is_empty()) {
                *v = None;
            }
        }
        self.poll_seconds = self.poll_seconds.max(1);
        self.heartbeat_seconds = self.heartbeat_seconds.max(5);
    }
}

/// `--config`, else `wowtracker-companion.toml` in the current folder, else next to the program.
pub fn default_config_path() -> PathBuf {
    let local = PathBuf::from(CONFIG_FILE_NAME);
    if local.exists() {
        return local;
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(CONFIG_FILE_NAME)))
        .unwrap_or(local)
}

#[derive(Debug, Default, Clone)]
pub struct Args {
    pub config: Option<PathBuf>,
    pub wow_path: Option<PathBuf>,
    pub endpoint: Option<String>,
    pub token: Option<String>,
    pub once: bool,
    pub dry_run: bool,
    pub init: bool,
    pub probe: bool,
    pub show_lines: bool,
    pub log: Option<PathBuf>,
    pub help: bool,
    pub version: bool,
}

pub const USAGE: &str = "\
wowtracker-companion: sends your WowTracker addon data to your backend

USAGE:
    wowtracker-companion [OPTIONS]
    wowtracker-companion init        write an example config file
    wowtracker-companion probe       show which game files are written while you play, and
                                     how far behind the combat log is (--show-lines: every line;
                                     --log <file>: copy of the output, default wowtracker-probe.log)

OPTIONS:
    --config <file>      config file (default: wowtracker-companion.toml here or next to the program)
    --wow-path <folder>  game folder that contains WTF\\ (overrides config)
    --endpoint <addr>    backend address, e.g. 192.168.0.117:1337 (overrides config and in-game)
    --token <secret>     ingest token (overrides config and in-game)
    --once               send the current data once and exit
    --dry-run            print what would be sent and exit
    -h, --help           show this help
    -V, --version        show the version

ENVIRONMENT:
    WOWTRACKER_ENDPOINT, WOWTRACKER_TOKEN, WOWTRACKER_WOW_PATH work like the options above.
";

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "init" => out.init = true,
            "probe" => out.probe = true,
            "--show-lines" => out.show_lines = true,
            "--log" => out.log = Some(PathBuf::from(value("--log")?)),
            "--config" => out.config = Some(PathBuf::from(value("--config")?)),
            "--wow-path" => out.wow_path = Some(PathBuf::from(value("--wow-path")?)),
            "--endpoint" => out.endpoint = Some(value("--endpoint")?),
            "--token" => out.token = Some(value("--token")?),
            "--once" => out.once = true,
            "--dry-run" => out.dry_run = true,
            "-h" | "--help" => out.help = true,
            "-V" | "--version" => out.version = true,
            other => return Err(format!("unknown argument '{other}' (see --help)")),
        }
    }
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    out.endpoint = out.endpoint.or_else(|| env("WOWTRACKER_ENDPOINT"));
    out.token = out.token.or_else(|| env("WOWTRACKER_TOKEN"));
    out.wow_path = out.wow_path.or_else(|| env("WOWTRACKER_WOW_PATH").map(PathBuf::from));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        let mut cfg: Config = toml::from_str(EXAMPLE_CONFIG).unwrap();
        cfg.clean();
        assert!(cfg.wow_path.is_none());
        assert!(cfg.endpoint.is_none());
        assert_eq!(cfg.poll_seconds, 2);
        assert!(cfg.process_names.contains(&"Wow.exe".to_string()));
    }

    #[test]
    fn parses_args() {
        let a = parse_args(["--endpoint", "10.0.0.2:1337", "--once"].map(String::from)).unwrap();
        assert_eq!(a.endpoint.as_deref(), Some("10.0.0.2:1337"));
        assert!(a.once);
        assert!(parse_args(["--bogus".to_string()]).is_err());
    }
}
