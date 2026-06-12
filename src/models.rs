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
    #[serde(default)]
    created_by: Option<String>,
    #[serde(default)]
    created_by_agent_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TaskKind {
    Choice,
    Judgment,
    #[default]
    Text,
    ImageReview,
    Steps,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    #[serde(default, alias = "human_email", alias = "assigned_to")]
    target_human_email: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct FederationRegistry {
    #[serde(default)]
    nodes: Vec<FederationNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FederationNode {
    node_id: String,
    endpoint: String,
    agent_secret: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "default_federation_trust_level")]
    trust_level: String,
    #[serde(default = "default_federation_max_hops")]
    max_hops: u8,
}

#[derive(Clone, Debug, Serialize)]
struct FederationNodeSummary {
    node_id: String,
    endpoint: String,
    enabled: bool,
    description: String,
    tags: Vec<String>,
    trust_level: String,
    max_hops: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FederatedRequest {
    local_request_id: Uuid,
    origin_agent_email: String,
    target_node_id: String,
    remote_request_id: Uuid,
    path: Vec<String>,
    status: FederatedRequestStatus,
    created_at: u64,
    expires_at: u64,
    updated_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FederationLedgerEntry {
    sequence: u64,
    node_id: String,
    event_type: String,
    subject_id: String,
    previous_hash: String,
    event_hash: String,
    event_json: Value,
    created_at: u64,
}

#[derive(Clone, Debug, Serialize)]
struct FederationLedgerHead {
    sequence: u64,
    event_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FederatedRequestStatus {
    Pending,
    Answered,
    Expired,
    Failed,
}

impl FederatedRequestStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Answered => "answered",
            Self::Expired => "expired",
            Self::Failed => "failed",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "answered" => Self::Answered,
            "expired" => Self::Expired,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NetworkAskHumanRequest {
    #[serde(flatten)]
    request: CreateHumanRequest,
    #[serde(default)]
    target_node_id: Option<String>,
    #[serde(default)]
    route_tags: Vec<String>,
    #[serde(default = "default_federation_max_hops")]
    hop_limit: u8,
    #[serde(default)]
    path: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct NetworkSearchArgs {
    q: Option<String>,
    tag: Option<String>,
    #[serde(default)]
    include_local: bool,
}

#[derive(Debug, Deserialize)]
struct ReadNetworkLedgerArgs {
    limit: Option<u64>,
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
    platform_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    login: Option<String>,
    requests_handled: u64,
    sent_tokens: u64,
    latest_answered_at: Option<u64>,
    reputation: f64,
    ratings_count: u64,
    reputation_breakdown: ReputationBreakdown,
    profile: String,
    tags: Vec<String>,
    online: bool,
    online_sources: Vec<OnlineSource>,
}

#[derive(Clone, Debug)]
struct HumanLeaderboardStat {
    email: String,
    requests_handled: u64,
    sent_tokens: u64,
    latest_answered_at: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AgentTaskStatus {
    #[default]
    Open,
    InProgress,
    Done,
    Archived,
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
    reputation_breakdown: ReputationBreakdown,
}

impl Default for ReputationSummary {
    fn default() -> Self {
        Self {
            reputation: 5.0,
            ratings_count: 0,
            reputation_breakdown: ReputationBreakdown::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ReputationBreakdown {
    seed_source: Option<String>,
    seed_score: Option<f64>,
    seed_weight: f64,
    feedback_weight: f64,
    total_weight: f64,
    confidence: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReputationSeed {
    source: String,
    score: f64,
    weight: f64,
    details: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GithubAccountSnapshot {
    login: String,
    account_created_at: Option<String>,
    public_repos: u64,
    public_gists: u64,
    followers: u64,
    following: u64,
    total_stars_sampled: u64,
    source_repos_sampled: u64,
    fork_repos_sampled: u64,
    recent_events_sampled: u64,
    recent_activity_year: Option<i64>,
    fetched_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GithubReputationSeed {
    score: f64,
    weight: f64,
    snapshot: GithubAccountSnapshot,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HumanMemo {
    id: Uuid,
    target_email: String,
    author_email: String,
    #[serde(default)]
    author_agent_id: Option<String>,
    #[serde(default)]
    author_agent_name: Option<String>,
    body: String,
    created_at: u64,
    read_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
struct HumanMemoUnreadSource {
    author_email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author_agent_name: Option<String>,
    count: u64,
    latest_at: u64,
}

#[derive(Clone, Debug, Serialize)]
struct HumanMemoUnreadSummary {
    total: u64,
    sources: Vec<HumanMemoUnreadSource>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ConnectedAgent {
    id: String,
    owner_email: String,
    owner_platform_name: String,
    name: String,
    description: String,
    current_task: String,
    last_tool: String,
    first_seen_at: u64,
    last_seen_at: u64,
    last_request_at: Option<u64>,
    request_count: u64,
    reputation: f64,
    ratings_count: u64,
    reputation_breakdown: ReputationBreakdown,
    online: bool,
    relation_status: AgentRelationStatus,
    pending_messages: Vec<AgentHumanMessage>,
}

#[derive(Clone, Debug, Serialize)]
struct PublicConnectedAgent {
    owner_platform_name: String,
    name: String,
    description: String,
    current_task: String,
    last_tool: String,
    last_seen_at: u64,
    last_request_at: Option<u64>,
    request_count: u64,
    reputation: f64,
    ratings_count: u64,
    online: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AgentRelationStatus {
    #[default]
    None,
    HumanRequested,
    AgentRequested,
    Friends,
}

impl AgentRelationStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::HumanRequested => "human_requested",
            Self::AgentRequested => "agent_requested",
            Self::Friends => "friends",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "human_requested" => Self::HumanRequested,
            "agent_requested" => Self::AgentRequested,
            "friends" => Self::Friends,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AgentHumanMessage {
    id: Uuid,
    agent_id: String,
    human_email: String,
    direction: String,
    kind: String,
    body: String,
    status: String,
    created_at: u64,
    resolved_at: Option<u64>,
    read_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
struct PublicUserProfile {
    email: String,
    platform_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    login: Option<String>,
    provider: AuthProvider,
    profile: String,
    tags: Vec<String>,
    reputation: f64,
    ratings_count: u64,
    reputation_breakdown: ReputationBreakdown,
    friend_code: String,
    intro_code: String,
    visibility: ProfileVisibility,
    is_public: bool,
    is_friend: bool,
    friend_request_sent: bool,
    friend_request_received: bool,
    onboarding_completed: bool,
    online: bool,
    online_sources: Vec<OnlineSource>,
    last_login_at: u64,
    last_seen_at: u64,
    ban_expires_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum OnlineSource {
    Web,
    Wechat,
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
    MemoCreated {
        memo: HumanMemo,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Session {
    user: User,
    created_at: u64,
}

#[derive(Clone, Debug)]
struct AgentContext {
    email: String,
    agent_id: String,
    agent_name: String,
    directory_visibility: AgentDirectoryVisibility,
    directory_min_reputation: f64,
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AgentDirectoryVisibility {
    PublicUsers,
    ReputationAtLeast,
    SelfAndFriends,
    SelfOnly,
}

impl AgentDirectoryVisibility {
    fn allows_non_self(self) -> bool {
        !matches!(self, Self::SelfOnly)
    }
}

#[derive(Clone, Debug, Serialize)]
struct AdminSettings {
    allow_registration: bool,
    oauth_channels: Vec<OAuthChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    github_api_token: Option<String>,
    #[serde(default)]
    github_api_token_configured: bool,
    #[serde(default = "default_agent_secret_prefix", alias = "agent_secret")]
    agent_secret_prefix: Option<String>,
    #[serde(default)]
    allow_agent_directory: bool,
    #[serde(default = "default_agent_directory_visibility")]
    agent_directory_visibility: AgentDirectoryVisibility,
    #[serde(default = "default_agent_directory_min_reputation")]
    agent_directory_min_reputation: f64,
    #[serde(default)]
    webhooks: Vec<WebhookConfig>,
}

#[derive(Debug, Deserialize)]
struct RawAdminSettings {
    #[serde(default = "default_true")]
    allow_registration: bool,
    #[serde(default = "default_oauth_channels")]
    oauth_channels: Vec<OAuthChannel>,
    #[serde(default)]
    github_api_token: Option<String>,
    #[serde(default)]
    github_api_token_configured: bool,
    #[serde(default = "default_agent_secret_prefix", alias = "agent_secret")]
    agent_secret_prefix: Option<String>,
    #[serde(default)]
    allow_agent_directory: Option<bool>,
    #[serde(default)]
    agent_directory_visibility: Option<AgentDirectoryVisibility>,
    #[serde(default = "default_agent_directory_min_reputation")]
    agent_directory_min_reputation: f64,
    #[serde(default)]
    webhooks: Vec<WebhookConfig>,
}

impl<'de> Deserialize<'de> for AdminSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawAdminSettings::deserialize(deserializer)?;
        let agent_directory_visibility =
            match (raw.agent_directory_visibility, raw.allow_agent_directory) {
                (Some(visibility), Some(allow_agent_directory))
                    if visibility.allows_non_self() == allow_agent_directory =>
                {
                    visibility
                }
                (Some(_), Some(allow_agent_directory)) | (None, Some(allow_agent_directory)) => {
                    legacy_agent_directory_visibility(allow_agent_directory)
                }
                (Some(visibility), None) => visibility,
                (None, None) => default_agent_directory_visibility(),
            };
        Ok(Self {
            allow_registration: raw.allow_registration,
            oauth_channels: raw.oauth_channels,
            github_api_token: raw.github_api_token,
            github_api_token_configured: raw.github_api_token_configured,
            agent_secret_prefix: raw.agent_secret_prefix,
            allow_agent_directory: agent_directory_visibility.allows_non_self(),
            agent_directory_visibility,
            agent_directory_min_reputation: raw.agent_directory_min_reputation,
            webhooks: raw.webhooks,
        })
    }
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
            oauth_channels: default_oauth_channels(),
            github_api_token: None,
            github_api_token_configured: false,
            agent_secret_prefix: default_agent_secret_prefix(),
            allow_agent_directory: false,
            agent_directory_visibility: default_agent_directory_visibility(),
            agent_directory_min_reputation: default_agent_directory_min_reputation(),
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
    assigned_to: Option<String>,
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

fn default_federation_trust_level() -> String {
    "limited".to_string()
}

fn default_federation_max_hops() -> u8 {
    3
}

fn default_oauth_channels() -> Vec<OAuthChannel> {
    vec![OAuthChannel {
        provider: "github".to_string(),
        enabled: false,
        client_id: String::new(),
        client_secret: None,
    }]
}

fn default_agent_directory_visibility() -> AgentDirectoryVisibility {
    AgentDirectoryVisibility::SelfOnly
}

fn legacy_agent_directory_visibility(allow_agent_directory: bool) -> AgentDirectoryVisibility {
    if allow_agent_directory {
        AgentDirectoryVisibility::PublicUsers
    } else {
        default_agent_directory_visibility()
    }
}

fn default_agent_directory_min_reputation() -> f64 {
    5.0
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
    #[serde(default)]
    visibility: Option<ProfileVisibility>,
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
struct RateAgentRequest {
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
struct CreateHumanMemo {
    body: String,
}

#[derive(Debug, Deserialize)]
struct LeaveHumanMemoArgs {
    #[serde(alias = "email", alias = "human_email")]
    target_human_email: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct AgentPanelMessageCreate {
    #[serde(default)]
    body: String,
}

#[derive(Debug, Deserialize)]
struct AgentFriendRequestArgs {
    #[serde(alias = "email")]
    human_email: String,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct ListAgentInboxArgs {
    #[serde(default)]
    unread_only: bool,
    #[serde(default)]
    mark_read: bool,
    limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AgentAskMeArgs {
    #[serde(default)]
    body: String,
    #[serde(default, alias = "title")]
    title: String,
    #[serde(default, alias = "prompt")]
    prompt: String,
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
    platform_name: String,
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    github_id: Option<String>,
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
    visibility: ProfileVisibility,
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

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProfileVisibility {
    #[default]
    Private,
    Friends,
    Agents,
    Public,
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
            Ok(raw) => {
                let mut store: Self = serde_json::from_str(&raw).context("parse users file")?;
                let mut rekeyed = HashMap::new();
                for mut record in store.users.into_values() {
                    prepare_user_record(&mut record);
                    let key = user_record_key(&record);
                    if rekeyed.insert(key.clone(), record).is_some() {
                        anyhow::bail!("duplicate user key after platform migration: {key}");
                    }
                }
                store.users = rekeyed;
                validate_unique_platform_names(&store.users)?;
                Ok(store)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).context("read users file"),
        }
    }

    fn save(&self, path: &PathBuf) -> anyhow::Result<()> {
        validate_unique_platform_names(&self.users)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create users file directory")?;
        }
        let raw = serde_json::to_string_pretty(self).context("serialize users file")?;
        fs::write(path, raw).context("write users file")
    }

    fn insert(&mut self, record: UserRecord) {
        let mut record = record;
        prepare_user_record(&mut record);
        self.users.insert(user_record_key(&record), record);
    }
}

fn new_user_record(email: impl Into<String>, now: u64, profile: impl Into<String>) -> UserRecord {
    let email = normalize_email(&email.into());
    UserRecord {
        email,
        platform_name: String::new(),
        created_at: now,
        last_login_at: now,
        login: None,
        github_id: None,
        profile: profile.into(),
        tags: Vec::new(),
        agent_secret: Some(random_secret(24)),
        intro_code: random_intro_code(),
        visibility: ProfileVisibility::Private,
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
    record.login = record
        .login
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    record.github_id = record
        .github_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    record.platform_name = normalize_platform_name(&record.platform_name)
        .or_else(|| {
            record
                .login
                .as_deref()
                .and_then(normalize_platform_name)
        })
        .or_else(|| {
            record
                .email
                .split('@')
                .next()
                .and_then(normalize_platform_name)
        })
        .unwrap_or_else(|| format!("user-{}", random_secret(8).to_ascii_lowercase()));
    if record.is_public && record.visibility == ProfileVisibility::Private {
        record.visibility = ProfileVisibility::Public;
    }
    record.is_public = record.visibility == ProfileVisibility::Public;
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

fn user_record_key(record: &UserRecord) -> String {
    record
        .github_id
        .as_deref()
        .map(github_identity_key)
        .unwrap_or_else(|| normalize_email(&record.email))
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

        CREATE TABLE IF NOT EXISTS human_request_hides (
            user_email TEXT NOT NULL,
            request_id TEXT NOT NULL,
            hidden_at INTEGER NOT NULL,
            PRIMARY KEY (user_email, request_id)
        );
        CREATE INDEX IF NOT EXISTS idx_human_request_hides_user ON human_request_hides(user_email, hidden_at);

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
            weight REAL NOT NULL DEFAULT 1.0,
            note TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (rated_email, rater_email)
        );
        CREATE INDEX IF NOT EXISTS idx_human_ratings_rated ON human_ratings(rated_email);

        CREATE TABLE IF NOT EXISTS agent_ratings (
            agent_id TEXT NOT NULL,
            rater_email TEXT NOT NULL,
            score REAL NOT NULL,
            weight REAL NOT NULL DEFAULT 1.0,
            note TEXT,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (agent_id, rater_email)
        );
        CREATE INDEX IF NOT EXISTS idx_agent_ratings_agent ON agent_ratings(agent_id);

        CREATE TABLE IF NOT EXISTS reputation_seeds (
            email TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            score REAL NOT NULL,
            weight REAL NOT NULL,
            details_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_reputation_seeds_source ON reputation_seeds(source, updated_at);

        CREATE TABLE IF NOT EXISTS github_account_cache (
            login TEXT PRIMARY KEY,
            account_json TEXT NOT NULL,
            fetched_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_github_account_cache_expires ON github_account_cache(expires_at);

        CREATE TABLE IF NOT EXISTS human_reports (
            id TEXT PRIMARY KEY,
            reporter_email TEXT NOT NULL,
            reported_email TEXT NOT NULL,
            reason TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'open'
        );
        CREATE INDEX IF NOT EXISTS idx_human_reports_status ON human_reports(status, created_at);

        CREATE TABLE IF NOT EXISTS human_memos (
            id TEXT PRIMARY KEY,
            target_email TEXT NOT NULL,
            author_email TEXT NOT NULL,
            author_agent_id TEXT,
            author_agent_name TEXT,
            body TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            read_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_human_memos_target ON human_memos(target_email, created_at);
        CREATE INDEX IF NOT EXISTS idx_human_memos_author ON human_memos(author_email, created_at);

        CREATE TABLE IF NOT EXISTS agent_connections (
            id TEXT PRIMARY KEY,
            owner_email TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            current_task TEXT NOT NULL,
            last_tool TEXT NOT NULL,
            first_seen_at INTEGER NOT NULL,
            last_seen_at INTEGER NOT NULL,
            last_request_at INTEGER,
            request_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_agent_connections_owner ON agent_connections(owner_email, last_seen_at);
        CREATE INDEX IF NOT EXISTS idx_agent_connections_seen ON agent_connections(last_seen_at);

        CREATE TABLE IF NOT EXISTS agent_relations (
            agent_id TEXT NOT NULL,
            human_email TEXT NOT NULL,
            status TEXT NOT NULL,
            human_message TEXT,
            agent_message TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (agent_id, human_email)
        );
        CREATE INDEX IF NOT EXISTS idx_agent_relations_human ON agent_relations(human_email, status, updated_at);

        CREATE TABLE IF NOT EXISTS agent_human_messages (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            human_email TEXT NOT NULL,
            direction TEXT NOT NULL,
            kind TEXT NOT NULL,
            body TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            resolved_at INTEGER,
            read_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_agent_messages_agent ON agent_human_messages(agent_id, status, created_at);
        CREATE INDEX IF NOT EXISTS idx_agent_messages_human ON agent_human_messages(human_email, status, created_at);

        CREATE TABLE IF NOT EXISTS web_sessions (
            token_hash TEXT PRIMARY KEY,
            session_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_web_sessions_expires ON web_sessions(expires_at);

        CREATE TABLE IF NOT EXISTS federated_requests (
            local_request_id TEXT PRIMARY KEY,
            origin_agent_email TEXT NOT NULL,
            target_node_id TEXT NOT NULL,
            remote_request_id TEXT NOT NULL,
            path_json TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_federated_requests_origin ON federated_requests(origin_agent_email, status, updated_at);
        CREATE INDEX IF NOT EXISTS idx_federated_requests_remote ON federated_requests(target_node_id, remote_request_id);

        CREATE TABLE IF NOT EXISTS federation_ledger (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            node_id TEXT NOT NULL,
            event_type TEXT NOT NULL,
            subject_id TEXT NOT NULL,
            previous_hash TEXT NOT NULL,
            event_hash TEXT NOT NULL,
            event_json TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_federation_ledger_subject ON federation_ledger(subject_id, sequence);
        CREATE INDEX IF NOT EXISTS idx_federation_ledger_hash ON federation_ledger(event_hash);
        "#,
    )
    .context("initialize sqlite schema")?;
    migrate_reputation_schema(&conn).context("migrate sqlite schema")?;
    Ok(conn)
}

fn migrate_reputation_schema(conn: &Connection) -> anyhow::Result<()> {
    if !sqlite_table_has_column(conn, "human_ratings", "weight")? {
        conn.execute(
            "ALTER TABLE human_ratings ADD COLUMN weight REAL NOT NULL DEFAULT 1.0",
            [],
        )
        .context("add human_ratings.weight column")?;
    }
    if !sqlite_table_has_column(conn, "human_memos", "read_at")? {
        conn.execute("ALTER TABLE human_memos ADD COLUMN read_at INTEGER", [])
            .context("add human_memos.read_at column")?;
    }
    if !sqlite_table_has_column(conn, "human_memos", "author_agent_id")? {
        conn.execute("ALTER TABLE human_memos ADD COLUMN author_agent_id TEXT", [])
            .context("add human_memos.author_agent_id column")?;
    }
    if !sqlite_table_has_column(conn, "human_memos", "author_agent_name")? {
        conn.execute("ALTER TABLE human_memos ADD COLUMN author_agent_name TEXT", [])
            .context("add human_memos.author_agent_name column")?;
    }
    if !sqlite_table_has_column(conn, "agent_human_messages", "read_at")? {
        conn.execute("ALTER TABLE agent_human_messages ADD COLUMN read_at INTEGER", [])
            .context("add agent_human_messages.read_at column")?;
    }
    Ok(())
}

fn sqlite_table_has_column(conn: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("inspect sqlite table {table}"))?;
    let mut rows = stmt.query([]).with_context(|| format!("query sqlite table {table}"))?;
    while let Some(row) = rows.next().context("iterate sqlite table columns")? {
        let name: String = row
            .get(1)
            .with_context(|| format!("read sqlite table column name for {table}"))?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
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
