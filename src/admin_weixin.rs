async fn admin_weixin_login_start(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<WebhookConfig>, ApiError> {
    require_admin(&state, &headers)?;
    let webhook =
        find_webhook(&state, id).ok_or_else(|| ApiError::bad_request("webhook not found"))?;
    ensure_weixin_webhook(&webhook)?;

    let response = state
        .http
        .get(weixin_api_url(
            WEIXIN_DEFAULT_BASE_URL,
            "/ilink/bot/get_bot_qrcode",
        ))
        .header("iLink-App-ClientVersion", WEIXIN_CLIENT_VERSION)
        .query(&[("bot_type", "3")])
        .timeout(Duration::from_millis(weixin_api_timeout_ms(&webhook)))
        .send()
        .await
        .map_err(|err| ApiError::upstream(format!("Weixin QR code request failed: {err}")))?;
    if !response.status().is_success() {
        return Err(ApiError::upstream(format!(
            "Weixin QR code request returned {}",
            response.status()
        )));
    }
    let qr: WeixinQrCodeResponse = response
        .json()
        .await
        .map_err(|err| ApiError::upstream(format!("Weixin QR code response was invalid: {err}")))?;
    let qr_image = weixin_qrcode_data_uri(&qr.qrcode_img_content)?;

    let updated = update_webhook_config(&state, id, |webhook| {
        webhook.weixin_qrcode = Some(qr.qrcode.clone());
        webhook.weixin_qrcode_url = Some(qr_image.clone());
        webhook.weixin_status = Some("waiting".to_string());
        webhook.weixin_status_message = Some("请用微信扫码登录".to_string());
        webhook.weixin_bot_token = None;
        webhook.weixin_account_id = None;
        webhook.weixin_base_url = None;
        webhook.weixin_user_id = None;
        webhook.weixin_context_token = None;
        webhook.weixin_last_request_id = None;
        webhook.weixin_get_updates_buf = None;
        webhook.weixin_last_error = None;
    })?;
    Ok(Json(updated))
}

async fn admin_weixin_login_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<WeixinLoginStatusQuery>,
    headers: HeaderMap,
) -> Result<Json<WebhookConfig>, ApiError> {
    require_admin(&state, &headers)?;
    let webhook =
        find_webhook(&state, id).ok_or_else(|| ApiError::bad_request("webhook not found"))?;
    ensure_weixin_webhook(&webhook)?;
    let qrcode = normalize_optional_value(webhook.weixin_qrcode.as_deref())
        .ok_or_else(|| ApiError::bad_request("start Weixin scan login first"))?;
    let mut query_params = vec![("qrcode", qrcode)];
    if let Some(verify_code) = normalize_optional_value(query.verify_code.as_deref()) {
        query_params.push(("verify_code", verify_code));
    }

    let response = match state
        .http
        .get(weixin_api_url(
            WEIXIN_DEFAULT_BASE_URL,
            "/ilink/bot/get_qrcode_status",
        ))
        .header("iLink-App-ClientVersion", WEIXIN_CLIENT_VERSION)
        .timeout(Duration::from_millis(weixin_api_timeout_ms(&webhook)))
        .query(&query_params)
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) if err.is_timeout() => {
            let updated = update_webhook_config(&state, id, |webhook| {
                webhook.weixin_status = Some("wait".to_string());
                webhook.weixin_status_message = Some("请用微信扫码登录".to_string());
                webhook.weixin_last_error = None;
            })?;
            return Ok(Json(updated));
        }
        Err(err) => {
            return Err(ApiError::upstream(format!(
                "Weixin QR status request failed: {err}"
            )));
        }
    };
    if !response.status().is_success() {
        return Err(ApiError::upstream(format!(
            "Weixin QR status request returned {}",
            response.status()
        )));
    }
    let status: WeixinQrStatusResponse = response.json().await.map_err(|err| {
        ApiError::upstream(format!("Weixin QR status response was invalid: {err}"))
    })?;

    let status_name = status.status.trim().to_ascii_lowercase();
    let updated = update_webhook_config(&state, id, |webhook| match status_name.as_str() {
        "confirmed" => {
            webhook.enabled = true;
            webhook.weixin_status = Some("confirmed".to_string());
            webhook.weixin_status_message = Some("已扫码登录，正在接收微信消息".to_string());
            webhook.weixin_bot_token = normalize_optional_value(status.bot_token.as_deref());
            webhook.weixin_account_id = normalize_optional_value(status.ilink_bot_id.as_deref());
            webhook.weixin_user_id = normalize_optional_value(status.ilink_user_id.as_deref());
            webhook.weixin_base_url = normalize_optional_value(status.baseurl.as_deref());
            webhook.weixin_qrcode = None;
            webhook.weixin_qrcode_url = None;
            webhook.weixin_context_token = None;
            webhook.weixin_last_request_id = None;
            webhook.weixin_get_updates_buf = None;
            webhook.weixin_last_error = None;
        }
        "expired" => {
            webhook.weixin_status = Some("expired".to_string());
            webhook.weixin_status_message = Some("二维码已过期，请重新生成".to_string());
            webhook.weixin_qrcode = None;
            webhook.weixin_qrcode_url = None;
        }
        "scaned" | "scanned" => {
            webhook.weixin_status = Some("scaned".to_string());
            webhook.weixin_status_message = Some("已扫码，请在手机微信确认登录".to_string());
        }
        "wait" | "waiting" | "" => {
            webhook.weixin_status = Some("wait".to_string());
            webhook.weixin_status_message = Some("请用微信扫码登录".to_string());
        }
        other => {
            webhook.weixin_status = Some(other.to_string());
            webhook.weixin_status_message = Some(format!("扫码状态：{other}"));
        }
    })?;
    Ok(Json(updated))
}

async fn admin_weixin_logout(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<WebhookConfig>, ApiError> {
    require_admin(&state, &headers)?;
    let webhook =
        find_webhook(&state, id).ok_or_else(|| ApiError::bad_request("webhook not found"))?;
    ensure_weixin_webhook(&webhook)?;
    let updated = update_webhook_config(&state, id, |webhook| {
        webhook.enabled = false;
        webhook.weixin_qrcode = None;
        webhook.weixin_qrcode_url = None;
        webhook.weixin_status = Some("logged_out".to_string());
        webhook.weixin_status_message = Some("已退出扫码登录".to_string());
        webhook.weixin_bot_token = None;
        webhook.weixin_account_id = None;
        webhook.weixin_base_url = None;
        webhook.weixin_user_id = None;
        webhook.weixin_context_token = None;
        webhook.weixin_last_request_id = None;
        webhook.weixin_get_updates_buf = None;
        webhook.weixin_last_error = None;
        webhook.weixin_last_seen_at = None;
    })?;
    Ok(Json(updated))
}
