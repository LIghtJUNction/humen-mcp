fn require_session(state: &AppState, headers: &HeaderMap) -> Result<Session, ApiError> {
    let session = state
        .session_from_headers(headers)
        .ok_or_else(|| ApiError::unauthorized("missing or invalid bearer token"))?;
    ensure_user_allowed(state, &session.user.email)?;
    Ok(session)
}

fn github_oauth_credentials(state: &AppState) -> Result<(String, String), ApiError> {
    if state.github_enabled() {
        return Ok((
            state.config.github_client_id.clone().unwrap_or_default(),
            state
                .config
                .github_client_secret
                .clone()
                .unwrap_or_default(),
        ));
    }
    let settings = state
        .admin_settings
        .lock()
        .map_err(|_| ApiError::internal("settings lock poisoned"))?;
    let channel = settings
        .oauth_channels
        .iter()
        .find(|channel| channel.provider == "github" && channel.enabled)
        .ok_or_else(|| ApiError::bad_request("GitHub OAuth is not configured"))?;
    let client_secret = channel
        .client_secret
        .as_ref()
        .filter(|secret| !secret.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("GitHub OAuth client secret is not configured"))?;
    if channel.client_id.trim().is_empty() {
        return Err(ApiError::bad_request(
            "GitHub OAuth client id is not configured",
        ));
    }
    Ok((channel.client_id.clone(), client_secret.clone()))
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
    } else if let Some(record) = users
        .users
        .values()
        .find(|record| normalize_email(&record.email) == normalize_email(email))
    {
        if record
            .ban_expires_at
            .is_some_and(|expires_at| expires_at > now_unix())
        {
            return Err(ApiError::unauthorized("user is banned"));
        }
    }
    Ok(())
}

fn upsert_github_user(
    state: &AppState,
    github_id: &str,
    login: Option<&str>,
    email: &str,
) -> Result<String, ApiError> {
    validate_email_like_identifier(email)?;
    let login = login.and_then(|value| normalize_optional_value(Some(value)));
    let legacy_email_key = normalize_email(email);
    let legacy_login_key = login.as_deref().map(normalize_email);
    let admin_email = normalize_email(&state.config.admin_email);
    let github_account_key = github_identity_key(github_id);
    let account_key = if legacy_email_key == admin_email
        || legacy_login_key.as_deref() == Some(admin_email.as_str())
    {
        admin_email.clone()
    } else {
        github_account_key.clone()
    };
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let now = now_unix();
    let existing_key = if users.users.contains_key(&account_key) {
        Some(account_key.clone())
    } else if users.users.contains_key(&github_account_key) {
        Some(github_account_key.clone())
    } else if users.users.contains_key(&legacy_email_key) {
        Some(legacy_email_key)
    } else {
        legacy_login_key
            .as_ref()
            .filter(|key| users.users.contains_key(*key))
            .cloned()
    };

    if let Some(existing_key) = existing_key {
        if existing_key != account_key {
            if let Some(record) = users.users.remove(&existing_key) {
                users.users.insert(account_key.clone(), record);
            }
        }
        let record = users
            .users
            .get_mut(&account_key)
            .ok_or_else(|| ApiError::internal("failed to migrate GitHub user"))?;
        prepare_user_record(record);
        record.github_id = Some(github_id.trim().to_string());
        record.login = login;
        record.email = normalize_email(email);
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
        let mut record = new_user_record(email.to_string(), now, String::new());
        record.github_id = Some(github_id.trim().to_string());
        record.login = login;
        users.users.insert(account_key.clone(), record);
    }
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save GitHub user: {err}")))?;
    Ok(account_key)
}

fn user_profiles(
    state: &AppState,
    query: Option<&str>,
    tag: Option<&str>,
) -> Result<Vec<PublicUserProfile>, ApiError> {
    let online = online_emails(state);
    let reputations = db_reputation_map(state)?;
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let query = query
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    let tag = tag.and_then(normalize_tag);
    let admin_email = normalize_email(&state.config.admin_email);
    let has_admin_record = users
        .users
        .values()
        .any(|record| normalize_email(&record.email) == admin_email);
    let mut profiles: Vec<_> = users
        .users
        .values()
        .map(|record| {
            public_profile_from_record_with_online_and_reputation(
                record,
                &admin_email,
                &online,
                reputation_for(&reputations, &profile_user_key(record, &admin_email)),
            )
        })
        .filter(|profile| profile_matches(profile, query.as_deref(), tag.as_deref()))
        .collect();

    if !has_admin_record {
        let admin_profile = synthetic_admin_profile(
            state,
            &online,
            &admin_email,
            reputation_for(&reputations, &admin_email),
        );
        if profile_matches(&admin_profile, query.as_deref(), tag.as_deref()) {
            profiles.push(admin_profile);
        }
    }

    let mut profiles = dedupe_profiles_by_identity(profiles);
    sort_profiles_by_reputation(&mut profiles);
    Ok(profiles)
}

fn visible_user_profiles_for_session(
    state: &AppState,
    viewer_email: &str,
    query: Option<&str>,
    tag: Option<&str>,
) -> Result<Vec<PublicUserProfile>, ApiError> {
    let viewer_email = normalize_email(viewer_email);
    if viewer_email == normalize_email(&state.config.admin_email) {
        return user_profiles(state, query, tag);
    }
    let online = online_emails(state);
    let reputations = db_reputation_map(state)?;
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let query = query
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    let tag = tag.and_then(normalize_tag);
    let admin_email = normalize_email(&state.config.admin_email);
    let viewer_key = canonical_user_key_from_identifier(&users, &viewer_email, &admin_email)
        .unwrap_or_else(|| viewer_email.clone());
    let viewer = users.users.get(&viewer_key);
    let friends = viewer
        .map(|record| normalize_email_list(record.friends.clone()))
        .unwrap_or_default();
    let incoming = viewer
        .map(|record| normalize_email_list(record.friend_requests.clone()))
        .unwrap_or_default();
    let profiles: Vec<_> = users
        .users
        .values()
        .filter(|record| {
            let record_key = profile_user_key(record, &admin_email);
            let is_admin_record = normalize_email(&record.email) == admin_email;
            if is_admin_record && viewer_key != admin_email {
                return false;
            }
            record_key == viewer_key
                || record.visibility == ProfileVisibility::Public
                || friends.iter().any(|friend| friend == &record_key)
        })
        .map(|record| {
            public_profile_from_record_for_viewer(
                record,
                &admin_email,
                &online,
                reputation_for(&reputations, &profile_user_key(record, &admin_email)),
                &viewer_key,
                &friends,
                &incoming,
            )
        })
        .filter(|profile| profile_matches(profile, query.as_deref(), tag.as_deref()))
        .collect();
    let mut profiles = dedupe_profiles_by_identity(profiles);
    sort_profiles_by_reputation(&mut profiles);
    Ok(profiles)
}

fn agent_visible_profiles(
    state: &AppState,
    agent: &AgentContext,
    query: Option<&str>,
    tag: Option<&str>,
) -> Result<Vec<PublicUserProfile>, ApiError> {
    let online = online_emails(state);
    let reputations = db_reputation_map(state)?;
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let query = query
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    let tag = tag.and_then(normalize_tag);
    let admin_email = normalize_email(&state.config.admin_email);
    let agent_key = canonical_user_key_from_identifier(&users, &agent.email, &admin_email)
        .unwrap_or_else(|| normalize_email(&agent.email));
    let viewer = users.users.get(&agent_key);
    let friends = viewer
        .map(|record| normalize_email_list(record.friends.clone()))
        .unwrap_or_default();
    let min_reputation = agent.directory_min_reputation;
    let profiles: Vec<_> = users
        .users
        .values()
        .filter(|record| {
            agent_can_see_record(
                agent,
                record,
                &admin_email,
                &friends,
                reputation_for(&reputations, &profile_user_key(record, &admin_email)).reputation,
                min_reputation,
            )
        })
        .map(|record| {
            public_profile_from_record_with_online_and_reputation(
                record,
                &admin_email,
                &online,
                reputation_for(&reputations, &profile_user_key(record, &admin_email)),
            )
        })
        .filter(|profile| profile_matches(profile, query.as_deref(), tag.as_deref()))
        .collect();
    let mut profiles = dedupe_profiles_by_identity(profiles);
    sort_profiles_by_reputation(&mut profiles);
    Ok(profiles)
}

fn sort_profiles_by_reputation(profiles: &mut [PublicUserProfile]) {
    profiles.sort_by(|left, right| {
        right
            .online
            .cmp(&left.online)
            .then_with(|| {
                right
                    .reputation
                    .partial_cmp(&left.reputation)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| right.ratings_count.cmp(&left.ratings_count))
            .then_with(|| left.email.cmp(&right.email))
    });
}

fn dedupe_profiles_by_identity(profiles: Vec<PublicUserProfile>) -> Vec<PublicUserProfile> {
    let mut by_identity: HashMap<String, PublicUserProfile> = HashMap::new();
    for profile in profiles {
        let key = profile_identity_key(&profile);
        match by_identity.get(&key) {
            Some(current) if profile_rank(current) >= profile_rank(&profile) => {}
            _ => {
                by_identity.insert(key, profile);
            }
        }
    }
    by_identity.into_values().collect()
}

fn profile_identity_key(profile: &PublicUserProfile) -> String {
    if let Some(login) = profile
        .login
        .as_deref()
        .and_then(|value| normalize_optional_value(Some(value)))
    {
        return format!("github-login:{}", login.to_ascii_lowercase());
    }
    normalize_email(&profile.email)
}

fn profile_rank(profile: &PublicUserProfile) -> (u8, u8, u64) {
    (
        u8::from(profile.email.starts_with("github:")),
        u8::from(profile.online),
        profile.last_login_at,
    )
}

fn agent_can_see_record(
    agent: &AgentContext,
    record: &UserRecord,
    admin_email: &str,
    friends: &[String],
    reputation: f64,
    min_reputation: f64,
) -> bool {
    if normalize_email(&record.email) == normalize_email(admin_email) {
        return false;
    }
    let record_key = profile_user_key(record, admin_email);
    if record_key == agent.email {
        return true;
    }
    match agent.directory_visibility {
        AgentDirectoryVisibility::SelfOnly => false,
        AgentDirectoryVisibility::SelfAndFriends => {
            friends.iter().any(|friend| friend == &record_key)
        }
        AgentDirectoryVisibility::PublicUsers => profile_visible_to_agents(record),
        AgentDirectoryVisibility::ReputationAtLeast => {
            profile_visible_to_agents(record) && reputation >= min_reputation
        }
    }
}

fn profile_visible_to_agents(record: &UserRecord) -> bool {
    matches!(
        record.visibility,
        ProfileVisibility::Public | ProfileVisibility::Agents
    )
}

fn visible_tag_counts_for_session(
    state: &AppState,
    viewer_email: &str,
) -> Result<Vec<Value>, ApiError> {
    tag_counts_from_profiles(visible_user_profiles_for_session(
        state,
        viewer_email,
        None,
        None,
    )?)
}

fn agent_visible_tag_counts(
    state: &AppState,
    agent: &AgentContext,
) -> Result<Vec<Value>, ApiError> {
    tag_counts_from_profiles(agent_visible_profiles(state, agent, None, None)?)
}

fn tag_counts_from_profiles(profiles: Vec<PublicUserProfile>) -> Result<Vec<Value>, ApiError> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for profile in profiles {
        for tag in profile.tags {
            if let Some(tag) = normalize_tag(&tag) {
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

fn get_or_create_user_record(state: &AppState, email: &str) -> Result<UserRecord, ApiError> {
    let email = normalize_email(email);
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let now = now_unix();
    let record = users
        .users
        .entry(email.clone())
        .or_insert_with(|| new_user_record(email.clone(), now, default_profile_template(&email)));
    prepare_user_record(record);
    let record = record.clone();
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save user profile: {err}")))?;
    Ok(record)
}

fn ensure_user_agent_fields(
    state: &AppState,
    email: &str,
) -> Result<(String, String, ProfileVisibility, bool), ApiError> {
    let email = normalize_email(email);
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let now = now_unix();
    let record = users
        .users
        .entry(email.clone())
        .or_insert_with(|| new_user_record(email.clone(), now, default_profile_template(&email)));
    prepare_user_record(record);
    let suffix = record.agent_secret.clone().unwrap_or_default();
    let intro_code = record.intro_code.clone();
    let visibility = record.visibility;
    let onboarding_completed = record.onboarding_completed;
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save user agent fields: {err}")))?;
    Ok((suffix, intro_code, visibility, onboarding_completed))
}

fn public_profile_from_record(state: &AppState, record: &UserRecord) -> PublicUserProfile {
    let online = online_emails(state);
    let admin_email = normalize_email(&state.config.admin_email);
    let reputation =
        db_reputation_summary_for(state, &profile_user_key(record, &admin_email)).unwrap_or_default();
    public_profile_from_record_with_online_and_reputation(
        record,
        &admin_email,
        &online,
        reputation,
    )
}

fn public_profile_from_record_with_online_and_reputation(
    record: &UserRecord,
    admin_email: &str,
    online: &HashMap<String, usize>,
    reputation: ReputationSummary,
) -> PublicUserProfile {
    let email = normalize_email(&record.email);
    let is_admin = email == admin_email;
    let account_key = profile_user_key(record, admin_email);
    let visibility = if is_admin {
        ProfileVisibility::Private
    } else {
        record.visibility
    };
    let provider = if is_admin {
        AuthProvider::Password
    } else {
        AuthProvider::Github
    };
    PublicUserProfile {
        email: account_key.clone(),
        login: record.login.clone(),
        provider,
        profile: record.profile.clone(),
        tags: public_tags_for_record(record, is_admin),
        reputation: reputation.reputation,
        ratings_count: reputation.ratings_count,
        reputation_breakdown: reputation.reputation_breakdown,
        friend_code: record.intro_code.clone(),
        intro_code: record.intro_code.clone(),
        visibility,
        is_public: visibility == ProfileVisibility::Public,
        is_friend: false,
        friend_request_sent: false,
        friend_request_received: false,
        onboarding_completed: record.onboarding_completed,
        online: online.contains_key(&online_identity_key(record)),
        last_login_at: record.last_login_at,
        last_seen_at: user_record_last_seen_at(record),
        ban_expires_at: if is_admin {
            None
        } else {
            record.ban_expires_at
        },
    }
}

fn public_profile_from_record_for_viewer(
    record: &UserRecord,
    admin_email: &str,
    online: &HashMap<String, usize>,
    reputation: ReputationSummary,
    viewer_email: &str,
    viewer_friends: &[String],
    viewer_incoming: &[String],
) -> PublicUserProfile {
    let mut profile = public_profile_from_record_with_online_and_reputation(
        record,
        admin_email,
        online,
        reputation,
    );
    let email = normalize_email(&record.email);
    let viewer_email = normalize_email(viewer_email);
    if email != viewer_email {
        profile.is_friend = viewer_friends.iter().any(|friend| friend == &email);
        profile.friend_request_sent = record
            .friend_requests
            .iter()
            .any(|requester| normalize_email(requester) == viewer_email);
        profile.friend_request_received =
            viewer_incoming.iter().any(|requester| requester == &email);
    }
    profile
}

fn public_tags_for_record(record: &UserRecord, is_admin: bool) -> Vec<String> {
    let mut tags = normalize_tags(record.tags.clone());
    if is_admin {
        tags.push(ADMIN_TAG.to_string());
        tags.sort();
        tags.dedup();
    }
    tags
}

fn synthetic_admin_profile(
    state: &AppState,
    online: &HashMap<String, usize>,
    admin_email: &str,
    reputation: ReputationSummary,
) -> PublicUserProfile {
    PublicUserProfile {
        email: state.config.admin_email.clone(),
        login: None,
        provider: AuthProvider::Password,
        profile: "Administrator".to_string(),
        tags: vec![ADMIN_TAG.to_string()],
        reputation: reputation.reputation,
        ratings_count: reputation.ratings_count,
        reputation_breakdown: reputation.reputation_breakdown,
        friend_code: String::new(),
        intro_code: String::new(),
        visibility: ProfileVisibility::Private,
        is_public: false,
        is_friend: false,
        friend_request_sent: false,
        friend_request_received: false,
        onboarding_completed: true,
        online: online.contains_key(admin_email),
        last_login_at: 0,
        last_seen_at: 0,
        ban_expires_at: None,
    }
}

fn user_record_last_seen_at(record: &UserRecord) -> u64 {
    record
        .active_periods
        .iter()
        .filter_map(|period| period.disconnected_at.or(Some(period.connected_at)))
        .max()
        .unwrap_or(record.last_login_at)
        .max(record.last_login_at)
}

fn reputation_for(
    reputations: &HashMap<String, ReputationSummary>,
    email: &str,
) -> ReputationSummary {
    reputations
        .get(&normalize_email(email))
        .cloned()
        .unwrap_or_default()
}

fn profile_matches(profile: &PublicUserProfile, query: Option<&str>, tag: Option<&str>) -> bool {
    let query_matches = query.is_none_or(|query| {
        profile.email.to_ascii_lowercase().contains(query)
            || profile.profile.to_ascii_lowercase().contains(query)
            || profile
                .tags
                .iter()
                .any(|tag| tag.to_ascii_lowercase().contains(query))
    });
    let tag_matches = tag.is_none_or(|tag| profile.tags.iter().any(|candidate| candidate == tag));
    query_matches && tag_matches
}

fn default_profile_template(email: &str) -> String {
    format!(
        "Hi, I am {email}.\n\nSkills: #review #ops\nAvailable for: short human-in-the-loop checks\nNotes: timezone, language, escalation preferences"
    )
}

fn public_oauth_channels(channels: &[OAuthChannel]) -> Vec<OAuthChannel> {
    channels
        .iter()
        .cloned()
        .map(|mut channel| {
            channel.client_secret = None;
            channel
        })
        .collect()
}

fn persist_admin_settings(state: &AppState, settings: &AdminSettings) -> Result<(), ApiError> {
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    users.admin_settings = settings.clone();
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save admin settings: {err}")))
}

fn oauth_channel_enabled(settings: &AdminSettings, provider: &str) -> bool {
    let provider = normalize_oauth_provider(provider);
    settings.oauth_channels.iter().any(|channel| {
        channel.provider == provider
            && channel.enabled
            && non_empty(Some(channel.client_id.as_str()))
            && non_empty(channel.client_secret.as_deref())
    })
}

fn sanitize_admin_settings(mut settings: AdminSettings) -> AdminSettings {
    settings.github_api_token = normalize_optional_value(settings.github_api_token.as_deref());
    settings.github_api_token_configured = settings.github_api_token.is_some();
    settings.agent_secret_prefix =
        normalize_optional_value(settings.agent_secret_prefix.as_deref())
            .or_else(default_agent_secret_prefix);
    if !settings.agent_directory_min_reputation.is_finite() {
        settings.agent_directory_min_reputation = default_agent_directory_min_reputation();
    }
    settings.agent_directory_min_reputation =
        settings.agent_directory_min_reputation.clamp(0.0, 10.0);
    settings.allow_agent_directory = settings.agent_directory_visibility.allows_non_self();
    for channel in &mut settings.oauth_channels {
        channel.provider = normalize_oauth_provider(&channel.provider);
        channel.client_id = channel.client_id.trim().to_string();
        channel.client_secret = channel
            .client_secret
            .as_deref()
            .map(str::trim)
            .filter(|secret| !secret.is_empty())
            .map(str::to_string);
    }
    settings
        .oauth_channels
        .retain(|channel| !channel.provider.is_empty());
    for webhook in &mut settings.webhooks {
        webhook.name = webhook.name.trim().to_string();
        webhook.url = webhook.url.trim().to_string();
        webhook.assigned_to = normalize_optional_value(webhook.assigned_to.as_deref())
            .map(|email| normalize_email(&email));
        webhook.secret = normalize_optional_value(webhook.secret.as_deref());
        webhook.kind = normalize_webhook_kind(&webhook.kind);
        webhook.help_prompt = webhook.help_prompt.trim().to_string();
        webhook.weixin_qrcode = normalize_optional_value(webhook.weixin_qrcode.as_deref());
        webhook.weixin_qrcode_url = normalize_optional_value(webhook.weixin_qrcode_url.as_deref());
        webhook.weixin_status = normalize_optional_value(webhook.weixin_status.as_deref());
        webhook.weixin_status_message =
            normalize_optional_value(webhook.weixin_status_message.as_deref());
        webhook.weixin_bot_token = normalize_optional_value(webhook.weixin_bot_token.as_deref());
        webhook.weixin_account_id = normalize_optional_value(webhook.weixin_account_id.as_deref());
        webhook.weixin_base_url = normalize_optional_value(webhook.weixin_base_url.as_deref());
        webhook.weixin_user_id = normalize_optional_value(webhook.weixin_user_id.as_deref());
        webhook.weixin_context_token =
            normalize_optional_value(webhook.weixin_context_token.as_deref());
        webhook.weixin_get_updates_buf =
            normalize_optional_value(webhook.weixin_get_updates_buf.as_deref());
        webhook.weixin_last_error = normalize_optional_value(webhook.weixin_last_error.as_deref());
    }
    settings
        .webhooks
        .retain(|webhook| !webhook.url.is_empty() || webhook.kind == "wechat");
    settings
}

fn merge_admin_secret_settings(
    state: &AppState,
    settings: &mut AdminSettings,
) -> Result<(), ApiError> {
    let current_token = state
        .admin_settings
        .lock()
        .map_err(|_| ApiError::internal("settings lock poisoned"))?
        .github_api_token
        .clone();
    settings.github_api_token = match settings.github_api_token.as_deref() {
        None => current_token,
        Some(value) if value.trim().is_empty() => None,
        Some(value) => Some(value.trim().to_string()),
    };
    Ok(())
}

fn admin_settings_response(state: &AppState, mut settings: AdminSettings) -> AdminSettings {
    settings.github_api_token_configured = effective_github_api_token(state).is_some();
    settings.github_api_token = None;
    settings
}

fn normalize_webhook_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "wechat" => "wechat".to_string(),
        _ => "generic".to_string(),
    }
}

fn normalize_oauth_provider(provider: &str) -> String {
    provider
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn normalize_optional_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn require_agent_access(state: &AppState, headers: &HeaderMap) -> Result<AgentContext, ApiError> {
    let settings = state
        .admin_settings
        .lock()
        .map_err(|_| ApiError::internal("settings lock poisoned"))?
        .clone();
    let prefix = normalize_optional_value(settings.agent_secret_prefix.as_deref())
        .ok_or_else(|| ApiError::unauthorized("agent secret prefix is not configured"))?;
    let provided = provided_agent_secret(headers)
        .ok_or_else(|| ApiError::unauthorized("agent secret is required for this MCP server"))?;
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    for record in users.users.values_mut() {
        prepare_user_record(record);
        let Some(suffix) = normalize_optional_value(record.agent_secret.as_deref()) else {
            continue;
        };
        if provided == format!("{prefix}{suffix}") {
            let email = canonical_user_key_from_record(record);
            drop(users);
            ensure_user_allowed(state, &email)?;
            return Ok(AgentContext {
                email,
                agent_id: String::new(),
                agent_name: String::new(),
                directory_visibility: settings.agent_directory_visibility,
                directory_min_reputation: settings.agent_directory_min_reputation,
            });
        }
    }
    Err(ApiError::unauthorized("missing or invalid agent secret"))
}

fn provided_agent_secret(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-humen-agent-secret")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .map(str::to_string)
        })
}

fn mcp_public_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/mcp") {
        base.to_string()
    } else {
        format!("{base}/mcp")
    }
}

#[cfg(test)]
fn tag_counts(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let admin_email = normalize_email(&state.config.admin_email);
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut has_admin_record = false;
    for record in users.users.values() {
        let is_admin = normalize_email(&record.email) == admin_email;
        has_admin_record |= is_admin;
        for tag in public_tags_for_record(record, is_admin) {
            *counts.entry(tag).or_default() += 1;
        }
    }
    if !has_admin_record {
        *counts.entry(ADMIN_TAG.to_string()).or_default() += 1;
    }
    let mut tags: Vec<_> = counts
        .into_iter()
        .map(|(tag, count)| json!({ "tag": tag, "count": count }))
        .collect();
    tags.sort_by_key(|item| item["tag"].as_str().unwrap_or_default().to_string());
    Ok(tags)
}
