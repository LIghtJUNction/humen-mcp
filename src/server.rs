#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    if let Some(Command::InitAdmin(args)) = cli.command {
        init_admin(args)?;
        return Ok(());
    }

    let config = cli.config;
    let bind = config.bind;
    let web_dist = config.web_dist.clone();
    let state = AppState::new(config)?;
    tokio::spawn(trash_cleanup_loop(state.clone()));
    tokio::spawn(weixin_poll_loop(state.clone()));
    let shutdown_tx = state.shutdown.clone();

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/mcp", get(mcp_get).post(mcp))
        .route("/mcp/", get(web_panel_legacy_redirect))
        .nest("/api", api_router())
        .nest("/mcp/api", api_router())
        .nest_service("/mcp/assets", ServeDir::new(format!("{web_dist}/assets")))
        .fallback_service(
            ServeDir::new(&web_dist).fallback(ServeFile::new(format!("{web_dist}/index.html"))),
        )
        .with_state(state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http());

    info!("listening on http://{bind}");
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .context("bind server socket")?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = shutdown_tx.send(());
        })
        .await
        .context("serve humen-mcp")?;

    Ok(())
}

async fn web_panel_legacy_redirect() -> Redirect {
    Redirect::permanent("/")
}

fn api_router() -> Router<AppState> {
    Router::new()
        .route("/auth/config", get(auth_config))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/passkey/start", post(passkey_authentication_start))
        .route(
            "/auth/passkey/finish",
            post(passkey_authentication_finish),
        )
        .route("/auth/oauth/github/start", get(github_oauth_start))
        .route("/auth/oauth/github/callback", get(github_oauth_callback))
        .route("/public/leaderboard", get(list_public_leaderboard))
        .route("/public/agents", get(list_public_agents))
        .route("/public/users/{username}", get(public_user_profile))
        .route("/me", get(me))
        .route("/me/profile", get(me_profile).post(update_me_profile))
        .route("/passkeys", get(list_passkeys))
        .route(
            "/passkeys/register/start",
            post(passkey_registration_start),
        )
        .route(
            "/passkeys/register/finish",
            post(passkey_registration_finish),
        )
        .route("/passkeys/{id}/delete", post(delete_passkey))
        .route("/agent/access", get(agent_access))
        .route("/agent/secret", post(update_agent_secret))
        .route(
            "/admin/webhooks",
            get(admin_webhooks).post(admin_update_webhooks),
        )
        .route(
            "/admin/webhooks/{id}/weixin/login/start",
            post(admin_weixin_login_start),
        )
        .route(
            "/admin/webhooks/{id}/weixin/login/status",
            get(admin_weixin_login_status),
        )
        .route(
            "/admin/webhooks/{id}/weixin/logout",
            post(admin_weixin_logout),
        )
        .route("/requests", get(list_requests))
        .route("/requests/{id}/hide", post(hide_request))
        .route("/requests/{id}/answer", post(answer_request))
        .route("/sent", get(list_sent_requests))
        .route("/stats/leaderboard", get(list_leaderboard))
        .route("/tasks", get(list_agent_tasks))
        .route("/tasks/{id}/status", post(update_agent_task_status))
        .route("/trash", get(list_trash))
        .route("/trash/clear", post(clear_trash))
        .route("/users/online", get(list_online_users))
        .route("/users/search", get(search_users))
        .route("/agents", get(list_connected_agents))
        .route(
            "/agents/{id}/friend-request",
            post(create_agent_friend_request),
        )
        .route("/agents/{id}/accept", post(accept_agent_friend_request))
        .route("/agents/{id}/ask-me", post(create_agent_ask_me_request))
        .route(
            "/agents/{id}/messages/{message_id}",
            patch(update_agent_human_message),
        )
        .route("/agents/{id}/rate", post(rate_agent))
        .route("/memos/unread", get(unread_human_memos))
        .route(
            "/humans/{email}/memos",
            get(list_human_memos).post(create_human_memo),
        )
        .route("/humans/rate", post(rate_human))
        .route("/humans/report", post(report_human))
        .route("/leaderboard", get(list_leaderboard))
        .route("/tags", get(list_tags))
        .route("/friends", get(list_friends).post(create_friend_request))
        .route("/friends/{email}/accept", post(accept_friend_request))
        .route("/friends/{email}/remove", post(remove_friend))
        .route("/admin/users", get(admin_list_users).post(admin_add_user))
        .route("/admin/users/{email}", post(admin_update_user))
        .route("/admin/users/{email}/kick", post(admin_kick_user))
        .route("/admin/reports", get(admin_reports))
        .route("/admin/update", get(admin_update_status).post(admin_start_update))
        .route(
            "/admin/settings",
            get(admin_settings).post(admin_update_settings),
        )
        .route("/ws", get(ws_handler))
}

fn init_admin(args: InitAdminArgs) -> anyhow::Result<()> {
    let email = match args.email {
        Some(email) => normalize_email(&email),
        None => prompt("Admin email")?,
    };
    validate_email_like_identifier(&email).map_err(|err| anyhow::anyhow!(err.message))?;

    let admin_pass = args.admin_pass.unwrap_or_else(|| random_secret(32));
    let session_secret = random_secret(64);
    let mut lines = match fs::read_to_string(&args.env_file) {
        Ok(raw) => raw.lines().map(str::to_string).collect(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => default_env_lines(),
        Err(err) => return Err(err).with_context(|| format!("read {}", args.env_file.display())),
    };

    set_env_value(&mut lines, "HUMEN_ADMIN_EMAIL", &email);
    set_env_value(&mut lines, "HUMEN_ADMIN_PASSWORD", &admin_pass);
    set_env_value(&mut lines, "HUMEN_SESSION_SECRET", &session_secret);
    set_env_value(
        &mut lines,
        "HUMEN_USERS_FILE",
        "/var/lib/humen-mcp/users.json",
    );

    if let Some(parent) = args.env_file.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&args.env_file, format!("{}\n", lines.join("\n")))
        .with_context(|| format!("write {}", args.env_file.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&args.env_file, fs::Permissions::from_mode(0o640))
            .with_context(|| format!("chmod {}", args.env_file.display()))?;
    }

    println!("Initialized admin account in {}", args.env_file.display());
    println!("Admin email: {email}");
    println!("Admin password: {admin_pass}");
    println!("Restart the service after changing the env file:");
    println!("  sudo systemctl restart humen-mcp.service");
    Ok(())
}

fn prompt(label: &str) -> anyhow::Result<String> {
    eprint!("{label}: ");
    io::stderr().flush().context("flush prompt")?;
    let mut value = String::new();
    io::stdin().read_line(&mut value).context("read prompt")?;
    let value = value.trim().to_string();
    if value.is_empty() {
        anyhow::bail!("{label} is required");
    }
    Ok(value)
}

fn default_env_lines() -> Vec<String> {
    [
        "HUMEN_BIND=127.0.0.1:8787",
        "HUMEN_PUBLIC_BASE_URL=https://your-domain.example",
        "HUMEN_WEB_DIST=/usr/share/humen-mcp/web",
        "HUMEN_USERS_FILE=/var/lib/humen-mcp/users.json",
        "HUMEN_DB_FILE=/var/lib/humen-mcp/humen-mcp.sqlite3",
        "HUMEN_ADMIN_EMAIL=<admin-email>",
        "HUMEN_ADMIN_PASSWORD=change-me",
        "HUMEN_SESSION_SECRET=change-this-to-a-long-random-secret",
        "HUMEN_TRASH_RETENTION_SECONDS=604800",
        "HUMEN_CLEANUP_INTERVAL_SECONDS=60",
        "HUMEN_GITHUB_CLIENT_ID=",
        "HUMEN_GITHUB_CLIENT_SECRET=",
        "HUMEN_GITHUB_API_TOKEN=",
        "HUMEN_SELF_UPDATE_COMMAND=",
        "HUMEN_SELF_UPDATE_TIMEOUT_SECONDS=120",
        "HUMEN_PLUGIN_DIR=",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn set_env_value(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}=");
    if let Some(line) = lines
        .iter_mut()
        .find(|line| line.trim_start().starts_with(&prefix))
    {
        *line = format!("{key}={value}");
    } else {
        lines.push(format!("{key}={value}"));
    }
}

async fn healthz() -> Json<Value> {
    Json(json!({ "ok": true }))
}
async fn mcp_get(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if !mcp_accepts_sse(&headers) {
        return Ok((
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "humen-mcp JSON-RPC endpoint. Use POST /mcp with application/json, or GET /mcp with Accept: text/event-stream for notifications.\n",
        )
            .into_response());
    }
    let agent = require_agent_access(&state, &headers)?;
    let agent = db_touch_agent_presence(&state, &agent, &headers)?;
    Ok(mcp_sse(state, agent).into_response())
}
