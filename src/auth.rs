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
) -> Result<Redirect, ApiError> {
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

    let email = user
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| user.get("login").and_then(Value::as_str))
        .ok_or_else(|| ApiError::upstream("GitHub user response had no email or login"))?;
    let email = normalize_email(email);
    upsert_github_user(&state, &email)?;
    match github_reputation_seed_for_oauth_user(&state, &user, access_token).await {
        Ok(seed) => {
            db_upsert_reputation_seed(&state, &email, github_seed_as_reputation_seed(&seed))?;
        }
        Err(err) => {
            warn!(email, error = %err.message, "failed to compute GitHub reputation seed");
        }
    }
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
        let record = users
            .users
            .entry(email.clone())
            .or_insert_with(|| new_user_record(email.clone(), now, String::new()));
        prepare_user_record(record);
        record.profile = payload.profile;
        record.tags = normalize_tags(payload.tags);
        if let Some(is_public) = payload.is_public {
            record.is_public = is_public;
        }
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
    let (suffix, intro_code, is_public, onboarding_completed) =
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
        "is_public": is_public,
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
        let record = users
            .users
            .entry(normalize_email(&session.user.email))
            .or_insert_with(|| {
                new_user_record(
                    session.user.email.clone(),
                    now,
                    default_profile_template(&session.user.email),
                )
            });
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
    require_admin(&state, &headers)?;
    let settings = state
        .admin_settings
        .lock()
        .map_err(|_| ApiError::internal("settings lock poisoned"))?
        .clone();
    Ok(Json(settings.webhooks))
}

async fn admin_update_webhooks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<WebhooksUpdate>,
) -> Result<Json<Vec<WebhookConfig>>, ApiError> {
    require_admin(&state, &headers)?;
    let mut settings = state
        .admin_settings
        .lock()
        .map_err(|_| ApiError::internal("settings lock poisoned"))?
        .clone();
    settings.webhooks = payload.webhooks;
    let sanitized = sanitize_admin_settings(settings);
    {
        let mut stored = state
            .admin_settings
            .lock()
            .map_err(|_| ApiError::internal("settings lock poisoned"))?;
        *stored = sanitized.clone();
    }
    persist_admin_settings(&state, &sanitized)?;
    Ok(Json(sanitized.webhooks))
}
