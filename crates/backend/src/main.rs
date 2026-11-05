mod config;
mod history;
mod store;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use config::{Cidr, Config};
use history::History;
use serde::Deserialize;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use store::Store;
use tokio::sync::{broadcast, RwLock};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::services::ServeDir;
use wowtracker_protocol::{IngestPayload, TrailPoint};

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
    history: Arc<History>,
    /// Pushes (event name, JSON data) to every open /api/stream.
    events: broadcast::Sender<(&'static str, String)>,
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
    let history = History::open(&cfg.history_file)?;
    // Backfill from the state file (a no-op for anything already stored).
    let mut backfilled = 0;
    for c in store.characters.values() {
        backfilled += history.record_character(c)?;
    }
    println!("history: {}{}", cfg.history_file.display(), if backfilled > 0 { format!(" ({backfilled} rows backfilled)") } else { String::new() });

    let app = App {
        store: Arc::new(RwLock::new(store)),
        cfg: Arc::new(cfg.clone()),
        allow: Arc::new(allow),
        save_lock: Arc::new(tokio::sync::Mutex::new(())),
        history: Arc::new(history),
        events: broadcast::channel(64).0,
    };

    let ingest = Router::new()
        .route("/ingest", post(ingest))
        .route("/health", get(health))
        .with_state(app.clone());

    let mut public = Router::new()
        .route("/api/state", get(state))
        .route("/api/history", get(history_route))
        .route("/api/sessions", get(sessions_route))
        .route("/api/sessions/{start}", get(session_route))
        .route("/api/stream", get(stream))
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
    // History gets everything, even a save the current state ignores.
    let chars = payload.characters.clone();
    let (snapshot, live) = {
        let mut store = app.store.write().await;
        let live = payload.live.as_ref().and_then(|l| {
            let key = store.live_target(l)?;
            let mut points = l.trail.clone();
            points.push(TrailPoint { t: l.t, map_id: l.map_id, x: l.x, y: l.y });
            Some((key, points))
        });
        let changed = store.apply(payload, unix_now(), app.cfg.trail_max);
        (changed.then(|| store.clone()), live)
    };
    let history = app.history.clone();
    let recorded = tokio::task::spawn_blocking(move || -> Result<usize, String> {
        let mut n = 0;
        for c in &chars {
            n += history.record_character(c)?;
        }
        if let Some((key, points)) = live {
            n += history.record_positions(&key, &points, "combatLog")?;
        }
        Ok(n)
    })
    .await;
    match recorded {
        Ok(Err(e)) => eprintln!("warning: could not write history: {e}"),
        Err(e) => eprintln!("warning: history task failed: {e}"),
        Ok(Ok(_)) => {}
    }
    if let Some(snapshot) = snapshot {
        println!("ingest: {what} from {}", peer.ip());
        // Tell open pages to fetch the new state.
        let _ = app.events.send(("update", "{}".into()));
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

#[derive(Deserialize)]
struct HistoryQuery {
    /// `Name-Realm`; default: the most recently saved character.
    char: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
    /// Comma-separated journal kinds.
    kinds: Option<String>,
    limit: Option<usize>,
}

const HISTORY_LIMIT_MAX: usize = 50_000;

/// The requested character if it exists, else the most recently saved one.
async fn pick_char(app: &App, wanted: Option<&str>) -> Option<String> {
    let store = app.store.read().await;
    match wanted {
        Some(k) => store.characters.contains_key(k).then(|| k.to_string()),
        None => store.characters.values().max_by_key(|c| c.saved_at.unwrap_or(0)).map(|c| c.key.clone()),
    }
}

fn json_no_store<T: serde::Serialize>(v: T) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(v)).into_response()
}

fn history_error(e: String) -> Response {
    eprintln!("warning: history query failed: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, "history query failed").into_response()
}

async fn history_route(State(app): State<App>, Query(q): Query<HistoryQuery>) -> Response {
    let Some(key) = pick_char(&app, q.char.as_deref()).await else {
        return (StatusCode::NOT_FOUND, "no such character").into_response();
    };
    let kinds: Option<Vec<String>> =
        q.kinds.map(|k| k.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
    let (from, to) = (q.from.unwrap_or(0), q.to.unwrap_or(i64::MAX));
    let limit = q.limit.unwrap_or(5_000).min(HISTORY_LIMIT_MAX);
    let history = app.history.clone();
    match tokio::task::spawn_blocking(move || history.range(&key, from, to, kinds.as_deref(), limit)).await {
        Ok(Ok(data)) => json_no_store(data),
        Ok(Err(e)) => history_error(e),
        Err(e) => history_error(e.to_string()),
    }
}

async fn sessions_route(State(app): State<App>, Query(q): Query<HistoryQuery>) -> Response {
    let Some(key) = pick_char(&app, q.char.as_deref()).await else {
        return (StatusCode::NOT_FOUND, "no such character").into_response();
    };
    let history = app.history.clone();
    match tokio::task::spawn_blocking(move || history.sessions(&key)).await {
        Ok(Ok(list)) => json_no_store(list),
        Ok(Err(e)) => history_error(e),
        Err(e) => history_error(e.to_string()),
    }
}

async fn session_route(State(app): State<App>, Path(start): Path<i64>, Query(q): Query<HistoryQuery>) -> Response {
    let Some(key) = pick_char(&app, q.char.as_deref()).await else {
        return (StatusCode::NOT_FOUND, "no such character").into_response();
    };
    let history = app.history.clone();
    match tokio::task::spawn_blocking(move || history.session(&key, start)).await {
        Ok(Ok(Some(detail))) => json_no_store(detail),
        Ok(Ok(None)) => (StatusCode::NOT_FOUND, "no such session").into_response(),
        Ok(Err(e)) => history_error(e),
        Err(e) => history_error(e.to_string()),
    }
}

/// Server-sent events: `update` when the state changed (fetch /api/state),
/// `live` with a small live frame when one arrives.
async fn stream(State(app): State<App>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = BroadcastStream::new(app.events.subscribe());
    let events = rx.filter_map(|msg| match msg {
        Ok((name, data)) => Some(Ok(Event::default().event(name).data(data))),
        // A slow client missed messages: have it refetch everything.
        Err(_) => Some(Ok(Event::default().event("update").data("{}"))),
    });
    Sse::new(events).keep_alive(KeepAlive::default())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
