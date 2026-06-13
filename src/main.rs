use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    fs,
    io::{self, Write},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{
        IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, patch, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use clap::{Args, Parser, Subcommand};
use dashmap::DashMap;
use futures_util::{StreamExt, stream};
use humen_mcp_sdk::{HumenPluginManifest, HumenTaskKind, RequestTemplate};
#[cfg(test)]
use humen_mcp_sdk::{RouteStrategy, ScoringRule, ThirdPartyChannel};
use qrcode::{QrCode, render::svg};
use rand::{Rng, distr::Alphanumeric};
use reqwest::Client;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
const MEMO_BODY_MAX_CHARS: usize = 1200;
const MEMO_BODY_MAX_LINES: usize = 30;
const MEMO_UNREAD_LIMIT_PER_PAIR: u64 = 25;
const HUMAN_MEMO_LIST_LIMIT: u64 = 50;
const AGENT_INBOX_LIMIT_MAX: u64 = 100;
const AGENT_PANEL_MESSAGES_LIMIT: u64 = 25;

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
include!("plugins.rs");
include!("federation.rs");
include!("mcp.rs");
include!("profiles.rs");
include!("utils.rs");
include!("error.rs");
include!("tests.rs");
