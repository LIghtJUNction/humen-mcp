#[derive(Clone, Debug, Serialize, Deserialize)]
struct HumanRequest {
    id: Uuid,
    kind: TaskKind,
    title: String,
    prompt: String,
    choices: Vec<String>,
    image_url: Option<String>,
    image_base64: Option<String>,
    image_mime_type: Option<String>,
    steps: Vec<String>,
    created_at: u64,
    timeout_seconds: u64,
    expires_at: u64,
    tags: Vec<String>,
    #[serde(default)]
    assigned_to: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TaskKind {
    Choice,
    Judgment,
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
    #[serde(
        alias = "image_base_64",
        alias = "base64_image",
        alias = "base_64_image",
        alias = "base64",
        alias = "base_64"
    )]
    image_base64: Option<String>,
    #[serde(alias = "mime_type")]
    image_mime_type: Option<String>,
    #[serde(default)]
    steps: Vec<String>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    #[serde(default)]
    background: bool,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AnsweredRequest {
    request: HumanRequest,
    answer: HumanAnswer,
    answered_late: bool,
}

#[derive(Clone, Debug, Serialize)]
struct HumanLeaderboardEntry {
    email: String,
    requests_handled: u64,
    sent_tokens: u64,
    latest_answered_at: Option<u64>,
    reputation: f64,
    ratings_count: u64,
    profile: String,
    tags: Vec<String>,
    online: bool,
}

#[derive(Clone, Debug)]
struct HumanLeaderboardStat {
    email: String,
    requests_handled: u64,
    sent_tokens: u64,
    latest_answered_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AgentTaskStatus {
    Open,
    InProgress,
    Done,
    Archived,
}

impl Default for AgentTaskStatus {
    fn default() -> Self {
        Self::Open
    }
}

impl AgentTaskStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Archived => "archived",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AgentTask {
    id: Uuid,
    title: String,
    description: String,
    steps: Vec<String>,
    tags: Vec<String>,
    created_by: String,
    assigned_to: String,
    created_at: u64,
    updated_at: u64,
    due_at: Option<u64>,
    status: AgentTaskStatus,
    human_note: Option<String>,
    completed_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HumanReport {
    id: Uuid,
    reporter_email: String,
    reported_email: String,
    reason: String,
    created_at: u64,
    status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReputationSummary {
    reputation: f64,
    ratings_count: u64,
}

impl Default for ReputationSummary {
    fn default() -> Self {
        Self {
            reputation: 5.0,
            ratings_count: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(dead_code)]
struct LateHumanReply {
    request: HumanRequest,
    answer: HumanAnswer,
    expired_at: Option<u64>,
    answered_late: bool,
    read_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
struct PublicUserProfile {
    email: String,
    provider: AuthProvider,
    profile: String,
    tags: Vec<String>,
    reputation: f64,
    ratings_count: u64,
    friend_code: String,
    intro_code: String,
    is_public: bool,
    is_friend: bool,
    friend_request_sent: bool,
    friend_request_received: bool,
    onboarding_completed: bool,
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
        request: HumanRequest,
        answer: HumanAnswer,
        answered_late: bool,
    },
    RequestExpired {
        id: Uuid,
        expired_request: ExpiredRequest,
    },
    TaskCreated {
        task: AgentTask,
    },
    TaskUpdated {
        task: AgentTask,
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
    Passkey,
}

#[derive(Clone, Debug)]
struct Session {
    user: User,
    created_at: u64,
}

#[derive(Clone, Debug)]
struct AgentContext {
    email: String,
    can_view_directory: bool,
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
    passkey_enabled: bool,
    allow_registration: bool,
    oauth_channels: Vec<OAuthChannel>,
}

#[derive(Clone, Debug, Serialize)]
struct PasskeyInfo {
    id: Uuid,
    name: String,
    created_at: u64,
    last_used_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredPasskey {
    id: Uuid,
    name: String,
    created_at: u64,
    #[serde(default)]
    last_used_at: Option<u64>,
    credential: Passkey,
}

#[derive(Clone, Debug)]
struct PendingPasskeyRegistration {
    email: String,
    state: PasskeyRegistration,
    created_at: u64,
}

#[derive(Clone, Debug)]
struct PendingPasskeyAuthentication {
    email: String,
    state: PasskeyAuthentication,
    created_at: u64,
}

#[derive(Debug, Serialize)]
struct PasskeyRegistrationStartResponse {
    registration_id: Uuid,
    options: CreationChallengeResponse,
}

#[derive(Debug, Deserialize)]
struct PasskeyRegistrationFinishRequest {
    registration_id: Uuid,
    #[serde(default)]
    name: Option<String>,
    credential: RegisterPublicKeyCredential,
}

#[derive(Debug, Deserialize)]
struct PasskeyAuthenticationStartRequest {
    email: String,
}

#[derive(Debug, Serialize)]
struct PasskeyAuthenticationStartResponse {
    authentication_id: Uuid,
    options: RequestChallengeResponse,
}

#[derive(Debug, Deserialize)]
struct PasskeyAuthenticationFinishRequest {
    authentication_id: Uuid,
    credential: PublicKeyCredential,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AdminSettings {
    allow_registration: bool,
    oauth_channels: Vec<OAuthChannel>,
    #[serde(default = "default_agent_secret_prefix", alias = "agent_secret")]
    agent_secret_prefix: Option<String>,
    #[serde(default)]
    allow_agent_directory: bool,
    #[serde(default)]
    webhooks: Vec<WebhookConfig>,
}

#[derive(Debug, Serialize)]
struct AdminUpdateStatus {
    current_version: String,
    enabled: bool,
    running: bool,
    timeout_seconds: u64,
}

#[derive(Debug, Serialize)]
struct AdminUpdateResponse {
    ok: bool,
    current_version: String,
    started: bool,
    message: String,
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Default for AdminSettings {
    fn default() -> Self {
        Self {
            allow_registration: true,
            oauth_channels: vec![OAuthChannel {
                provider: "github".to_string(),
                enabled: false,
                client_id: String::new(),
                client_secret: None,
            }],
            agent_secret_prefix: default_agent_secret_prefix(),
            allow_agent_directory: false,
            webhooks: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OAuthChannel {
    provider: String,
    enabled: bool,
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WebhookConfig {
    id: Uuid,
    name: String,
    url: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    secret: Option<String>,
    #[serde(default = "default_webhook_kind")]
    kind: String,
    #[serde(default = "default_webhook_help_prompt")]
    help_prompt: String,
    #[serde(default)]
    weixin_qrcode: Option<String>,
    #[serde(default)]
    weixin_qrcode_url: Option<String>,
    #[serde(default)]
    weixin_status: Option<String>,
    #[serde(default)]
    weixin_status_message: Option<String>,
    #[serde(default)]
    weixin_bot_token: Option<String>,
    #[serde(default)]
    weixin_account_id: Option<String>,
    #[serde(default)]
    weixin_base_url: Option<String>,
    #[serde(default)]
    weixin_user_id: Option<String>,
    #[serde(default)]
    weixin_context_token: Option<String>,
    #[serde(default)]
    weixin_last_request_id: Option<Uuid>,
    #[serde(default)]
    weixin_get_updates_buf: Option<String>,
    #[serde(default)]
    weixin_last_error: Option<String>,
    #[serde(default)]
    weixin_last_seen_at: Option<u64>,
    #[serde(default)]
    weixin_long_poll_timeout_ms: Option<u64>,
    #[serde(default)]
    weixin_api_timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct IncomingMessage {
    source: String,
    sender: String,
    content: String,
    #[serde(default)]
    raw: Value,
}

#[derive(Debug, Deserialize)]
struct WebhooksUpdate {
    webhooks: Vec<WebhookConfig>,
}

#[derive(Debug, Deserialize)]
struct WeixinLoginStatusQuery {
    verify_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WeixinQrCodeResponse {
    qrcode: String,
    qrcode_img_content: String,
}

#[derive(Debug, Deserialize)]
struct WeixinQrStatusResponse {
    status: String,
    #[serde(default)]
    bot_token: Option<String>,
    #[serde(default)]
    ilink_bot_id: Option<String>,
    #[serde(default)]
    ilink_user_id: Option<String>,
    #[serde(default)]
    baseurl: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_webhook_kind() -> String {
    "generic".to_string()
}

fn default_webhook_help_prompt() -> String {
    [
        "直接回复本消息就是回答。",
        "如果问题积压，请引用对应问题回复，系统会优先匹配引用中的 请求ID 或 [humen:短ID]。",
        "网页处理地址：{url}",
        "请求ID：{request_id}",
        "短ID：{short_id}",
    ]
    .join("\n")
}

fn default_agent_secret_prefix() -> Option<String> {
    Some(format!("humen-{}-", random_secret(18)))
}

#[derive(Debug, Deserialize)]
struct ProfileUpdate {
    profile: String,
    #[serde(default)]
    tags: Vec<String>,
    is_public: Option<bool>,
    onboarding_completed: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct AgentSecretUpdate {
    agent_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FriendRequestCreate {
    email: Option<String>,
    #[serde(alias = "friend_code")]
    intro_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateAgentTask {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    steps: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    due_at: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AgentTaskUpdate {
    status: AgentTaskStatus,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentTaskQuery {
    status: Option<AgentTaskStatus>,
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
struct RateHumanRequest {
    #[serde(alias = "email")]
    rated_email: String,
    score: f64,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReportHumanRequest {
    #[serde(alias = "email")]
    reported_email: String,
    reason: String,
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
    agent_secret: Option<String>,
    #[serde(default)]
    intro_code: String,
    #[serde(default)]
    is_public: bool,
    #[serde(default)]
    friends: Vec<String>,
    #[serde(default)]
    friend_requests: Vec<String>,
    #[serde(default)]
    onboarding_completed: bool,
    #[serde(default)]
    ban_expires_at: Option<u64>,
    #[serde(default)]
    active_periods: Vec<ActivePeriod>,
    #[serde(default)]
    passkey_user_id: Option<Uuid>,
    #[serde(default)]
    passkeys: Vec<StoredPasskey>,
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
    #[serde(default)]
    admin_settings: AdminSettings,
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

fn new_user_record(email: impl Into<String>, now: u64, profile: impl Into<String>) -> UserRecord {
    let email = normalize_email(&email.into());
    UserRecord {
        email,
        created_at: now,
        last_login_at: now,
        profile: profile.into(),
        tags: Vec::new(),
        agent_secret: Some(random_secret(24)),
        intro_code: random_intro_code(),
        is_public: false,
        friends: Vec::new(),
        friend_requests: Vec::new(),
        onboarding_completed: false,
        ban_expires_at: None,
        active_periods: Vec::new(),
        passkey_user_id: None,
        passkeys: Vec::new(),
    }
}

fn prepare_user_record(record: &mut UserRecord) {
    record.email = normalize_email(&record.email);
    if normalize_optional_value(record.agent_secret.as_deref()).is_none() {
        record.agent_secret = Some(random_secret(24));
    }
    if record.intro_code.trim().is_empty() {
        record.intro_code = random_intro_code();
    }
    record.friends = normalize_email_list(std::mem::take(&mut record.friends));
    record.friend_requests = normalize_email_list(std::mem::take(&mut record.friend_requests));
    if record.passkeys.iter().any(|passkey| passkey.name.trim().is_empty()) {
        for passkey in &mut record.passkeys {
            if passkey.name.trim().is_empty() {
                passkey.name = "Passkey".to_string();
            }
        }
    }
}

fn open_db(path: &PathBuf) -> anyhow::Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create sqlite directory")?;
    }
    let conn =
        Connection::open(path).with_context(|| format!("open sqlite db {}", path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .context("enable sqlite WAL")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS human_requests (
            id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            request_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            expired_at INTEGER,
            expire_reason TEXT,
            answer_json TEXT,
            answered_at INTEGER,
            answered_late INTEGER NOT NULL DEFAULT 0,
            read_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_human_requests_status ON human_requests(status);
        CREATE INDEX IF NOT EXISTS idx_human_requests_answered_late ON human_requests(answered_late, answered_at);

        CREATE TABLE IF NOT EXISTS agent_tasks (
            id TEXT PRIMARY KEY,
            task_json TEXT NOT NULL,
            created_by TEXT NOT NULL,
            assigned_to TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            due_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_agent_tasks_assigned ON agent_tasks(assigned_to, status, updated_at);
        CREATE INDEX IF NOT EXISTS idx_agent_tasks_created_by ON agent_tasks(created_by, updated_at);

        CREATE TABLE IF NOT EXISTS human_ratings (
            rated_email TEXT NOT NULL,
            rater_email TEXT NOT NULL,
            score REAL NOT NULL,
            note TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (rated_email, rater_email)
        );
        CREATE INDEX IF NOT EXISTS idx_human_ratings_rated ON human_ratings(rated_email);

        CREATE TABLE IF NOT EXISTS human_reports (
            id TEXT PRIMARY KEY,
            reporter_email TEXT NOT NULL,
            reported_email TEXT NOT NULL,
            reason TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'open'
        );
        CREATE INDEX IF NOT EXISTS idx_human_reports_status ON human_reports(status, created_at);
        "#,
    )
    .context("initialize sqlite schema")?;
    Ok(conn)
}

fn load_buffered_requests(state: &AppState) -> anyhow::Result<()> {
    let now = now_unix();
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("sqlite lock poisoned"))?;
    let mut stmt = db.prepare(
        "SELECT request_json, status, expired_at, expire_reason FROM human_requests \
         WHERE status IN ('pending', 'expired')",
    )?;
    let rows = stmt.query_map([], |row| {
        let request_json: String = row.get(0)?;
        let status: String = row.get(1)?;
        let expired_at: Option<u64> = row.get(2)?;
        let reason: Option<String> = row.get(3)?;
        Ok((request_json, status, expired_at, reason))
    })?;
    for row in rows {
        let (request_json, status, expired_at, reason) = row?;
        let request: HumanRequest = serde_json::from_str(&request_json)?;
        if status == "pending" && request.expires_at > now {
            state.requests.insert(request.id, request);
        } else {
            state.trash.insert(
                request.id,
                ExpiredRequest {
                    request,
                    expired_at: expired_at.unwrap_or(now),
                    reason: reason.unwrap_or_else(|| "Human request timed out".to_string()),
                },
            );
        }
    }
    Ok(())
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

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ReadLateRepliesArgs {
    request_id: Option<Uuid>,
    since: Option<u64>,
    #[serde(default)]
    unread_only: bool,
    #[serde(default)]
    mark_read: bool,
    limit: Option<u64>,
}
