mod config;
mod store;

use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use config::{Cidr, Config};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use store::Store;
use tokio::sync::RwLock;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::services::ServeDir;
use wowtracker_protocol::IngestPayload;

const USAGE: &str = "\
wowtracker-backend: receives data from the WowTracker companion and serves it to the website

USAGE:
    wowtracker-backend [--config <file>]
    wowtracker-backend init [--config <file>]   write an example config file

The config file defaults to wowtracker-backend.toml in the current folder.
Environment overrides: WOWTRACKER_INGEST_BIND, WOWTRACKER_INGEST_ALLOW, WOWTRACKER_INGEST_TOKEN,
WOWTRACKER_PUBLIC_BIND, WOWTRACKER_FRONTEND_DIR, WOWTRACKER_CORS_ORIGINS, WOWTRACKER_DATA_FILE.
";

#[derive(Clone)]
struct App {
    store: Arc<RwLock<Store>>,
    cfg: Arc<Config>,
    allow: Arc<Vec<Cidr>>,
    /// Serializes writes of the state file.
    save_lock: Arc<tokio::sync::Mutex<()>>,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut config_path = PathBuf::from(config::CONFIG_FILE_NAME);
    let mut init = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "init" => init = true,
            "--config" => config_path = PathBuf::from(args.next().ok_or("--config needs a value")?),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("wowtracker-backend {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => return Err(format!("unknown argument '{other}' (see --help)")),
        }
    }

    if init {
        if config_path.exists() {
            return Err(format!("{} already exists", config_path.display()));
        }
        std::fs::write(&config_path, config::EXAMPLE_CONFIG).map_err(|e| e.to_string())?;
        println!("Wrote {}", config_path.display());
        return Ok(());
    }

    let mut cfg = if config_path.exists() {
        let text = std::fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
        println!("config: {}", config_path.display());
        toml::from_str::<Config>(&text).map_err(|e| format!("{}: {e}", config_path.display()))?
    } else {
        println!("no {} found, using defaults (run `wowtracker-backend init` to create one)", config_path.display());
        Config::default()
    };
    cfg.apply_env();
    let allow = cfg.validate()?;

    let store = Store::load(&cfg.data_file);
    println!("state file: {} ({} character(s) loaded)", cfg.data_file.display(), store.characters.len());

    let app = App {
        store: Arc::new(RwLock::new(store)),
        cfg: Arc::new(cfg.clone()),
        allow: Arc::new(allow),
        save_lock: Arc::new(tokio::sync::Mutex::new(())),
    };

    let ingest = Router::new()
        .route("/ingest", post(ingest))
        .route("/health", get(health))
        .with_state(app.clone());

    let mut public = Router::new()
        .route("/api/state", get(state))
        .route("/api/health", get(health))
        .with_state(app.clone());
    if let Some(cors) = cors_layer(&cfg.cors_origins)? {
        public = public.layer(cors);
    }
    if !cfg.frontend_dir.trim().is_empty() {
        let dir = PathBuf::from(cfg.frontend_dir.trim());
        if !dir.join("index.html").exists() {
            return Err(format!("frontend_dir {} has no index.html", dir.display()));
        }
        println!("serving website from {}", dir.display());
        public = public.fallback_service(ServeDir::new(dir));
    }

    let ingest_listener = tokio::net::TcpListener::bind(&cfg.ingest_bind)
        .await
        .map_err(|e| format!("cannot listen on ingest_bind {}: {e}", cfg.ingest_bind))?;
    let public_listener = tokio::net::TcpListener::bind(&cfg.public_bind)
        .await
        .map_err(|e| format!("cannot listen on public_bind {}: {e}", cfg.public_bind))?;

    println!("ingest (private):    http://{}/ingest   allowed from: {}", cfg.ingest_bind, cfg.ingest_allow.join(", "));
    println!("public (read-only):  http://{}/api/state", cfg.public_bind);
    if cfg.ingest_token.is_empty() {
        println!("note: ingest_token is empty, so any allowed address can send data");
    }

    let ingest_server = axum::serve(ingest_listener, ingest.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown());
    let public_server = axum::serve(public_listener, public).with_graceful_shutdown(shutdown());
    tokio::try_join!(async { ingest_server.await }, async { public_server.await }).map_err(|e| e.to_string())?;
    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

fn cors_layer(origins: &[String]) -> Result<Option<CorsLayer>, String> {
    if origins.is_empty() {
        return Ok(None);
    }
    let base = CorsLayer::new().allow_methods([Method::GET]);
    if origins.iter().any(|o| o == "*") {
        return Ok(Some(base.allow_origin(Any)));
    }
    let list = origins
        .iter()
        .map(|o| HeaderValue::from_str(o.trim_end_matches('/')).map_err(|_| format!("bad origin in cors_origins: {o}")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(base.allow_origin(AllowOrigin::list(list))))
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

async fn health() -> &'static str {
    "ok"
}

async fn ingest(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    payload: Result<Json<IngestPayload>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if !app.allow.iter().any(|c| c.contains(peer.ip())) {
        eprintln!("ingest: refused {} (not in ingest_allow)", peer.ip());
        return (StatusCode::FORBIDDEN, "address not allowed").into_response();
    }
    if !app.cfg.ingest_token.is_empty() {
        let given = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if !constant_time_eq(given.as_bytes(), app.cfg.ingest_token.as_bytes()) {
            eprintln!("ingest: bad token from {}", peer.ip());
            return (StatusCode::UNAUTHORIZED, "bad token").into_response();
        }
    }
    let Json(payload) = match payload {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, e.body_text()).into_response(),
    };

    let what = match (payload.characters.len(), payload.live.is_some()) {
        (0, true) => "combat log position".to_string(),
        (n, _) => format!("{n} character(s)"),
    };
    let snapshot = {
        let mut store = app.store.write().await;
        let changed = store.apply(payload, unix_now(), app.cfg.trail_max);
        changed.then(|| store.clone())
    };
    if let Some(snapshot) = snapshot {
        println!("ingest: {what} from {}", peer.ip());
        if let Err(e) = save(&app, &snapshot).await {
            eprintln!("warning: could not save {}: {e}", app.cfg.data_file.display());
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn save(app: &App, store: &Store) -> std::io::Result<()> {
    let _guard = app.save_lock.lock().await;
    let json = serde_json::to_vec_pretty(store)?;
    let path = &app.cfg.data_file;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, json).await?;
    tokio::fs::rename(&tmp, path).await
}

async fn state(State(app): State<App>) -> Response {
    let state = app.store.read().await.public_state(unix_now(), app.cfg.offline_after_seconds);
    ([(header::CACHE_CONTROL, "no-store")], Json(state)).into_response()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
