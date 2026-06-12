fn online_web_emails(state: &AppState) -> HashMap<String, usize> {
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
            let key = online_identity_key(record);
            *counts.entry(key).or_insert(0) += active_count;
        }
    }
    counts
}

fn online_presence_sources(state: &AppState) -> HashMap<String, HashSet<OnlineSource>> {
    let mut sources: HashMap<String, HashSet<OnlineSource>> = online_web_emails(state)
        .into_keys()
        .map(|email| (email, HashSet::from([OnlineSource::Web])))
        .collect();
    let webhooks = state
        .admin_settings
        .lock()
        .map(|settings| settings.webhooks.clone())
        .unwrap_or_default();
    for webhook in webhooks {
        if normalize_webhook_kind(&webhook.kind) != "wechat"
            || !webhook.enabled
            || normalize_optional_value(webhook.weixin_bot_token.as_deref()).is_none()
        {
            continue;
        }
        let Some(assigned_to) = normalize_optional_value(webhook.assigned_to.as_deref()) else {
            continue;
        };
        let key = canonical_user_key_from_email(state, &assigned_to);
        sources
            .entry(key)
            .or_default()
            .insert(OnlineSource::Wechat);
    }
    sources
}

fn online_identity_key(record: &UserRecord) -> String {
    if let Some(login) = record
        .login
        .as_deref()
        .and_then(|value| normalize_optional_value(Some(value)))
    {
        return format!("github-login:{}", login.to_ascii_lowercase());
    }
    canonical_user_key_from_record(record)
}

fn canonical_user_key_from_record(record: &UserRecord) -> String {
    if let Some(github_id) = record
        .github_id
        .as_deref()
        .and_then(|value| normalize_optional_value(Some(value)))
    {
        return github_identity_key(&github_id);
    }
    normalize_email(&record.email)
}

fn profile_user_key(record: &UserRecord, admin_email: &str) -> String {
    let email = normalize_email(&record.email);
    if email == normalize_email(admin_email) {
        email
    } else {
        canonical_user_key_from_record(record)
    }
}

fn canonical_user_key_from_email(state: &AppState, email: &str) -> String {
    let wanted = normalize_email(email);
    if wanted == normalize_email(&state.config.admin_email) {
        return wanted;
    }
    let Ok(users) = state.users.lock() else {
        return wanted;
    };
    canonical_user_key_from_identifier(&users, &wanted, &state.config.admin_email)
        .unwrap_or(wanted)
}

fn same_user_identity(state: &AppState, left: &str, right: &str) -> bool {
    canonical_user_key_from_email(state, left) == canonical_user_key_from_email(state, right)
}

fn canonical_user_key_from_identifier(
    users: &UserStore,
    identifier: &str,
    admin_email: &str,
) -> Option<String> {
    let wanted = normalize_email(identifier);
    if wanted.is_empty() {
        return None;
    }
    if wanted == normalize_email(admin_email) {
        return Some(wanted);
    }
    users
        .users
        .get(&wanted)
        .map(canonical_user_key_from_record)
        .or_else(|| {
            users
                .users
                .values()
                .find(|record| {
                    normalize_email(&record.email) == wanted
                        || record
                            .github_id
                            .as_deref()
                            .is_some_and(|github_id| github_identity_key(github_id) == wanted)
                        || (!record.platform_name.trim().is_empty()
                            && normalize_platform_name(&record.platform_name)
                                .is_some_and(|value| value == wanted))
                        || record
                            .login
                            .as_deref()
                            .is_some_and(|login| normalize_email(login) == wanted)
                })
                .map(canonical_user_key_from_record)
        })
}

fn validate_unique_platform_names(users: &HashMap<String, UserRecord>) -> anyhow::Result<()> {
    let mut seen = HashSet::new();
    for record in users.values() {
        let Some(platform_name) = normalize_platform_name(&platform_name_for_record(record)) else {
            continue;
        };
        if !seen.insert(platform_name.clone()) {
            anyhow::bail!("duplicate platform name: {platform_name}");
        }
    }
    Ok(())
}

fn clear_stale_active_periods(users: &mut UserStore) {
    let now = now_unix();
    for record in users.users.values_mut() {
        for period in &mut record.active_periods {
            if period.disconnected_at.is_none() {
                period.disconnected_at = Some(now);
                period.duration_seconds = Some(now.saturating_sub(period.connected_at));
            }
        }
    }
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut normalized: Vec<_> = tags
        .into_iter()
        .filter_map(|tag| normalize_tag(&tag))
        .filter(|tag| !is_reserved_tag(tag))
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

fn is_reserved_tag(tag: &str) -> bool {
    tag.eq_ignore_ascii_case(ADMIN_TAG)
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

fn normalize_image_payload(
    image_base64: Option<String>,
    image_mime_type: Option<String>,
) -> (Option<String>, Option<String>) {
    let Some(raw) = image_base64.map(|value| value.trim().to_string()) else {
        return (None, None);
    };
    if raw.is_empty() {
        return (None, None);
    }

    if let Some(data_url) = raw.strip_prefix("data:") {
        if let Some((metadata, data)) = data_url.split_once(',') {
            let metadata_lower = metadata.to_ascii_lowercase();
            if metadata_lower.ends_with(";base64") {
                let mime = metadata[..metadata.len().saturating_sub(";base64".len())].trim();
                return (
                    Some(strip_base64_whitespace(data)),
                    Some(normalize_image_mime_type(Some(mime))),
                );
            }
        }
    }

    (
        Some(strip_base64_whitespace(&raw)),
        Some(normalize_image_mime_type(image_mime_type.as_deref())),
    )
}

fn normalize_image_mime_type(value: Option<&str>) -> String {
    let mime = value.unwrap_or("image/png").trim().to_ascii_lowercase();
    if mime.starts_with("image/")
        && mime
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '+' | '-'))
    {
        mime
    } else {
        "image/png".to_string()
    }
}

fn strip_base64_whitespace(value: &str) -> String {
    value.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn normalize_platform_name(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return None;
    }
    let mut normalized = String::with_capacity(value.len());
    let mut prev_dash = false;
    for ch in value.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch)
        } else if matches!(ch, '-' | '_' | '.') {
            Some('-')
        } else {
            None
        };
        let Some(mapped) = mapped else {
            continue;
        };
        if mapped == '-' {
            if prev_dash || normalized.is_empty() {
                prev_dash = true;
                continue;
            }
            prev_dash = true;
            normalized.push(mapped);
            continue;
        }
        prev_dash = false;
        normalized.push(mapped);
    }
    while normalized.ends_with('-') {
        normalized.pop();
    }
    if normalized.len() < 2 {
        return None;
    }
    Some(normalized.chars().take(32).collect())
}

fn platform_name_for_record(record: &UserRecord) -> String {
    let raw = record.platform_name.trim();
    if !raw.is_empty() {
        return raw.to_string();
    }
    if let Some(login) = record.login.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        return login.to_string();
    }
    if let Some(local_part) = record
        .email
        .split('@')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return local_part.to_string();
    }
    record.email.clone()
}

fn normalize_email_list(values: Vec<String>) -> Vec<String> {
    let mut values: Vec<_> = values
        .into_iter()
        .map(|value| normalize_email(&value))
        .filter(|value| !value.is_empty())
        .collect();
    values.sort();
    values.dedup();
    values
}

fn validate_email_like_identifier(email: &str) -> Result<(), ApiError> {
    if email.len() < 2 || email.contains(char::is_whitespace) {
        return Err(ApiError::bad_request("valid GitHub identity is required"));
    }
    Ok(())
}

fn github_identity_key(id: &str) -> String {
    format!("github:{}", id.trim())
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

fn normalize_memo_body(body: &str) -> Result<String, ApiError> {
    let normalized = body.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request("memo body is required"));
    }
    if trimmed.chars().count() > MEMO_BODY_MAX_CHARS {
        return Err(ApiError::bad_request(format!(
            "memo body must be at most {MEMO_BODY_MAX_CHARS} characters"
        )));
    }
    if trimmed.lines().count() > MEMO_BODY_MAX_LINES {
        return Err(ApiError::bad_request(format!(
            "memo body must be at most {MEMO_BODY_MAX_LINES} lines"
        )));
    }
    Ok(trimmed.to_string())
}

fn normalize_optional_memo_body(body: &str) -> Result<Option<String>, ApiError> {
    if body.trim().is_empty() {
        Ok(None)
    } else {
        normalize_memo_body(body).map(Some)
    }
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

fn random_intro_code() -> String {
    format!("hm-{}", random_secret(8).to_ascii_lowercase())
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
