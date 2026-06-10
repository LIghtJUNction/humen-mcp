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

    #[arg(long, env = "HUMEN_DB_FILE", default_value = "./humen-mcp.sqlite3")]
    db_file: PathBuf,

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

    #[arg(long, env = "HUMEN_GITHUB_API_TOKEN")]
    github_api_token: Option<String>,

    #[arg(
        long,
        env = "HUMEN_TRASH_RETENTION_SECONDS",
        default_value_t = 7 * 24 * 60 * 60
    )]
    trash_retention_seconds: u64,

    #[arg(long, env = "HUMEN_CLEANUP_INTERVAL_SECONDS", default_value_t = 60)]
    cleanup_interval_seconds: u64,

    #[arg(long, env = "HUMEN_SELF_UPDATE_COMMAND", default_value = "")]
    self_update_command: String,

    #[arg(
        long,
        env = "HUMEN_SELF_UPDATE_TIMEOUT_SECONDS",
        default_value_t = 120
    )]
    self_update_timeout_seconds: u64,

    #[arg(long, env = "HUMEN_PLUGIN_DIR", default_value = "")]
    plugin_dir: String,
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    requests: Arc<DashMap<Uuid, HumanRequest>>,
    waiters: Arc<DashMap<Uuid, oneshot::Sender<HumanAnswer>>>,
    trash: Arc<DashMap<Uuid, ExpiredRequest>>,
    sessions: Arc<DashMap<String, Session>>,
    passkey_registrations: Arc<DashMap<Uuid, PendingPasskeyRegistration>>,
    passkey_authentications: Arc<DashMap<Uuid, PendingPasskeyAuthentication>>,
    users: Arc<Mutex<UserStore>>,
    admin_settings: Arc<Mutex<AdminSettings>>,
    db: Arc<Mutex<Connection>>,
    events: broadcast::Sender<ServerEvent>,
    shutdown: broadcast::Sender<()>,
    self_update_running: Arc<AtomicBool>,
    plugins: Arc<PluginRegistry>,
    http: Client,
    webauthn: Option<Arc<Webauthn>>,
}

impl AppState {
    fn new(config: Config) -> anyhow::Result<Self> {
        let (events, _) = broadcast::channel(128);
        let (shutdown, _) = broadcast::channel(8);
        let mut users = UserStore::load(&config.users_file)?;
        users.admin_settings = sanitize_admin_settings(users.admin_settings.clone());
        for record in users.users.values_mut() {
            prepare_user_record(record);
        }
        clear_stale_active_periods(&mut users);
        let admin_settings = users.admin_settings.clone();
        let db = open_db(&config.db_file)?;
        let plugins = load_plugins(config.plugin_dir.trim());
        let webauthn = match build_webauthn(&config.public_base_url) {
            Ok(webauthn) => Some(Arc::new(webauthn)),
            Err(err) => {
                warn!(error = %err, "passkey support is disabled");
                None
            }
        };
        let state = Self {
            config: Arc::new(config),
            requests: Arc::new(DashMap::new()),
            waiters: Arc::new(DashMap::new()),
            trash: Arc::new(DashMap::new()),
            sessions: Arc::new(DashMap::new()),
            passkey_registrations: Arc::new(DashMap::new()),
            passkey_authentications: Arc::new(DashMap::new()),
            users: Arc::new(Mutex::new(users)),
            admin_settings: Arc::new(Mutex::new(admin_settings)),
            db: Arc::new(Mutex::new(db)),
            events,
            shutdown,
            self_update_running: Arc::new(AtomicBool::new(false)),
            plugins: Arc::new(plugins),
            http: Client::new(),
            webauthn,
        };
        load_buffered_requests(&state)?;
        Ok(state)
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
        let session = Session {
            user: user.clone(),
            created_at: now_unix(),
        };
        self.sessions.insert(token_hash.clone(), session.clone());
        if let Err(err) = db_store_web_session(
            self,
            &token_hash,
            &session,
            session.created_at + WEB_SESSION_TTL_SECONDS,
        ) {
            warn!(error = %err.message, "failed to persist web session");
        }
        AuthResponse {
            token: raw_token,
            user,
        }
    }

    fn session_from_headers(&self, headers: &HeaderMap) -> Option<Session> {
        if let Some(session) = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .and_then(|token| self.session_from_token(token))
        {
            return Some(session);
        }
        session_cookie_token(headers).and_then(|token| self.session_from_token(token))
    }

    fn session_from_token(&self, token: &str) -> Option<Session> {
        let token_hash = self.hash_token(token);
        if let Some(session) = self.sessions.get(&token_hash).map(|s| s.clone()) {
            return Some(session);
        }
        let session = match db_get_web_session(self, &token_hash) {
            Ok(session) => session,
            Err(err) => {
                warn!(error = %err.message, "failed to restore web session");
                None
            }
        }?;
        self.sessions.insert(token_hash, session.clone());
        Some(session)
    }

    fn destroy_session_token(&self, token: &str) {
        let token_hash = self.hash_token(token);
        self.sessions.remove(&token_hash);
        if let Err(err) = db_delete_web_session(self, &token_hash) {
            warn!(error = %err.message, "failed to delete web session");
        }
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

fn build_webauthn(public_base_url: &str) -> anyhow::Result<Webauthn> {
    let mut origin = Url::parse(public_base_url.trim_end_matches('/'))
        .with_context(|| format!("parse public base URL for passkeys: {public_base_url}"))?;
    origin.set_path("");
    origin.set_query(None);
    origin.set_fragment(None);
    let rp_id = origin
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("public base URL has no host"))?
        .to_string();
    WebauthnBuilder::new(&rp_id, &origin)
        .map_err(|err| anyhow::anyhow!("invalid passkey relying party: {err:?}"))?
        .rp_name("humen-mcp")
        .allow_any_port(is_loopback_rp_id(&rp_id))
        .build()
        .map_err(|err| anyhow::anyhow!("build passkey relying party: {err:?}"))
}

fn is_loopback_rp_id(rp_id: &str) -> bool {
    matches!(rp_id, "localhost" | "127.0.0.1" | "::1")
}
