use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
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

#[derive(Debug, Clone, Parser)]
#[command(author, version, about)]
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

    #[arg(long, env = "HUMEN_ADMIN_EMAIL", default_value = "admin@example.com")]
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
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    requests: Arc<DashMap<Uuid, HumanRequest>>,
    waiters: Arc<DashMap<Uuid, oneshot::Sender<HumanAnswer>>>,
    sessions: Arc<DashMap<String, Session>>,
    events: broadcast::Sender<ServerEvent>,
    http: Client,
}

impl AppState {
    fn new(config: Config) -> Self {
        let (events, _) = broadcast::channel(128);
        Self {
            config: Arc::new(config),
            requests: Arc::new(DashMap::new()),
            waiters: Arc::new(DashMap::new()),
            sessions: Arc::new(DashMap::new()),
            events,
            http: Client::new(),
        }
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

#[derive(Clone, Debug, Deserialize)]
struct AnswerRequest {
    answer: String,
    note: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerEvent {
    RequestCreated { request: HumanRequest },
    RequestAnswered { id: Uuid, answer: HumanAnswer },
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::parse();
    let bind = config.bind;
    let web_dist = config.web_dist.clone();
    let state = AppState::new(config);

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/mcp", post(mcp))
        .route("/api/auth/login", post(login))
        .route("/api/auth/oauth/github/start", get(github_oauth_start))
        .route(
            "/api/auth/oauth/github/callback",
            get(github_oauth_callback),
        )
        .route("/api/me", get(me))
        .route("/api/requests", get(list_requests))
        .route("/api/requests/{id}/answer", post(answer_request))
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

async fn healthz() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    if payload.email == state.config.admin_email && payload.pass == state.config.admin_password {
        Ok(Json(
            state.create_session(payload.email, AuthProvider::Password),
        ))
    } else {
        Err(ApiError::unauthorized("invalid email or password"))
    }
}

async fn github_oauth_start(State(state): State<AppState>) -> Result<Redirect, ApiError> {
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
    if let Some(token) = query.token.as_deref() {
        state
            .session_from_token(token)
            .ok_or_else(|| ApiError::unauthorized("missing or invalid websocket token"))?;
    } else {
        require_session(&state, &headers)?;
    }
    Ok(ws.on_upgrade(move |socket| websocket(socket, state)))
}

async fn websocket(mut socket: WebSocket, state: AppState) {
    let initial: Vec<_> = state
        .requests
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    if socket
        .send(Message::Text(
            json!({ "type": "snapshot", "requests": initial })
                .to_string()
                .into(),
        ))
        .await
        .is_err()
    {
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
            "tools": [{
                "name": "ask_humen",
                "description": "Ask a logged-in human to complete a simple task and return the answer.",
                "inputSchema": ask_humen_schema()
            }]
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
    if name != "ask_humen" {
        return Ok(Json(mcp_error(id, -32602, "unknown tool")));
    }

    let arguments = payload
        .params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Null);
    let create: CreateHumanRequest = serde_json::from_value(arguments)
        .map_err(|err| ApiError::bad_request(format!("invalid ask_humen arguments: {err}")))?;
    let request = HumanRequest {
        id: Uuid::new_v4(),
        kind: create.kind,
        title: create.title,
        prompt: create.prompt,
        choices: create.choices,
        image_url: create.image_url,
        steps: create.steps,
        created_at: now_unix(),
        timeout_seconds: create.timeout_seconds.clamp(5, 3600),
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
            state.requests.remove(&request.id);
            state.waiters.remove(&request.id);
            Ok(Json(mcp_error(id, -32001, "human answer timed out")))
        }
    }
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
                "minimum": 5,
                "maximum": 3600,
                "default": 300
            }
        }
    })
}

fn require_session(state: &AppState, headers: &HeaderMap) -> Result<Session, ApiError> {
    state
        .session_from_headers(headers)
        .ok_or_else(|| ApiError::unauthorized("missing or invalid bearer token"))
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

fn default_timeout() -> u64 {
    300
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}
