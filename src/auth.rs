async fn auth_config(State(state): State<AppState>) -> Json<AuthConfigResponse> {
    let settings = state
        .admin_settings
        .lock()
        .map(|settings| settings.clone())
        .unwrap_or_default();
    Json(AuthConfigResponse {
        github_enabled: oauth_channel_enabled(&settings, "github") || state.github_enabled(),
        passkey_enabled: state.webauthn.is_some(),
        allow_registration: settings.allow_registration,
        oauth_channels: public_oauth_channels(&settings.oauth_channels),
    })
}

async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    if let Some(user) = authenticate_password(&state, &payload.email, &payload.pass)? {
        ensure_user_allowed(&state, &user.email)?;
        let auth = state.create_session(user.email, AuthProvider::Password);
        Ok(auth_json_response(&state, &auth))
    } else {
        Err(ApiError::unauthorized("invalid email or password"))
    }
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = session_token_from_headers(&headers) {
        state.destroy_session_token(token);
    }
    let mut response = Json(json!({ "ok": true })).into_response();
    append_auth_cookie(&mut response, &state, None);
    response
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
    let (client_id, _) = github_oauth_credentials(&state)?;
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
) -> Result<Response, ApiError> {
    let (client_id, client_secret) = github_oauth_credentials(&state)?;

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

    let github_id = user
        .get("id")
        .and_then(Value::as_u64)
        .map(|id| id.to_string())
        .or_else(|| {
            user.get("node_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| ApiError::upstream("GitHub user response had no stable id"))?;
    let login = user
        .get("login")
        .and_then(Value::as_str)
        .map(str::to_string);
    let email = user
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| user.get("login").and_then(Value::as_str))
        .ok_or_else(|| ApiError::upstream("GitHub user response had no email or login"))?;
    let email = normalize_email(email);
    let account_key = upsert_github_user(&state, &github_id, login.as_deref(), &email)?;
    match github_reputation_seed_for_oauth_user(&state, &user, access_token).await {
        Ok(seed) => {
            db_upsert_reputation_seed(&state, &account_key, github_seed_as_reputation_seed(&seed))?;
        }
        Err(err) => {
            warn!(email, github_id, error = %err.message, "failed to compute GitHub reputation seed");
        }
    }
    ensure_user_allowed(&state, &account_key)?;
    let auth = state.create_session(account_key, AuthProvider::Github);
    let redirect = format!("{}/", state.config.public_base_url.trim_end_matches('/'));
    let mut response = Redirect::temporary(&redirect).into_response();
    append_auth_cookie(&mut response, &state, Some(&auth.token));
    Ok(response)
}

const AUTH_COOKIE_NAME: &str = "humen-mcp-token";
const WEB_SESSION_TTL_SECONDS: u64 = 60 * 60 * 24 * 30;

fn auth_json_response(state: &AppState, auth: &AuthResponse) -> Response {
    let mut response = Json(auth).into_response();
    append_auth_cookie(&mut response, state, Some(&auth.token));
    response
}

fn append_auth_cookie(response: &mut Response, state: &AppState, token: Option<&str>) {
    let cookie = auth_cookie_value(state, token);
    if let Ok(value) = cookie.parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

fn auth_cookie_value(state: &AppState, token: Option<&str>) -> String {
    let path = auth_cookie_path(&state.config.public_base_url);
    let secure = if state.config.public_base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    match token {
        Some(token) => format!(
            "{AUTH_COOKIE_NAME}={token}; Max-Age={WEB_SESSION_TTL_SECONDS}; Path={path}; HttpOnly; SameSite=Lax{secure}"
        ),
        None => format!(
            "{AUTH_COOKIE_NAME}=; Max-Age=0; Path={path}; HttpOnly; SameSite=Lax{secure}"
        ),
    }
}

fn auth_cookie_path(public_base_url: &str) -> &'static str {
    let Some(after_scheme) = public_base_url.split_once("://").map(|(_, rest)| rest) else {
        return "/";
    };
    let path = after_scheme
        .split_once('/')
        .map(|(_, path)| format!("/{path}"))
        .unwrap_or_else(|| "/".to_string());
    if path == "/mcp" || path.starts_with("/mcp/") {
        "/mcp"
    } else {
        "/"
    }
}

fn session_token_from_headers(headers: &HeaderMap) -> Option<&str> {
    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    {
        return Some(token);
    }
    session_cookie_token(headers)
}

fn session_cookie_token(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == AUTH_COOKIE_NAME && !value.is_empty()).then_some(value)
    })
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let profile = get_or_create_user_record(&state, &session.user.email)?;
    Ok(Json(json!({
        "user": session.user,
        "profile": public_profile_from_record(&state, &profile),
        "created_at": session.created_at
    })))
}

async fn me_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<PublicUserProfile>, ApiError> {
    let session = require_session(&state, &headers)?;
    let record = get_or_create_user_record(&state, &session.user.email)?;
    Ok(Json(public_profile_from_record(&state, &record)))
}

async fn update_me_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ProfileUpdate>,
) -> Result<Json<PublicUserProfile>, ApiError> {
    let session = require_session(&state, &headers)?;
    let email = normalize_email(&session.user.email);
    let record = {
        let mut users = state
            .users
            .lock()
            .map_err(|_| ApiError::internal("user store lock poisoned"))?;
        let now = now_unix();
        let admin_email = normalize_email(&state.config.admin_email);
        let key = canonical_user_key_from_identifier(&users, &email, &admin_email).unwrap_or_else(|| {
            let mut record = new_user_record(email.clone(), now, String::new());
            prepare_user_record(&mut record);
            let key = user_record_key(&record);
            users.users.insert(key.clone(), record);
            key
        });
        let record = users
            .users
            .get_mut(&key)
            .ok_or_else(|| ApiError::internal("failed to create user profile"))?;
        prepare_user_record(record);
        record.profile = payload.profile;
        record.tags = normalize_tags(payload.tags);
        if let Some(visibility) = payload.visibility {
            record.visibility = visibility;
        } else if let Some(is_public) = payload.is_public {
            record.visibility = if is_public {
                ProfileVisibility::Public
            } else {
                ProfileVisibility::Private
            };
        }
        record.is_public = record.visibility == ProfileVisibility::Public;
        if let Some(onboarding_completed) = payload.onboarding_completed {
            record.onboarding_completed = onboarding_completed;
        }
        let record = record.clone();
        users
            .save(&state.config.users_file)
            .map_err(|err| ApiError::internal(format!("failed to save profile: {err}")))?;
        record
    };
    Ok(Json(public_profile_from_record(&state, &record)))
}

async fn agent_access(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let settings = state
        .admin_settings
        .lock()
        .map_err(|_| ApiError::internal("settings lock poisoned"))?
        .clone();
    let prefix = normalize_optional_value(settings.agent_secret_prefix.as_deref())
        .ok_or_else(|| ApiError::internal("agent secret prefix is not configured"))?;
    let (suffix, intro_code, visibility, onboarding_completed) =
        ensure_user_agent_fields(&state, &session.user.email)?;
    let mcp_url = mcp_public_url(&state.config.public_base_url);
    Ok(Json(json!({
        "user": session.user.email,
        "mcp_url": mcp_url,
        "secret_required": true,
        "agent_secret_prefix": prefix,
        "user_agent_secret": suffix,
        "agent_secret": format!("{}{}", prefix, suffix),
        "allow_agent_directory": settings.allow_agent_directory,
        "agent_directory_visibility": settings.agent_directory_visibility,
        "agent_directory_min_reputation": settings.agent_directory_min_reputation,
        "friend_code": intro_code.clone(),
        "intro_code": intro_code,
        "visibility": visibility,
        "is_public": visibility == ProfileVisibility::Public,
        "onboarding_completed": onboarding_completed
    })))
}

async fn update_agent_secret(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AgentSecretUpdate>,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let suffix = normalize_optional_value(payload.agent_secret.as_deref())
        .unwrap_or_else(|| random_secret(24));
    if suffix.len() < 12 {
        return Err(ApiError::bad_request(
            "agent secret must be at least 12 characters",
        ));
    }
    {
        let mut users = state
            .users
            .lock()
            .map_err(|_| ApiError::internal("user store lock poisoned"))?;
        let now = now_unix();
        let session_email = normalize_email(&session.user.email);
        let admin_email = normalize_email(&state.config.admin_email);
        let key = canonical_user_key_from_identifier(&users, &session_email, &admin_email).unwrap_or_else(|| {
            let mut record = new_user_record(
                session.user.email.clone(),
                now,
                default_profile_template(&session.user.email),
            );
            prepare_user_record(&mut record);
            let key = user_record_key(&record);
            users.users.insert(key.clone(), record);
            key
        });
        let record = users
            .users
            .get_mut(&key)
            .ok_or_else(|| ApiError::internal("failed to create user agent secret"))?;
        prepare_user_record(record);
        record.agent_secret = Some(suffix);
        users
            .save(&state.config.users_file)
            .map_err(|err| ApiError::internal(format!("failed to save agent secret: {err}")))?;
    }
    agent_access(State(state), headers).await
}

async fn admin_webhooks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<WebhookConfig>>, ApiError> {
    let session = require_session(&state, &headers)?;
    let is_admin = normalize_email(&session.user.email) == normalize_email(&state.config.admin_email);
    let settings = state
        .admin_settings
        .lock()
        .map_err(|_| ApiError::internal("settings lock poisoned"))?
        .clone();
    Ok(Json(webhooks_visible_to_session(
        &state,
        &session.user.email,
        is_admin,
        &settings.webhooks,
    )))
}

async fn admin_update_webhooks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<WebhooksUpdate>,
) -> Result<Json<Vec<WebhookConfig>>, ApiError> {
    let session = require_session(&state, &headers)?;
    let is_admin = normalize_email(&session.user.email) == normalize_email(&state.config.admin_email);
    let current = state
        .admin_settings
        .lock()
        .map_err(|_| ApiError::internal("settings lock poisoned"))?
        .clone();
    let sanitized = merge_webhooks_for_session(
        &state,
        &session.user.email,
        is_admin,
        current,
        payload.webhooks,
    )?;
    {
        let mut stored = state
            .admin_settings
            .lock()
            .map_err(|_| ApiError::internal("settings lock poisoned"))?;
        *stored = sanitized.clone();
    }
    persist_admin_settings(&state, &sanitized)?;
    Ok(Json(webhooks_visible_to_session(
        &state,
        &session.user.email,
        is_admin,
        &sanitized.webhooks,
    )))
}
