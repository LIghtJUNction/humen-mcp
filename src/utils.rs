fn online_emails(state: &AppState) -> HashMap<String, usize> {
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
            counts.insert(normalize_email(&record.email), active_count);
        }
    }
    counts
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
