fn parse_weixin_message(raw: Value) -> IncomingMessage {
    let sender = first_string(
        &raw,
        &[
            "sender",
            "from",
            "from_user",
            "fromUser",
            "from_user_id",
            "fromUserId",
            "from_user_name",
            "fromUserName",
            "from_username",
            "fromUsername",
            "wxid",
            "user",
            "user_id",
            "userId",
            "nickname",
            "remark",
        ],
    )
    .unwrap_or_else(|| "wechat".to_string());
    let content = first_string(
        &raw,
        &[
            "content",
            "text",
            "message",
            "msg",
            "body",
            "text_content",
            "textContent",
            "push_content",
            "pushContent",
            "msg_content",
            "msgContent",
        ],
    )
    .unwrap_or_else(|| serde_json::to_string(&raw).unwrap_or_default());
    IncomingMessage {
        source: "wechat".to_string(),
        sender,
        content,
        raw,
    }
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = value.get(*key).and_then(Value::as_str) {
            let found = found.trim();
            if !found.is_empty() {
                return Some(found.to_string());
            }
        }
    }
    if let Some(object) = value.as_object() {
        for nested in object.values() {
            if nested.is_object() || nested.is_array() {
                if let Some(found) = first_string(nested, keys) {
                    return Some(found);
                }
            }
        }
    }
    if let Some(array) = value.as_array() {
        for nested in array {
            if let Some(found) = first_string(nested, keys) {
                return Some(found);
            }
        }
    }
    None
}

fn weixin_base_info() -> Value {
    json!({
        "channel_version": "1.0.0"
    })
}

fn weixin_message_context_token(raw: &Value) -> Option<String> {
    first_string(raw, &["context_token", "contextToken"])
}

fn weixin_message_user_id(raw: &Value) -> Option<String> {
    first_string(
        raw,
        &[
            "from_user_id",
            "fromUserId",
            "from",
            "from_user",
            "user_id",
            "userId",
            "sender",
        ],
    )
}

fn weixin_message_is_from_bot(raw: &Value, webhook: &WebhookConfig) -> bool {
    let Some(account_id) = normalize_optional_value(webhook.weixin_account_id.as_deref()) else {
        return false;
    };
    weixin_message_user_id(raw).is_some_and(|sender| normalize_email(&sender) == normalize_email(&account_id))
}

fn find_webhook(state: &AppState, id: Uuid) -> Option<WebhookConfig> {
    state
        .admin_settings
        .lock()
        .ok()?
        .webhooks
        .iter()
        .find(|webhook| webhook.id == id)
        .cloned()
}

fn update_webhook_config(
    state: &AppState,
    id: Uuid,
    mutate: impl FnOnce(&mut WebhookConfig),
) -> Result<WebhookConfig, ApiError> {
    let sanitized = {
        let mut settings = state
            .admin_settings
            .lock()
            .map_err(|_| ApiError::internal("settings lock poisoned"))?
            .clone();
        let webhook = settings
            .webhooks
            .iter_mut()
            .find(|webhook| webhook.id == id)
            .ok_or_else(|| ApiError::bad_request("webhook not found"))?;
        mutate(webhook);
        sanitize_admin_settings(settings)
    };
    let updated = sanitized
        .webhooks
        .iter()
        .find(|webhook| webhook.id == id)
        .cloned()
        .ok_or_else(|| ApiError::bad_request("webhook not found"))?;
    {
        let mut stored = state
            .admin_settings
            .lock()
            .map_err(|_| ApiError::internal("settings lock poisoned"))?;
        *stored = sanitized.clone();
    }
    persist_admin_settings(state, &sanitized)?;
    Ok(updated)
}

fn ensure_weixin_webhook(webhook: &WebhookConfig) -> Result<(), ApiError> {
    if normalize_webhook_kind(&webhook.kind) == "wechat" {
        Ok(())
    } else {
        Err(ApiError::bad_request("webhook is not a Weixin integration"))
    }
}

fn weixin_qrcode_data_uri(content: &str) -> Result<String, ApiError> {
    let code = QrCode::new(content.as_bytes()).map_err(|err| {
        ApiError::upstream(format!("Weixin QR code could not be rendered: {err}"))
    })?;
    let image = code
        .render::<svg::Color>()
        .min_dimensions(280, 280)
        .dark_color(svg::Color("#111827"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        BASE64_STANDARD.encode(image.as_bytes())
    ))
}

fn weixin_api_url(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn weixin_api_base_url(webhook: &WebhookConfig) -> String {
    normalize_optional_value(webhook.weixin_base_url.as_deref())
        .unwrap_or_else(|| WEIXIN_DEFAULT_BASE_URL.to_string())
}

fn weixin_api_timeout_ms(webhook: &WebhookConfig) -> u64 {
    webhook
        .weixin_api_timeout_ms
        .unwrap_or(WEIXIN_DEFAULT_API_TIMEOUT_MS)
        .clamp(3_000, 60_000)
}

fn weixin_poll_timeout_ms(webhook: &WebhookConfig) -> u64 {
    webhook
        .weixin_long_poll_timeout_ms
        .unwrap_or(WEIXIN_DEFAULT_POLL_TIMEOUT_MS)
        .clamp(5_000, 120_000)
}

fn weixin_uin_header() -> String {
    let random: u32 = rand::rng().random();
    BASE64_STANDARD.encode(random.to_string().as_bytes())
}

fn dispatch_webhooks(
    state: &AppState,
    event: &'static str,
    source: &'static str,
    request: &HumanRequest,
    raw: Option<Value>,
) {
    let webhooks = state
        .admin_settings
        .lock()
        .map(|settings| settings.webhooks.clone())
        .unwrap_or_default();
    let request = request.clone();
    let raw = raw.clone();
    for webhook in webhooks.into_iter().filter(|webhook| webhook.enabled) {
        if normalize_webhook_kind(&webhook.kind) == "wechat" {
            if event == "request_created" && source != "wechat" {
                let state = state.clone();
                let request = request.clone();
                tokio::spawn(async move {
                    if let Err(err) =
                        send_weixin_request_notification(state.clone(), webhook.clone(), &request)
                            .await
                    {
                        warn!(
                            webhook_id = %webhook.id,
                            error = %err.message,
                            "Weixin notification delivery failed"
                        );
                        let message = err.message;
                        let _ = update_webhook_config(&state, webhook.id, |stored| {
                            stored.weixin_last_error = Some(message);
                        });
                    }
                });
            }
            continue;
        }
        if !non_empty(Some(&webhook.url)) {
            continue;
        }
        let http = state.http.clone();
        let help_prompt = render_webhook_help_prompt(state, &webhook, &request);
        let payload = json!({
            "event": event,
            "source": source,
            "webhook_id": webhook.id,
            "request": request,
            "help_prompt": help_prompt,
            "raw": raw,
            "sent_at": now_unix()
        });
        tokio::spawn(async move {
            let mut req = http.post(webhook.url.trim()).json(&payload);
            if let Some(secret) = normalize_optional_value(webhook.secret.as_deref()) {
                req = req.header("x-humen-webhook-secret", secret);
            }
            match req.send().await {
                Ok(response) if response.status().is_success() => {}
                Ok(response) => warn!(
                    webhook_id = %webhook.id,
                    status = %response.status(),
                    "webhook returned non-success status"
                ),
                Err(err) => warn!(webhook_id = %webhook.id, %err, "webhook delivery failed"),
            }
        });
    }
}

async fn send_weixin_request_notification(
    state: AppState,
    webhook: WebhookConfig,
    request: &HumanRequest,
) -> Result<(), ApiError> {
    let token = normalize_optional_value(webhook.weixin_bot_token.as_deref())
        .ok_or_else(|| ApiError::bad_request("Weixin bot token is missing"))?;
    let to_user_id = normalize_optional_value(webhook.weixin_user_id.as_deref())
        .ok_or_else(|| ApiError::bad_request("Weixin target user id is missing"))?;
    let context_token = normalize_optional_value(webhook.weixin_context_token.as_deref()).ok_or_else(|| {
        ApiError::bad_request(
            "Weixin context_token is missing; send one message to this bot first, then retry the MCP request",
        )
    })?;
    let text = format_weixin_request_notification(&state, &webhook, request);
    let body = json!({
        "base_info": weixin_base_info(),
        "msg": {
            "from_user_id": "",
            "to_user_id": to_user_id,
            "client_id": format!("humen-mcp-{}-{}", request.id, now_unix()),
            "message_type": 2,
            "message_state": 2,
            "context_token": context_token,
            "item_list": [{
                "type": 1,
                "text_item": {
                    "text": text
                }
            }]
        }
    });
    let response = state
        .http
        .post(weixin_api_url(
            &weixin_api_base_url(&webhook),
            "/ilink/bot/sendmessage",
        ))
        .header("AuthorizationType", "ilink_bot_token")
        .header("Authorization", format!("Bearer {token}"))
        .header("X-WECHAT-UIN", weixin_uin_header())
        .timeout(Duration::from_millis(weixin_api_timeout_ms(&webhook)))
        .json(&body)
        .send()
        .await
        .map_err(|err| ApiError::upstream(format!("Weixin sendmessage request failed: {err}")))?;
    let status = response.status();
    let body_text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ApiError::upstream(format!(
            "Weixin sendmessage returned {status}: {body_text}"
        )));
    }
    if let Some(raw) = parse_optional_json(&body_text) {
        let ret = raw
            .get("ret")
            .and_then(Value::as_i64)
            .or_else(|| raw.get("errcode").and_then(Value::as_i64))
            .unwrap_or(0);
        if ret == -14 {
            update_webhook_config(&state, webhook.id, |stored| {
                stored.enabled = false;
                stored.weixin_status = Some("expired".to_string());
                stored.weixin_status_message = Some("微信登录态已失效，请重新扫码登录".to_string());
                stored.weixin_bot_token = None;
                stored.weixin_context_token = None;
                stored.weixin_last_request_id = None;
                stored.weixin_get_updates_buf = None;
                stored.weixin_last_error = Some("Weixin bot token expired".to_string());
            })?;
            return Ok(());
        }
        if ret != 0 {
            return Err(ApiError::upstream(format!(
                "Weixin sendmessage returned ret={ret}: {}",
                serde_json::to_string(&raw).unwrap_or_default()
            )));
        }
    }
    update_webhook_config(&state, webhook.id, |stored| {
        stored.weixin_status = Some("confirmed".to_string());
        stored.weixin_status_message = Some("已发送 MCP 请求通知到微信".to_string());
        stored.weixin_last_request_id = Some(request.id);
        stored.weixin_last_error = None;
    })?;
    Ok(())
}

fn parse_optional_json(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        serde_json::from_str(trimmed).ok()
    }
}

fn format_weixin_request_notification(
    state: &AppState,
    webhook: &WebhookConfig,
    request: &HumanRequest,
) -> String {
    let request_id = request.id.to_string();
    let short_id = request_short_id(request.id);
    let mut lines = vec![
        format!("[humen:{short_id}] 有新的 Agent 请求"),
        format!("请求ID：{request_id}"),
        format!("标题：{}", request.title),
        format!("类型：{}", task_kind_label(&request.kind)),
        format!("内容：{}", truncate_text(&request.prompt, 700)),
    ];
    if !request.choices.is_empty() {
        lines.push(format!("选项：{}", request.choices.join(" / ")));
    }
    if !request.steps.is_empty() {
        lines.push(format!("步骤：{}", truncate_text(&request.steps.join("；"), 500)));
    }
    let mut sections = vec![lines.join("\n")];
    if let Some(help_prompt) = render_webhook_help_prompt(state, webhook, request) {
        sections.push(help_prompt);
    }
    truncate_text(&sections.join("\n\n"), 1800)
}

fn render_webhook_help_prompt(
    state: &AppState,
    webhook: &WebhookConfig,
    request: &HumanRequest,
) -> Option<String> {
    let help_prompt = webhook.help_prompt.trim();
    if help_prompt.is_empty() {
        return None;
    }
    Some(
        help_prompt
            .replace("{url}", &mcp_public_url(&state.config.public_base_url))
            .replace("{request_id}", &request.id.to_string())
            .replace("{short_id}", &request_short_id(request.id))
            .replace("{title}", &request.title),
    )
}

fn request_short_id(id: Uuid) -> String {
    id.to_string().chars().take(8).collect()
}

fn task_kind_label(kind: &TaskKind) -> &'static str {
    match kind {
        TaskKind::Choice => "选择",
        TaskKind::Judgment => "判断",
        TaskKind::Text => "文本",
        TaskKind::ImageReview => "图片审阅",
        TaskKind::Steps => "步骤",
    }
}

fn truncate_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut output: String = value.chars().take(limit.saturating_sub(3)).collect();
    output.push_str("...");
    output
}

fn answer_weixin_message(
    state: &AppState,
    webhook: &WebhookConfig,
    incoming: &IncomingMessage,
) -> Result<Option<AnsweredRequest>, ApiError> {
    let answer_text = incoming.content.trim();
    if answer_text.is_empty() {
        return Ok(None);
    }
    let Some(request_id) = resolve_weixin_answer_target(state, webhook, incoming)? else {
        return Ok(None);
    };
    if !is_answerable_request(state, request_id)? {
        return Ok(None);
    }
    let answer = HumanAnswer {
        answer: answer_text.to_string(),
        note: Some(format!("来自微信：{}", incoming.sender)),
        answered_by: format!("wechat:{}", incoming.sender),
        answered_at: now_unix(),
    };
    answer_request_internal(state, request_id, None, answer).map(Some)
}

fn resolve_weixin_answer_target(
    state: &AppState,
    webhook: &WebhookConfig,
    incoming: &IncomingMessage,
) -> Result<Option<Uuid>, ApiError> {
    let mut strings = vec![incoming.content.as_str()];
    collect_json_strings(&incoming.raw, &mut strings);
    if let Some(id) = strings.iter().find_map(|text| find_uuid_in_text(text)) {
        return Ok(Some(id));
    }
    let candidates = candidate_answer_requests(state);
    for text in &strings {
        let text = text.to_ascii_lowercase();
        if let Some(request) = candidates
            .iter()
            .find(|request| text.contains(&request_short_id(request.id)))
        {
            return Ok(Some(request.id));
        }
    }
    if let Some(id) = webhook.weixin_last_request_id {
        if is_answerable_request(state, id)? {
            return Ok(Some(id));
        }
    }
    Ok(candidates.first().map(|request| request.id))
}

fn candidate_answer_requests(state: &AppState) -> Vec<HumanRequest> {
    let now = now_unix();
    let mut requests: Vec<_> = state
        .requests
        .iter()
        .filter(|entry| {
            let request = entry.value();
            request.assigned_to.is_some()
                && now <= request.expires_at
                && !request
                    .tags
                    .iter()
                    .any(|tag| tag == "#wechat" || tag == "#weixin")
        })
        .map(|entry| entry.value().clone())
        .collect();
    requests.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    requests
}

fn is_answerable_request(state: &AppState, id: Uuid) -> Result<bool, ApiError> {
    if state.requests.contains_key(&id) || state.trash.contains_key(&id) {
        return Ok(true);
    }
    Ok(db_get_request(state, id)?.is_some_and(|(_, status)| status != "answered"))
}

fn collect_json_strings<'a>(value: &'a Value, output: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => output.push(text),
        Value::Array(items) => {
            for item in items {
                collect_json_strings(item, output);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_json_strings(item, output);
            }
        }
        _ => {}
    }
}

fn find_uuid_in_text(text: &str) -> Option<Uuid> {
    for (start, _) in text.char_indices() {
        let Some(candidate) = text.get(start..start.saturating_add(36)) else {
            continue;
        };
        if candidate.len() == 36 && looks_like_uuid(candidate) {
            if let Ok(id) = Uuid::parse_str(candidate) {
                return Some(id);
            }
        }
    }
    None
}

fn looks_like_uuid(value: &str) -> bool {
    value.chars().enumerate().all(|(index, ch)| match index {
        8 | 13 | 18 | 23 => ch == '-',
        _ => ch.is_ascii_hexdigit(),
    })
}

async fn weixin_poll_loop(state: AppState) {
    let mut shutdown = state.shutdown.subscribe();
    loop {
        let webhooks = state
            .admin_settings
            .lock()
            .map(|settings| {
                settings
                    .webhooks
                    .iter()
                    .filter(|webhook| {
                        webhook.enabled
                            && normalize_webhook_kind(&webhook.kind) == "wechat"
                            && normalize_optional_value(webhook.weixin_bot_token.as_deref())
                                .is_some()
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if webhooks.is_empty() {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                _ = shutdown.recv() => return,
            }
            continue;
        }

        let mut handles = webhooks
            .into_iter()
            .map(|webhook| {
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(err) = poll_weixin_updates_once(state.clone(), webhook.clone()).await
                    {
                        warn!(
                            webhook_id = %webhook.id,
                            error = %err.message,
                            "Weixin polling failed"
                        );
                        let message = err.message;
                        let _ = update_webhook_config(&state, webhook.id, |stored| {
                            stored.weixin_last_error = Some(message);
                        });
                    }
                })
            })
            .collect::<Vec<_>>();
        while let Some(mut handle) = handles.pop() {
            tokio::select! {
                _ = shutdown.recv() => {
                    handle.abort();
                    for handle in handles {
                        handle.abort();
                    }
                    return;
                }
                _ = &mut handle => {}
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            _ = shutdown.recv() => return,
        }
    }
}

async fn poll_weixin_updates_once(state: AppState, webhook: WebhookConfig) -> Result<(), ApiError> {
    let token = normalize_optional_value(webhook.weixin_bot_token.as_deref())
        .ok_or_else(|| ApiError::bad_request("Weixin bot token is missing"))?;
    let timeout_ms = weixin_poll_timeout_ms(&webhook);
    let body = json!({
        "base_info": weixin_base_info(),
        "get_updates_buf": webhook.weixin_get_updates_buf.clone().unwrap_or_default()
    });
    let response = state
        .http
        .post(weixin_api_url(
            &weixin_api_base_url(&webhook),
            "/ilink/bot/getupdates",
        ))
        .header("AuthorizationType", "ilink_bot_token")
        .header("Authorization", format!("Bearer {token}"))
        .header("X-WECHAT-UIN", weixin_uin_header())
        .timeout(Duration::from_millis(timeout_ms.saturating_add(5_000)))
        .json(&body)
        .send()
        .await
        .map_err(|err| ApiError::upstream(format!("Weixin getupdates request failed: {err}")))?;
    if !response.status().is_success() {
        return Err(ApiError::upstream(format!(
            "Weixin getupdates returned {}",
            response.status()
        )));
    }
    let raw: Value = response.json().await.map_err(|err| {
        ApiError::upstream(format!("Weixin getupdates response was invalid: {err}"))
    })?;

    let ret = raw
        .get("ret")
        .and_then(Value::as_i64)
        .or_else(|| raw.get("errcode").and_then(Value::as_i64))
        .unwrap_or(0);
        if ret == -14 {
            update_webhook_config(&state, webhook.id, |stored| {
                stored.enabled = false;
                stored.weixin_status = Some("expired".to_string());
                stored.weixin_status_message = Some("微信登录态已失效，请重新扫码登录".to_string());
                stored.weixin_bot_token = None;
                stored.weixin_context_token = None;
                stored.weixin_last_request_id = None;
                stored.weixin_get_updates_buf = None;
                stored.weixin_last_error = Some("Weixin bot token expired".to_string());
            })?;
            return Ok(());
    }
    if ret != 0 {
        return Err(ApiError::upstream(format!(
            "Weixin getupdates returned ret={ret}: {}",
            serde_json::to_string(&raw).unwrap_or_default()
        )));
    }

    let messages = raw
        .get("msgs")
        .or_else(|| raw.get("messages"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut latest_context_token = None;
    let mut latest_user_id = None;
    for message in &messages {
        latest_context_token = weixin_message_context_token(message).or(latest_context_token);
        latest_user_id = weixin_message_user_id(message).or(latest_user_id);
        if weixin_message_is_from_bot(message, &webhook) {
            continue;
        }
        let incoming = parse_weixin_message(message.clone());
        match answer_weixin_message(&state, &webhook, &incoming) {
            Ok(Some(answered)) => {
                dispatch_webhooks(
                    &state,
                    "message_answered",
                    "wechat",
                    &answered.request,
                    Some(message.clone()),
                );
            }
            Ok(None) => {
                let request = create_incoming_request(&state, &incoming);
                dispatch_webhooks(
                    &state,
                    "message_received",
                    "wechat",
                    &request,
                    Some(message.clone()),
                );
            }
            Err(err) => {
                warn!(
                    webhook_id = %webhook.id,
                    error = %err.message,
                    "Weixin reply could not be applied as an answer"
                );
                let request = create_incoming_request(&state, &incoming);
                dispatch_webhooks(
                    &state,
                    "message_received",
                    "wechat",
                    &request,
                    Some(message.clone()),
                );
            }
        }
    }

    let next_buf = raw
        .get("get_updates_buf")
        .or_else(|| raw.get("getUpdatesBuf"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let next_timeout = raw
        .get("longpolling_timeout_ms")
        .or_else(|| raw.get("longpollingTimeoutMs"))
        .and_then(Value::as_u64);
    update_webhook_config(&state, webhook.id, |stored| {
        if let Some(next_buf) = next_buf.clone() {
            stored.weixin_get_updates_buf = Some(next_buf);
        }
        if let Some(next_timeout) = next_timeout {
            stored.weixin_long_poll_timeout_ms = Some(next_timeout);
        }
        if let Some(context_token) = latest_context_token.clone() {
            stored.weixin_context_token = Some(context_token);
        }
        if let Some(user_id) = latest_user_id.clone() {
            stored.weixin_user_id = Some(user_id);
        }
        stored.weixin_status = Some("confirmed".to_string());
        stored.weixin_status_message = Some("已扫码登录，正在接收微信消息".to_string());
        if stored
            .weixin_last_error
            .as_deref()
            .is_some_and(|message| message.starts_with("Weixin getupdates"))
        {
            stored.weixin_last_error = None;
        }
        if !messages.is_empty() {
            stored.weixin_last_seen_at = Some(now_unix());
            stored.weixin_last_error = None;
        }
    })?;

    Ok(())
}
