use serde::Deserialize;
use std::net::IpAddr;
use std::path::PathBuf;

pub const CONFIG_FILE_NAME: &str = "wowtracker-backend.toml";

pub const EXAMPLE_CONFIG: &str = r#"# WowTracker backend settings.
#
# Data flow:  addon -> companion -> [ingest listener] backend [public listener] -> website
# Only the public listener should ever be reachable from the internet (through Caddy).

# PRIVATE. The companion sends data here. Keep it on your LAN: do not port-forward
# it and do not proxy it. Set the same address in game with /wt endpoint <ip:port>.
ingest_bind = "0.0.0.0:1337"

# Who may send to the ingest listener. Default: localhost and private LAN ranges.
# To accept from anywhere (e.g. a backend on a public server), use ["0.0.0.0/0", "::/0"].
ingest_allow = ["127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "::1/128", "fc00::/7"]

# Shared secret the companion must send (set it in game with /wt token, or in the
# companion config). Empty = no token. Strongly recommended if ingest is not LAN-only.
ingest_token = ""

# PUBLIC, read-only. Serves GET /api/state (and the website, if frontend_dir is set).
# Point Caddy (or Vercel rewrites) at this.
public_bind = "127.0.0.1:8080"

# Serve the static website from this folder on the public listener. Empty = API only.
frontend_dir = ""

# Browser origins allowed to call the API directly (only needed when the website is on
# another domain and does not proxy /api). Example: ["https://wowtracker.osrs.dk"], or ["*"].
cors_origins = []

# Where the last known state is kept between restarts.
data_file = "wowtracker-state.json"

# If the companion has not been heard from for this long, the player counts as offline.
offline_after_seconds = 120

# Maximum trail points kept per character.
trail_max = 2000
"#;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ingest_bind: String,
    pub ingest_allow: Vec<String>,
    pub ingest_token: String,
    pub public_bind: String,
    pub frontend_dir: String,
    pub cors_origins: Vec<String>,
    pub data_file: PathBuf,
    pub offline_after_seconds: i64,
    pub trail_max: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            ingest_bind: "0.0.0.0:1337".into(),
            ingest_allow: ["127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "::1/128", "fc00::/7"]
                .into_iter()
                .map(String::from)
                .collect(),
            ingest_token: String::new(),
            public_bind: "127.0.0.1:8080".into(),
            frontend_dir: String::new(),
            cors_origins: Vec::new(),
            data_file: PathBuf::from("wowtracker-state.json"),
            offline_after_seconds: 120,
            trail_max: 2000,
        }
    }
}

impl Config {
    pub fn apply_env(&mut self) {
        let env = |k: &str| std::env::var(k).ok();
        if let Some(v) = env("WOWTRACKER_INGEST_BIND") {
            self.ingest_bind = v;
        }
        if let Some(v) = env("WOWTRACKER_INGEST_ALLOW") {
            self.ingest_allow = split_list(&v);
        }
        if let Some(v) = env("WOWTRACKER_INGEST_TOKEN") {
            self.ingest_token = v;
        }
        if let Some(v) = env("WOWTRACKER_PUBLIC_BIND") {
            self.public_bind = v;
        }
        if let Some(v) = env("WOWTRACKER_FRONTEND_DIR") {
            self.frontend_dir = v;
        }
        if let Some(v) = env("WOWTRACKER_CORS_ORIGINS") {
            self.cors_origins = split_list(&v);
        }
        if let Some(v) = env("WOWTRACKER_DATA_FILE") {
            self.data_file = PathBuf::from(v);
        }
    }

    pub fn validate(&self) -> Result<Vec<Cidr>, String> {
        if self.ingest_bind.trim() == self.public_bind.trim() {
            return Err("ingest_bind and public_bind must be different: the public side must never accept data".into());
        }
        self.ingest_allow.iter().map(|s| Cidr::parse(s)).collect()
    }
}

fn split_list(v: &str) -> Vec<String> {
    v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// An IP network such as `192.168.0.0/16`. A bare address means a single host.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cidr {
    addr: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn parse(s: &str) -> Result<Cidr, String> {
        let s = s.trim();
        let (ip, prefix) = match s.split_once('/') {
            Some((ip, p)) => (ip, Some(p)),
            None => (s, None),
        };
        let addr: IpAddr = ip.parse().map_err(|_| format!("bad address in ingest_allow: '{s}'"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p.parse::<u8>().ok().filter(|p| *p <= max).ok_or_else(|| format!("bad prefix in ingest_allow: '{s}'"))?,
            None => max,
        };
        Ok(Cidr { addr, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = if self.prefix == 0 { 0 } else { u32::MAX << (32 - self.prefix) };
                (u32::from(net) & mask) == (u32::from(ip) & mask)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = if self.prefix == 0 { 0 } else { u128::MAX << (128 - self.prefix) };
                (u128::from(net) & mask) == (u128::from(ip) & mask)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        let cfg: Config = toml::from_str(EXAMPLE_CONFIG).unwrap();
        assert_eq!(cfg.ingest_bind, "0.0.0.0:1337");
        assert_eq!(cfg.validate().unwrap().len(), 6);
    }

    #[test]
    fn cidr_matching() {
        let lan = Cidr::parse("192.168.0.0/16").unwrap();
        assert!(lan.contains("192.168.0.117".parse().unwrap()));
        assert!(lan.contains("::ffff:192.168.1.5".parse().unwrap()));
        assert!(!lan.contains("8.8.8.8".parse().unwrap()));
        assert!(Cidr::parse("0.0.0.0/0").unwrap().contains("8.8.8.8".parse().unwrap()));
        assert!(Cidr::parse("10.0.0.5").unwrap().contains("10.0.0.5".parse().unwrap()));
        assert!(Cidr::parse("fc00::/7").unwrap().contains("fd12::1".parse().unwrap()));
        assert!(Cidr::parse("1.2.3.4/33").is_err());
    }

    #[test]
    fn same_bind_is_rejected() {
        let cfg = Config { public_bind: "0.0.0.0:1337".into(), ..Config::default() };
        assert!(cfg.validate().is_err());
    }
}
