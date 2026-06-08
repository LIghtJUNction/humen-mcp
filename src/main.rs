use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    net::SocketAddr,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use clap::{Args, Parser, Subcommand};
use dashmap::DashMap;
use futures_util::StreamExt;
use rand::{distr::Alphanumeric, Rng};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, oneshot};
use tower_http::{
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    config: Config,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize or reset the admin account in the env file.
    InitAdmin(InitAdminArgs),
}

#[derive(Debug, Args)]
struct InitAdminArgs {
    #[arg(long, default_value = "/etc/humen-mcp.env")]
    env_file: PathBuf,

    #[arg(long)]
    email: Option<String>,

    #[arg(long = "password")]
    admin_pass: Option<String>,
}

#[derive(Debug, Clone, Args)]
struct Config {
    #[arg(long, env = "HUMEN_BIND", default_value = "127.0.0.1:8787")]
    bind: SocketAddr,

    #[arg(
        long,
        env = "HUMEN_PUBLIC_BASE_URL",
        default_value = "http://127.0.0.1:8787"
    )]
    public_base_url: String,

    #[arg(long, env = "HUMEN_WEB_DIST", default_value = "./humen-mcp-webui/dist")]
    web_dist: String,

    #[arg(
        long,
        env = "HUMEN_USERS_FILE",
        default_value = "./humen-mcp-users.json"
    )]
    users_file: PathBuf,

    #[arg(long, env = "HUMEN_ADMIN_EMAIL", default_value = "<admin-email>")]
    admin_email: String,

    #[arg(long, env = "HUMEN_ADMIN_PASSWORD", default_value = "change-me")]
    admin_password: String,

    #[arg(
        long,
        env = "HUMEN_SESSION_SECRET",
        default_value = "dev-secret-change-me"
    )]
    session_secret: String,

    #[arg(long, env = "HUMEN_GITHUB_CLIENT_ID")]
    github_client_id: Option<String>,

    #[arg(long, env = "HUMEN_GITHUB_CLIENT_SECRET")]
    github_client_secret: Option<String>,

    #[arg(
        long,
        env = "HUMEN_TRASH_RETENTION_SECONDS",
        default_value_t = 7 * 24 * 60 * 60
    )]
    trash_retention_seconds: u64,

    #[arg(long, env = "HUMEN_CLEANUP_INTERVAL_SECONDS", default_value_t = 60)]
    cleanup_interval_seconds: u64,
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    requests: Arc<DashMap<Uuid, HumanRequest>>,
    waiters: Arc<DashMap<Uuid, oneshot::Sender<HumanAnswer>>>,
    trash: Arc<DashMap<Uuid, ExpiredRequest>>,
    sessions: Arc<DashMap<String, Session>>,
    users: Arc<Mutex<UserStore>>,
    admin_settings: Arc<Mutex<AdminSettings>>,
    online_humans: Arc<AtomicUsize>,
    events: broadcast::Sender<ServerEvent>,
    http: Client,
}

impl AppState {
    fn new(config: Config) -> anyhow::Result<Self> {
        let (events, _) = broadcast::channel(128);
        let users = UserStore::load(&config.users_file)?;
        Ok(Self {
            config: Arc::new(config),
            requests: Arc::new(DashMap::new()),
            waiters: Arc::new(DashMap::new()),
            trash: Arc::new(DashMap::new()),
            sessions: Arc::new(DashMap::new()),
            users: Arc::new(Mutex::new(users)),
            admin_settings: Arc::new(Mutex::new(AdminSettings::default())),
            online_humans: Arc::new(AtomicUsize::new(0)),
            events,
            http: Client::new(),
        })
    }

    fn create_session(&self, email: impl Into<String>, provider: AuthProvider) -> AuthResponse {
        let raw_token: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(48)
            .map(char::from)
            .collect();
        let token_hash = self.hash_token(&raw_token);
        let user = User {
            email: email.into(),
            provider,
        };
        self.sessions.insert(
            token_hash,
            Session {
                user: user.clone(),
                created_at: now_unix(),
            },
        );
        AuthResponse {
            token: raw_token,
            user,
        }
    }

    fn session_from_headers(&self, headers: &HeaderMap) -> Option<Session> {
        let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
        let token = value.strip_prefix("Bearer ")?;
        self.session_from_token(token)
    }

    fn session_from_token(&self, token: &str) -> Option<Session> {
        self.sessions
            .get(&self.hash_token(token))
            .map(|s| s.clone())
    }

    fn hash_token(&self, token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.config.session_secret.as_bytes());
        hasher.update(b":");
        hasher.update(token.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn github_enabled(&self) -> bool {
        non_empty(self.config.github_client_id.as_deref())
            && non_empty(self.config.github_client_secret.as_deref())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HumanRequest {
    id: Uuid,
    kind: TaskKind,
    title: String,
    prompt: String,
    choices: Vec<String>,
    image_url: Option<String>,
    steps: Vec<String>,
    created_at: u64,
    timeout_seconds: u64,
    expires_at: u64,
    tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TaskKind {
    Choice,
    Text,
    ImageReview,
    Steps,
}

impl Default for TaskKind {
    fn default() -> Self {
        Self::Text
    }
}

#[derive(Clone, Debug, Deserialize)]
struct CreateHumanRequest {
    #[serde(default)]
    kind: TaskKind,
    title: String,
    prompt: String,
    #[serde(default)]
    choices: Vec<String>,
    image_url: Option<String>,
    #[serde(default)]
    steps: Vec<String>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HumanAnswer {
    answer: String,
    note: Option<String>,
    answered_by: String,
    answered_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ExpiredRequest {
    request: HumanRequest,
    expired_at: u64,
    reason: String,
}

#[derive(Clone, Debug, Serialize)]
struct PublicUserProfile {
    email: String,
    provider: AuthProvider,
    profile: String,
    tags: Vec<String>,
    online: bool,
    last_login_at: u64,
    ban_expires_at: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
struct AnswerRequest {
    answer: String,
    note: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerEvent {
    RequestCreated {
        request: HumanRequest,
    },
    RequestAnswered {
        id: Uuid,
        answer: HumanAnswer,
    },
    RequestExpired {
        id: Uuid,
        expired_request: ExpiredRequest,
    },
    TrashCleaned {
        removed_count: usize,
    },
    PresenceChanged {
        online_count: usize,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct User {
    email: String,
    provider: AuthProvider,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AuthProvider {
    Password,
    Github,
}

#[derive(Clone, Debug)]
struct Session {
    user: User,
    created_at: u64,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    #[serde(alias = "password")]
    pass: String,
}

#[derive(Debug, Serialize)]
struct AuthResponse {
    token: String,
    user: User,
}

#[derive(Debug, Serialize)]
struct AuthConfigResponse {
    github_enabled: bool,
    allow_registration: bool,
    oauth_channels: Vec<OAuthChannel>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AdminSettings {
    allow_registration: bool,
    oauth_channels: Vec<OAuthChannel>,
}

impl Default for AdminSettings {
    fn default() -> Self {
        Self {
            allow_registration: true,
            oauth_channels: vec![OAuthChannel {
                provider: "github".to_string(),
                enabled: false,
                client_id: String::new(),
            }],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OAuthChannel {
    provider: String,
    enabled: bool,
    client_id: String,
}

#[derive(Debug, Deserialize)]
struct AdminUserRequest {
    email: String,
    #[serde(default)]
    profile: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AdminUserUpdate {
    profile: Option<String>,
    tags: Option<Vec<String>>,
    ban_expires_at: Option<Option<u64>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UserRecord {
    email: String,
    #[serde(default)]
    created_at: u64,
    #[serde(default)]
    last_login_at: u64,
    #[serde(default)]
    profile: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    ban_expires_at: Option<u64>,
    #[serde(default)]
    active_periods: Vec<ActivePeriod>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ActivePeriod {
    #[serde(default)]
    user_id: String,
    connected_at: u64,
    disconnected_at: Option<u64>,
    duration_seconds: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct UserStore {
    #[serde(default)]
    users: HashMap<String, UserRecord>,
}

impl UserStore {
    fn load(path: &PathBuf) -> anyhow::Result<Self> {
        match fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw).context("parse users file"),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).context("read users file"),
        }
    }

    fn save(&self, path: &PathBuf) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create users file directory")?;
        }
        let raw = serde_json::to_string_pretty(self).context("serialize users file")?;
        fs::write(path, raw).context("write users file")
    }

    fn insert(&mut self, record: UserRecord) {
        self.users.insert(normalize_email(&record.email), record);
    }
}

#[derive(Debug, Deserialize)]
struct McpRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
struct OAuthCallback {
    code: String,
}

#[derive(Debug, Deserialize)]
struct WsQuery {
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: Option<String>,
    tag: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    if let Some(Command::InitAdmin(args)) = cli.command {
        init_admin(args)?;
        return Ok(());
    }

    let config = cli.config;
    let bind = config.bind;
    let web_dist = config.web_dist.clone();
    let state = AppState::new(config)?;
    tokio::spawn(trash_cleanup_loop(state.clone()));

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/mcp", get(mcp_get).post(mcp))
        .route("/api/auth/config", get(auth_config))
        .route("/api/auth/login", post(login))
        .route("/api/auth/oauth/github/start", get(github_oauth_start))
        .route(
            "/api/auth/oauth/github/callback",
            get(github_oauth_callback),
        )
        .route("/api/me", get(me))
        .route("/api/requests", get(list_requests))
        .route("/api/requests/{id}/answer", post(answer_request))
        .route("/api/trash", get(list_trash))
        .route("/api/trash/clear", post(clear_trash))
        .route("/api/users/online", get(list_online_users))
        .route("/api/users/search", get(search_users))
        .route("/api/tags", get(list_tags))
        .route(
            "/api/admin/users",
            get(admin_list_users).post(admin_add_user),
        )
        .route("/api/admin/users/{email}", post(admin_update_user))
        .route("/api/admin/users/{email}/kick", post(admin_kick_user))
        .route(
            "/api/admin/settings",
            get(admin_settings).post(admin_update_settings),
        )
        .route("/api/ws", get(ws_handler))
        .fallback_service(
            ServeDir::new(&web_dist).fallback(ServeFile::new(format!("{web_dist}/index.html"))),
        )
        .with_state(state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http());

    info!("listening on http://{bind}");
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .context("bind server socket")?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve humen-mcp")?;

    Ok(())
}

fn init_admin(args: InitAdminArgs) -> anyhow::Result<()> {
    let email = match args.email {
        Some(email) => normalize_email(&email),
        None => prompt("Admin email")?,
    };
    validate_email_like_identifier(&email).map_err(|err| anyhow::anyhow!(err.message))?;

    let admin_pass = args.admin_pass.unwrap_or_else(|| random_secret(32));
    let session_secret = random_secret(64);
    let mut lines = match fs::read_to_string(&args.env_file) {
        Ok(raw) => raw.lines().map(str::to_string).collect(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => default_env_lines(),
        Err(err) => return Err(err).with_context(|| format!("read {}", args.env_file.display())),
    };

    set_env_value(&mut lines, "HUMEN_ADMIN_EMAIL", &email);
    set_env_value(&mut lines, "HUMEN_ADMIN_PASSWORD", &admin_pass);
    set_env_value(&mut lines, "HUMEN_SESSION_SECRET", &session_secret);
    set_env_value(
        &mut lines,
        "HUMEN_USERS_FILE",
        "/var/lib/humen-mcp/users.json",
    );

    if let Some(parent) = args.env_file.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&args.env_file, format!("{}\n", lines.join("\n")))
        .with_context(|| format!("write {}", args.env_file.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&args.env_file, fs::Permissions::from_mode(0o640))
            .with_context(|| format!("chmod {}", args.env_file.display()))?;
    }

    println!("Initialized admin account in {}", args.env_file.display());
    println!("Admin email: {email}");
    println!("{} {}", "Admin password:", admin_pass);
    println!("Restart the service after changing the env file:");
    println!("  sudo systemctl restart humen-mcp.service");
    Ok(())
}

fn prompt(label: &str) -> anyhow::Result<String> {
    eprint!("{label}: ");
    io::stderr().flush().context("flush prompt")?;
    let mut value = String::new();
    io::stdin().read_line(&mut value).context("read prompt")?;
    let value = value.trim().to_string();
    if value.is_empty() {
        anyhow::bail!("{label} is required");
    }
    Ok(value)
}

fn default_env_lines() -> Vec<String> {
    [
        "HUMEN_BIND=127.0.0.1:8787",
        "HUMEN_PUBLIC_BASE_URL=https://your-domain.example/mcp",
        "HUMEN_WEB_DIST=/usr/share/humen-mcp/web",
        "HUMEN_USERS_FILE=/var/lib/humen-mcp/users.json",
        "HUMEN_ADMIN_EMAIL=<admin-email>",
        "HUMEN_ADMIN_PASSWORD=change-me",
        "HUMEN_SESSION_SECRET=change-this-to-a-long-random-secret",
        "HUMEN_TRASH_RETENTION_SECONDS=604800",
        "HUMEN_CLEANUP_INTERVAL_SECONDS=60",
        "HUMEN_GITHUB_CLIENT_ID=",
        "HUMEN_GITHUB_CLIENT_SECRET=",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn set_env_value(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}=");
    if let Some(line) = lines
        .iter_mut()
        .find(|line| line.trim_start().starts_with(&prefix))
    {
        *line = format!("{key}={value}");
    } else {
        lines.push(format!("{key}={value}"));
    }
}

async fn healthz() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn mcp_get() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "humen-mcp JSON-RPC endpoint. Use POST /mcp with application/json.\n",
    )
        .into_response()
}

async fn auth_config(State(state): State<AppState>) -> Json<AuthConfigResponse> {
    let mut settings = state
        .admin_settings
        .lock()
        .map(|settings| settings.clone())
        .unwrap_or_default();
    if let Some(github) = settings
        .oauth_channels
        .iter_mut()
        .find(|channel| channel.provider == "github")
    {
        github.enabled = state.github_enabled();
        github.client_id = state.config.github_client_id.clone().unwrap_or_default();
    }
    Json(AuthConfigResponse {
        github_enabled: state.github_enabled(),
        allow_registration: settings.allow_registration,
        oauth_channels: settings.oauth_channels,
    })
}

async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    if let Some(user) = authenticate_password(&state, &payload.email, &payload.pass)? {
        ensure_user_allowed(&state, &user.email)?;
        Ok(Json(
            state.create_session(user.email, AuthProvider::Password),
        ))
    } else {
        Err(ApiError::unauthorized("invalid email or password"))
    }
}

fn authenticate_password(
    state: &AppState,
    email: &str,
    pass: &str,
) -> Result<Option<User>, ApiError> {
    let normalized = normalize_email(email);
    if normalized == normalize_email(&state.config.admin_email)
        && pass == state.config.admin_password
    {
        return Ok(Some(User {
            email: state.config.admin_email.clone(),
            provider: AuthProvider::Password,
        }));
    }

    Ok(None)
}

async fn github_oauth_start(State(state): State<AppState>) -> Result<Redirect, ApiError> {
    if !state.github_enabled() {
        return Err(ApiError::bad_request("GitHub OAuth is not configured"));
    }
    let client_id = state
        .config
        .github_client_id
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("GitHub OAuth is not configured"))?;
    let redirect_uri = format!(
        "{}/api/auth/oauth/github/callback",
        state.config.public_base_url.trim_end_matches('/')
    );
    let url = format!(
        "https://github.com/login/oauth/authorize?client_id={client_id}&redirect_uri={redirect_uri}&scope=read:user%20user:email"
    );
    Ok(Redirect::temporary(&url))
}

async fn github_oauth_callback(
    State(state): State<AppState>,
    Query(query): Query<OAuthCallback>,
) -> Result<Redirect, ApiError> {
    if !state.github_enabled() {
        return Err(ApiError::bad_request("GitHub OAuth is not configured"));
    }
    let client_id = state
        .config
        .github_client_id
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("GitHub OAuth is not configured"))?;
    let client_secret = state
        .config
        .github_client_secret
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("GitHub OAuth is not configured"))?;

    let oauth_response: Value = state
        .http
        .post("https://github.com/login/oauth/access_token")
        .header(header::ACCEPT, "application/json")
        .json(&json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "code": query.code,
        }))
        .send()
        .await
        .map_err(|err| ApiError::upstream(format!("GitHub token exchange failed: {err}")))?
        .json()
        .await
        .map_err(|err| ApiError::upstream(format!("GitHub token response was invalid: {err}")))?;

    let access_token = oauth_response
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::upstream("GitHub token response did not include access_token"))?;

    let user: Value = state
        .http
        .get("https://api.github.com/user")
        .bearer_auth(access_token)
        .header(header::USER_AGENT, "humen-mcp")
        .send()
        .await
        .map_err(|err| ApiError::upstream(format!("GitHub user lookup failed: {err}")))?
        .json()
        .await
        .map_err(|err| ApiError::upstream(format!("GitHub user response was invalid: {err}")))?;

    let email = user
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| user.get("login").and_then(Value::as_str))
        .ok_or_else(|| ApiError::upstream("GitHub user response had no email or login"))?;
    let email = normalize_email(email);
    upsert_github_user(&state, &email)?;
    ensure_user_allowed(&state, &email)?;
    let auth = state.create_session(email, AuthProvider::Github);
    let redirect = format!(
        "{}/?token={}",
        state.config.public_base_url.trim_end_matches('/'),
        auth.token
    );
    Ok(Redirect::temporary(&redirect))
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    Ok(Json(json!({
        "user": session.user,
        "created_at": session.created_at
    })))
}

async fn list_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<HumanRequest>>, ApiError> {
    require_session(&state, &headers)?;
    let mut requests: Vec<_> = state
        .requests
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    requests.sort_by_key(|request| request.created_at);
    Ok(Json(requests))
}

async fn list_trash(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ExpiredRequest>>, ApiError> {
    require_session(&state, &headers)?;
    let mut trash: Vec<_> = state
        .trash
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    trash.sort_by_key(|entry| entry.expired_at);
    Ok(Json(trash))
}

async fn clear_trash(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let removed_count = state.trash.len();
    state.trash.clear();
    let _ = state
        .events
        .send(ServerEvent::TrashCleaned { removed_count });
    Ok(Json(json!({ "ok": true, "removed_count": removed_count })))
}

async fn list_online_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PublicUserProfile>>, ApiError> {
    require_session(&state, &headers)?;
    Ok(Json(
        user_profiles(&state, None, None)?
            .into_iter()
            .filter(|profile| profile.online)
            .collect(),
    ))
}

async fn search_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<PublicUserProfile>>, ApiError> {
    require_session(&state, &headers)?;
    Ok(Json(user_profiles(
        &state,
        query.q.as_deref(),
        query.tag.as_deref(),
    )?))
}

async fn list_tags(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_session(&state, &headers)?;
    Ok(Json(json!({ "tags": tag_counts(&state)? })))
}

async fn admin_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PublicUserProfile>>, ApiError> {
    require_admin(&state, &headers)?;
    Ok(Json(user_profiles(&state, None, None)?))
}

async fn admin_add_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AdminUserRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let email = normalize_email(&payload.email);
    validate_email_like_identifier(&email)?;
    let now = now_unix();
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    users.users.entry(email.clone()).or_insert(UserRecord {
        email: email.clone(),
        created_at: now,
        last_login_at: 0,
        profile: payload.profile,
        tags: normalize_tags(payload.tags),
        ban_expires_at: None,
        active_periods: Vec::new(),
    });
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save user: {err}")))?;
    Ok(Json(json!({ "ok": true, "email": email })))
}

async fn admin_update_user(
    State(state): State<AppState>,
    Path(email): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<AdminUserUpdate>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let email = normalize_email(&email);
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let record = users
        .users
        .get_mut(&email)
        .ok_or_else(|| ApiError::bad_request("user not found"))?;
    if let Some(profile) = payload.profile {
        record.profile = profile;
    }
    if let Some(tags) = payload.tags {
        record.tags = normalize_tags(tags);
    }
    if let Some(ban_expires_at) = payload.ban_expires_at {
        record.ban_expires_at = ban_expires_at;
    }
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save user: {err}")))?;
    Ok(Json(json!({ "ok": true })))
}

async fn admin_kick_user(
    State(state): State<AppState>,
    Path(email): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let email = normalize_email(&email);
    let mut removed_count = 0;
    state.sessions.retain(|_, session| {
        let keep = normalize_email(&session.user.email) != email;
        if !keep {
            removed_count += 1;
        }
        keep
    });
    Ok(Json(json!({ "ok": true, "removed_count": removed_count })))
}

async fn admin_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminSettings>, ApiError> {
    require_admin(&state, &headers)?;
    let mut settings = state
        .admin_settings
        .lock()
        .map_err(|_| ApiError::internal("settings lock poisoned"))?
        .clone();
    if let Some(github) = settings
        .oauth_channels
        .iter_mut()
        .find(|channel| channel.provider == "github")
    {
        github.enabled = state.github_enabled();
        github.client_id = state.config.github_client_id.clone().unwrap_or_default();
    }
    Ok(Json(settings))
}

async fn admin_update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AdminSettings>,
) -> Result<Json<AdminSettings>, ApiError> {
    require_admin(&state, &headers)?;
    let mut settings = state
        .admin_settings
        .lock()
        .map_err(|_| ApiError::internal("settings lock poisoned"))?;
    *settings = payload.clone();
    Ok(Json(payload))
}

async fn answer_request(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(payload): Json<AnswerRequest>,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let answer = HumanAnswer {
        answer: payload.answer,
        note: payload.note,
        answered_by: session.user.email,
        answered_at: now_unix(),
    };

    state.requests.remove(&id);
    if let Some((_, waiter)) = state.waiters.remove(&id) {
        if waiter.send(answer.clone()).is_err() {
            warn!(%id, "MCP caller already disconnected before human answer");
        }
    }
    let _ = state.events.send(ServerEvent::RequestAnswered {
        id,
        answer: answer.clone(),
    });
    Ok(Json(json!({ "ok": true, "answer": answer })))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let session = if let Some(token) = query.token.as_deref() {
        let session = state
            .session_from_token(token)
            .ok_or_else(|| ApiError::unauthorized("missing or invalid websocket token"))?;
        ensure_user_allowed(&state, &session.user.email)?;
        session
    } else {
        require_session(&state, &headers)?
    };
    Ok(ws.on_upgrade(move |socket| websocket(socket, state, session)))
}

async fn websocket(mut socket: WebSocket, state: AppState, session: Session) {
    let online_count = increment_online(&state);
    let active_index = begin_active_period(&state, &session.user.email);

    let initial: Vec<_> = state
        .requests
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    if socket
        .send(Message::Text(
            json!({
                "type": "snapshot",
                "requests": initial,
                "online_count": online_count
            })
            .to_string()
            .into(),
        ))
        .await
        .is_err()
    {
        decrement_online(&state);
        end_active_period(&state, &session.user.email, active_index);
        return;
    }

    let mut rx = state.events.subscribe();
    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        let Ok(text) = serde_json::to_string(&event) else {
                            continue;
                        };
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }

    decrement_online(&state);
    end_active_period(&state, &session.user.email, active_index);
}

fn decrement_online(state: &AppState) {
    let previous = state
        .online_humans
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
            Some(count.saturating_sub(1))
        })
        .unwrap_or(0);
    let online_count = previous.saturating_sub(1);
    let _ = state
        .events
        .send(ServerEvent::PresenceChanged { online_count });
}

fn increment_online(state: &AppState) -> usize {
    let online_count = state.online_humans.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = state
        .events
        .send(ServerEvent::PresenceChanged { online_count });
    online_count
}

fn expire_request(state: &AppState, id: Uuid, reason: String) -> Option<ExpiredRequest> {
    let (_, request) = state.requests.remove(&id)?;
    state.waiters.remove(&id);
    let expired = ExpiredRequest {
        request,
        expired_at: now_unix(),
        reason,
    };
    state.trash.insert(id, expired.clone());
    let _ = state.events.send(ServerEvent::RequestExpired {
        id,
        expired_request: expired.clone(),
    });
    Some(expired)
}

async fn trash_cleanup_loop(state: AppState) {
    let interval_seconds = state.config.cleanup_interval_seconds.max(1);
    let retention_seconds = state.config.trash_retention_seconds;
    let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
    loop {
        interval.tick().await;
        let cutoff = now_unix().saturating_sub(retention_seconds);
        let before = state.trash.len();
        state
            .trash
            .retain(|_, expired| expired.expired_at >= cutoff);
        let removed_count = before.saturating_sub(state.trash.len());
        if removed_count > 0 {
            let _ = state
                .events
                .send(ServerEvent::TrashCleaned { removed_count });
        }
    }
}

fn begin_active_period(state: &AppState, email: &str) -> Option<usize> {
    let email = normalize_email(email);
    let mut users = state.users.lock().ok()?;
    let now = now_unix();
    let record = users.users.entry(email.clone()).or_insert(UserRecord {
        email: email.clone(),
        created_at: now,
        last_login_at: now,
        profile: String::new(),
        tags: Vec::new(),
        ban_expires_at: None,
        active_periods: Vec::new(),
    });
    record.active_periods.push(ActivePeriod {
        user_id: email.clone(),
        connected_at: now,
        disconnected_at: None,
        duration_seconds: None,
    });
    let index = record.active_periods.len().saturating_sub(1);
    if let Err(err) = users.save(&state.config.users_file) {
        warn!(%err, "failed to save active period start");
    }
    Some(index)
}

fn end_active_period(state: &AppState, email: &str, active_index: Option<usize>) {
    let Some(active_index) = active_index else {
        return;
    };
    let email = normalize_email(email);
    let Ok(mut users) = state.users.lock() else {
        return;
    };
    let now = now_unix();
    if let Some(record) = users.users.get_mut(&email) {
        if let Some(period) = record.active_periods.get_mut(active_index) {
            if period.disconnected_at.is_none() {
                period.disconnected_at = Some(now);
                period.duration_seconds = Some(now.saturating_sub(period.connected_at));
            }
        }
    }
    if let Err(err) = users.save(&state.config.users_file) {
        warn!(%err, "failed to save active period end");
    }
}

async fn mcp(
    State(state): State<AppState>,
    Json(payload): Json<McpRequest>,
) -> Result<Json<Value>, ApiError> {
    if payload.jsonrpc.as_deref() != Some("2.0") {
        return Ok(Json(mcp_error(
            payload.id,
            -32600,
            "expected JSON-RPC 2.0 request",
        )));
    }

    let id = payload.id.clone();
    let result = match payload.method.as_str() {
        "initialize" => json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "humen-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
        "notifications/initialized" => Value::Null,
        "tools/list" => json!({
            "tools": [
                {
                    "name": "ask_humen",
                    "description": "Ask a logged-in human to complete a simple task and return the answer.",
                    "inputSchema": ask_humen_schema()
                },
                {
                    "name": "list_online_humens",
                    "description": "List online human operators and their public profiles.",
                    "inputSchema": json!({ "type": "object", "properties": {} })
                },
                {
                    "name": "search_humen_profiles",
                    "description": "Search human profiles by text or #tag.",
                    "inputSchema": json!({
                        "type": "object",
                        "properties": {
                            "q": { "type": "string" },
                            "tag": { "type": "string" }
                        }
                    })
                },
                {
                    "name": "list_humen_tags",
                    "description": "List known #tags and their usage counts.",
                    "inputSchema": json!({ "type": "object", "properties": {} })
                }
            ]
        }),
        "tools/call" => return call_tool(state, payload).await,
        _ => return Ok(Json(mcp_error(id, -32601, "method not found"))),
    };

    Ok(Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })))
}

async fn call_tool(state: AppState, payload: McpRequest) -> Result<Json<Value>, ApiError> {
    let id = payload.id.clone();
    let name = payload
        .params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("tools/call params.name is required"))?;
    match name {
        "ask_humen" => {}
        "list_online_humens" => {
            let users: Vec<_> = user_profiles(&state, None, None)?
                .into_iter()
                .filter(|profile| profile.online)
                .collect();
            return Ok(Json(mcp_text_result(id, json!({ "users": users }))));
        }
        "search_humen_profiles" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let q = arguments.get("q").and_then(Value::as_str);
            let tag = arguments.get("tag").and_then(Value::as_str);
            let users = user_profiles(&state, q, tag)?;
            return Ok(Json(mcp_text_result(id, json!({ "users": users }))));
        }
        "list_humen_tags" => {
            return Ok(Json(mcp_text_result(
                id,
                json!({ "tags": tag_counts(&state)? }),
            )));
        }
        _ => return Ok(Json(mcp_error(id, -32602, "unknown tool"))),
    }

    let arguments = payload
        .params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Null);
    let create: CreateHumanRequest = serde_json::from_value(arguments)
        .map_err(|err| ApiError::bad_request(format!("invalid ask_humen arguments: {err}")))?;
    let now = now_unix();
    let timeout_seconds = create.timeout_seconds.clamp(30, 86400);
    let mut tag_sources = vec![create.title.as_str(), create.prompt.as_str()];
    tag_sources.extend(create.steps.iter().map(String::as_str));
    let tags = extract_tags(&tag_sources);
    let request = HumanRequest {
        id: Uuid::new_v4(),
        kind: create.kind,
        title: create.title,
        prompt: create.prompt,
        choices: create.choices,
        image_url: create.image_url,
        steps: create.steps,
        created_at: now,
        timeout_seconds,
        expires_at: now.saturating_add(timeout_seconds),
        tags,
    };
    let timeout = Duration::from_secs(request.timeout_seconds);
    let (tx, rx) = oneshot::channel();
    state.waiters.insert(request.id, tx);
    state.requests.insert(request.id, request.clone());
    let _ = state.events.send(ServerEvent::RequestCreated {
        request: request.clone(),
    });

    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(answer)) => Ok(Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&answer).unwrap_or_else(|_| answer.answer.clone())
                }]
            }
        }))),
        Ok(Err(_)) => Ok(Json(mcp_error(id, -32000, "human answer channel closed"))),
        Err(_) => {
            state.waiters.remove(&request.id);
            let expired = expire_request(
                &state,
                request.id,
                format!(
                    "Human request timed out after {} seconds",
                    request.timeout_seconds
                ),
            )
            .unwrap_or_else(|| ExpiredRequest {
                request: request.clone(),
                expired_at: now_unix(),
                reason: format!(
                    "Human request timed out after {} seconds",
                    request.timeout_seconds
                ),
            });
            Ok(Json(mcp_error_with_data(
                id,
                -32001,
                &expired.reason,
                json!({
                    "request_id": expired.request.id,
                    "title": expired.request.title,
                    "timeout_seconds": expired.request.timeout_seconds,
                    "expired_at": expired.expired_at,
                    "suggestion": "Try again with a longer timeout or simplify the request."
                }),
            )))
        }
    }
}

fn mcp_text_result(id: Option<Value>, value: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
            }]
        }
    })
}

fn ask_humen_schema() -> Value {
    json!({
        "type": "object",
        "required": ["title", "prompt"],
        "properties": {
            "kind": {
                "type": "string",
                "enum": ["choice", "text", "image_review", "steps"],
                "default": "text"
            },
            "title": { "type": "string" },
            "prompt": { "type": "string" },
            "choices": {
                "type": "array",
                "items": { "type": "string" }
            },
            "image_url": { "type": "string" },
            "steps": {
                "type": "array",
                "items": { "type": "string" }
            },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 30,
                "maximum": 86400,
                "default": 60
            }
        }
    })
}

fn require_session(state: &AppState, headers: &HeaderMap) -> Result<Session, ApiError> {
    let session = state
        .session_from_headers(headers)
        .ok_or_else(|| ApiError::unauthorized("missing or invalid bearer token"))?;
    ensure_user_allowed(state, &session.user.email)?;
    Ok(session)
}

fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<Session, ApiError> {
    let session = require_session(state, headers)?;
    if normalize_email(&session.user.email) == normalize_email(&state.config.admin_email) {
        Ok(session)
    } else {
        Err(ApiError::unauthorized("admin access required"))
    }
}

fn ensure_user_allowed(state: &AppState, email: &str) -> Result<(), ApiError> {
    if normalize_email(email) == normalize_email(&state.config.admin_email) {
        return Ok(());
    }
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    if let Some(record) = users.users.get(&normalize_email(email)) {
        if record
            .ban_expires_at
            .is_some_and(|expires_at| expires_at > now_unix())
        {
            return Err(ApiError::unauthorized("user is banned"));
        }
    }
    Ok(())
}

fn upsert_github_user(state: &AppState, email: &str) -> Result<(), ApiError> {
    validate_email_like_identifier(email)?;
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let now = now_unix();
    if let Some(record) = users.users.get_mut(email) {
        record.last_login_at = now;
    } else {
        let allow_registration = state
            .admin_settings
            .lock()
            .map(|settings| settings.allow_registration)
            .unwrap_or(true);
        if !allow_registration {
            return Err(ApiError::unauthorized("new user registration is disabled"));
        }
        users.insert(UserRecord {
            email: email.to_string(),
            created_at: now,
            last_login_at: now,
            profile: String::new(),
            tags: Vec::new(),
            ban_expires_at: None,
            active_periods: Vec::new(),
        });
    }
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save GitHub user: {err}")))?;
    Ok(())
}

fn user_profiles(
    state: &AppState,
    query: Option<&str>,
    tag: Option<&str>,
) -> Result<Vec<PublicUserProfile>, ApiError> {
    let online = online_emails(state);
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let query = query.map(|value| value.to_ascii_lowercase());
    let tag = tag.and_then(normalize_tag);
    let mut profiles: Vec<_> = users
        .users
        .values()
        .map(|record| PublicUserProfile {
            email: record.email.clone(),
            provider: AuthProvider::Github,
            profile: record.profile.clone(),
            tags: record.tags.clone(),
            online: online.contains_key(&normalize_email(&record.email)),
            last_login_at: record.last_login_at,
            ban_expires_at: record.ban_expires_at,
        })
        .filter(|profile| {
            let query_matches = query.as_ref().is_none_or(|query| {
                profile.email.to_ascii_lowercase().contains(query)
                    || profile.profile.to_ascii_lowercase().contains(query)
                    || profile
                        .tags
                        .iter()
                        .any(|tag| tag.to_ascii_lowercase().contains(query))
            });
            let tag_matches = tag
                .as_ref()
                .is_none_or(|tag| profile.tags.iter().any(|candidate| candidate == tag));
            query_matches && tag_matches
        })
        .collect();

    let admin_email = normalize_email(&state.config.admin_email);
    if query
        .as_ref()
        .is_none_or(|query| admin_email.contains(query))
        && tag.is_none()
        && !profiles
            .iter()
            .any(|profile| normalize_email(&profile.email) == admin_email)
    {
        profiles.push(PublicUserProfile {
            email: state.config.admin_email.clone(),
            provider: AuthProvider::Password,
            profile: "Administrator".to_string(),
            tags: vec!["#admin".to_string()],
            online: online.contains_key(&admin_email),
            last_login_at: 0,
            ban_expires_at: None,
        });
    }

    profiles.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| a.email.cmp(&b.email)));
    Ok(profiles)
}

fn tag_counts(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for record in users.users.values() {
        for tag in &record.tags {
            if let Some(tag) = normalize_tag(tag) {
                *counts.entry(tag).or_default() += 1;
            }
        }
    }
    let mut tags: Vec<_> = counts
        .into_iter()
        .map(|(tag, count)| json!({ "tag": tag, "count": count }))
        .collect();
    tags.sort_by_key(|item| item["tag"].as_str().unwrap_or_default().to_string());
    Ok(tags)
}

fn online_emails(state: &AppState) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    let Ok(users) = state.users.lock() else {
        return counts;
    };
    for record in users.users.values() {
        let active_count = record
            .active_periods
            .iter()
            .filter(|period| period.disconnected_at.is_none())
            .count();
        if active_count > 0 {
            counts.insert(normalize_email(&record.email), active_count);
        }
    }
    counts
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut normalized: Vec<_> = tags
        .into_iter()
        .filter_map(|tag| normalize_tag(&tag))
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_tag(tag: &str) -> Option<String> {
    let tag = tag.trim().trim_matches(',').trim_matches(';');
    if tag.len() < 2 {
        return None;
    }
    let tag = if tag.starts_with('#') {
        tag.to_string()
    } else {
        format!("#{tag}")
    };
    if tag[1..]
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Some(tag.to_ascii_lowercase())
    } else {
        None
    }
}

fn extract_tags(values: &[&str]) -> Vec<String> {
    let mut tags = Vec::new();
    for value in values {
        for token in value.split_whitespace() {
            if token.starts_with('#') {
                if let Some(tag) = normalize_tag(token) {
                    tags.push(tag);
                }
            }
        }
    }
    normalize_tags(tags)
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn validate_email_like_identifier(email: &str) -> Result<(), ApiError> {
    if email.len() < 2 || email.contains(char::is_whitespace) {
        return Err(ApiError::bad_request("valid GitHub identity is required"));
    }
    Ok(())
}

fn non_empty(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn mcp_error(id: Option<Value>, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message.into()
        }
    })
}

fn mcp_error_with_data(
    id: Option<Value>,
    code: i64,
    message: impl Into<String>,
    data: Value,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message.into(),
            "data": data
        }
    })
}

fn default_timeout() -> u64 {
    60
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn random_secret(len: usize) -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }

    fn upstream(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> AppState {
        AppState::new(Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            public_base_url: "http://127.0.0.1:8787".to_string(),
            web_dist: "./humen-mcp-webui/dist".to_string(),
            users_file: std::env::temp_dir()
                .join(format!("humen-mcp-test-{}.json", Uuid::new_v4())),
            admin_email: "admin-local".to_string(),
            admin_password: "secret".to_string(),
            session_secret: "test-session-secret".to_string(),
            github_client_id: None,
            github_client_secret: None,
            trash_retention_seconds: 60,
            cleanup_interval_seconds: 60,
        })
        .unwrap()
    }

    #[test]
    fn ask_humen_schema_exposes_simple_task_kinds() {
        let schema = ask_humen_schema();
        let kinds = schema["properties"]["kind"]["enum"].as_array().unwrap();
        assert!(kinds.contains(&json!("choice")));
        assert!(kinds.contains(&json!("text")));
        assert!(kinds.contains(&json!("image_review")));
        assert!(kinds.contains(&json!("steps")));
        assert_eq!(
            schema["properties"]["timeout_seconds"]["default"],
            json!(60)
        );
        assert_eq!(default_timeout(), 60);
    }

    #[test]
    fn expiring_request_moves_it_to_trash_and_emits_event() {
        let state = test_state();
        let request = HumanRequest {
            id: Uuid::new_v4(),
            kind: TaskKind::Text,
            title: "Check status".to_string(),
            prompt: "Say ok".to_string(),
            choices: Vec::new(),
            image_url: None,
            steps: Vec::new(),
            created_at: 100,
            timeout_seconds: 60,
            expires_at: 160,
            tags: Vec::new(),
        };
        let (tx, _rx) = oneshot::channel();
        let mut events = state.events.subscribe();
        state.requests.insert(request.id, request.clone());
        state.waiters.insert(request.id, tx);

        let expired = expire_request(&state, request.id, "timeout".to_string()).unwrap();
        let event = events.try_recv().unwrap();

        assert!(state.requests.get(&request.id).is_none());
        assert!(state.waiters.get(&request.id).is_none());
        assert!(state.trash.get(&request.id).is_some());
        assert_eq!(expired.request.id, request.id);
        match event {
            ServerEvent::RequestExpired {
                id,
                expired_request,
            } => {
                assert_eq!(id, request.id);
                assert_eq!(expired_request.request.id, request.id);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn active_periods_persist_user_id_and_duration() {
        let state = test_state();
        let index = begin_active_period(&state, "user-one");
        end_active_period(&state, "user-one", index);

        let users = UserStore::load(&state.config.users_file).unwrap();
        let record = users.users.get("user-one").unwrap();
        let period = record.active_periods.first().unwrap();

        assert_eq!(period.user_id, "user-one");
        assert!(period.disconnected_at.is_some());
        assert!(period.duration_seconds.is_some());
    }

    #[test]
    fn bearer_session_round_trips() {
        let state = test_state();
        let auth = state.create_session("admin-local", AuthProvider::Password);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", auth.token).parse().unwrap(),
        );
        let session = state.session_from_headers(&headers).unwrap();

        assert_eq!(session.user.email, "admin-local");
        assert!(state.session_from_token("not-a-token").is_none());
    }

    #[test]
    fn online_count_saturates_on_extra_disconnects() {
        let state = test_state();
        assert_eq!(increment_online(&state), 1);
        decrement_online(&state);
        decrement_online(&state);

        assert_eq!(state.online_humans.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn init_admin_writes_env_file() {
        let env_file = std::env::temp_dir().join(format!("humen-mcp-env-{}.env", Uuid::new_v4()));

        init_admin(InitAdminArgs {
            env_file: env_file.clone(),
            email: Some("admin-local".to_string()),
            admin_pass: Some("fixed-admin-pass".to_string()),
        })
        .unwrap();

        let raw = fs::read_to_string(env_file).unwrap();
        assert!(raw.contains("HUMEN_ADMIN_EMAIL=admin-local"));
        assert!(raw.contains("HUMEN_ADMIN_PASSWORD=fixed-admin-pass"));
        assert!(raw.contains("HUMEN_USERS_FILE=/var/lib/humen-mcp/users.json"));
        assert!(!raw.contains("HUMEN_SESSION_SECRET=change-this-to-a-long-random-secret"));
    }
}
