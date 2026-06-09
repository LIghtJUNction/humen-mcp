use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
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
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use clap::{Args, Parser, Subcommand};
use dashmap::DashMap;
use futures_util::StreamExt;
use qrcode::{render::svg, QrCode};
use rand::{distr::Alphanumeric, Rng};
use reqwest::Client;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{
    process::Command as TokioCommand,
    sync::{broadcast, oneshot},
};
use tower_http::{
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{info, warn};
use uuid::Uuid;
use webauthn_rs::prelude::{
    CreationChallengeResponse, CredentialID, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse, Url, Webauthn,
    WebauthnBuilder,
};

const WEIXIN_DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const WEIXIN_CLIENT_VERSION: &str = "1";
const WEIXIN_DEFAULT_POLL_TIMEOUT_MS: u64 = 35_000;
const WEIXIN_DEFAULT_API_TIMEOUT_MS: u64 = 15_000;
const ADMIN_TAG: &str = "#admin";

include!("app_core.rs");
include!("models.rs");
include!("server.rs");
include!("auth.rs");
include!("passkeys.rs");
include!("admin_weixin.rs");
include!("github.rs");
include!("handlers.rs");
include!("self_update.rs");
include!("weixin.rs");
include!("ws.rs");
include!("storage.rs");
include!("mcp.rs");
include!("profiles.rs");
include!("utils.rs");
include!("error.rs");
include!("tests.rs");
