async fn admin_update_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminUpdateStatus>, ApiError> {
    require_admin(&state, &headers)?;
    Ok(Json(self_update_status(&state)))
}

async fn admin_start_update(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminUpdateResponse>, ApiError> {
    require_admin(&state, &headers)?;
    Ok(Json(run_self_update(state).await?))
}

fn self_update_status(state: &AppState) -> AdminUpdateStatus {
    AdminUpdateStatus {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        enabled: !state.config.self_update_command.trim().is_empty(),
        running: state.self_update_running.load(Ordering::SeqCst),
        timeout_seconds: state.config.self_update_timeout_seconds,
    }
}

async fn run_self_update(state: AppState) -> Result<AdminUpdateResponse, ApiError> {
    let command = state.config.self_update_command.trim().to_string();
    if command.is_empty() {
        return Err(ApiError::bad_request("self update command is not configured"));
    }
    if state
        .self_update_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(ApiError::conflict("self update is already running"));
    }

    let result = run_self_update_command(&state, &command).await;
    state.self_update_running.store(false, Ordering::SeqCst);
    result
}

async fn run_self_update_command(
    state: &AppState,
    command: &str,
) -> Result<AdminUpdateResponse, ApiError> {
    let timeout_seconds = state.config.self_update_timeout_seconds.clamp(5, 3600);
    let output = tokio::time::timeout(Duration::from_secs(timeout_seconds), async {
        TokioCommand::new("/bin/sh")
            .arg("-lc")
            .arg(command)
            .env("HUMEN_CURRENT_VERSION", env!("CARGO_PKG_VERSION"))
            .kill_on_drop(true)
            .output()
            .await
    })
    .await
    .map_err(|_| ApiError::upstream("self update command timed out"))?
    .map_err(|err| ApiError::upstream(format!("start self update command: {err}")))?;

    let stdout = command_output_text(&output.stdout);
    let stderr = command_output_text(&output.stderr);
    if !output.status.success() {
        return Err(ApiError::upstream(format!(
            "self update command failed{}{}",
            output
                .status
                .code()
                .map(|code| format!(" with exit code {code}"))
                .unwrap_or_default(),
            command_failure_detail(&stdout, &stderr)
        )));
    }

    Ok(AdminUpdateResponse {
        ok: true,
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        started: true,
        message: "self update started; the service may restart shortly".to_string(),
        status_code: output.status.code(),
        stdout,
        stderr,
    })
}

fn command_output_text(bytes: &[u8]) -> String {
    truncate_command_text(String::from_utf8_lossy(bytes).trim().to_string(), 4000)
}

fn command_failure_detail(stdout: &str, stderr: &str) -> String {
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        return String::new();
    };
    format!(": {}", truncate_command_text(detail.to_string(), 600))
}

fn truncate_command_text(value: String, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        truncated.push_str("...");
    }
    truncated
}
