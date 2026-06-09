const GITHUB_ACCOUNT_CACHE_TTL_SECONDS: u64 = 24 * 60 * 60;

async fn github_reputation_seed_for_oauth_user(
    state: &AppState,
    github_user: &Value,
    oauth_access_token: &str,
) -> Result<GithubReputationSeed, ApiError> {
    let login = github_user
        .get("login")
        .and_then(Value::as_str)
        .and_then(normalize_github_login)
        .ok_or_else(|| ApiError::upstream("GitHub user response had no login"))?;
    let now = now_unix();
    if let Some(snapshot) = db_get_fresh_github_account_snapshot(state, &login, now)? {
        return Ok(github_reputation_seed_from_snapshot(snapshot));
    }

    let bearer = effective_github_api_token(state).unwrap_or_else(|| oauth_access_token.to_string());
    let repos = github_get_json(
        state,
        &format!("https://api.github.com/users/{login}/repos?type=owner&sort=updated&per_page=30"),
        Some(&bearer),
    )
    .await
    .unwrap_or_else(|err| {
        warn!(login, error = %err.message, "failed to fetch GitHub repos for reputation seed");
        Value::Array(Vec::new())
    });
    let events = github_get_json(
        state,
        &format!("https://api.github.com/users/{login}/events/public?per_page=10"),
        Some(&bearer),
    )
    .await
    .unwrap_or_else(|err| {
        warn!(login, error = %err.message, "failed to fetch GitHub events for reputation seed");
        Value::Array(Vec::new())
    });

    let snapshot = github_account_snapshot_from_api(&login, github_user, &repos, &events, now);
    db_store_github_account_snapshot(state, &snapshot, GITHUB_ACCOUNT_CACHE_TTL_SECONDS)?;
    Ok(github_reputation_seed_from_snapshot(snapshot))
}

async fn github_get_json(
    state: &AppState,
    url: &str,
    bearer: Option<&str>,
) -> Result<Value, ApiError> {
    let mut request = state
        .http
        .get(url)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header(header::USER_AGENT, "humen-mcp");
    if let Some(bearer) = bearer.filter(|value| !value.trim().is_empty()) {
        request = request.bearer_auth(bearer);
    }
    let response = request
        .send()
        .await
        .map_err(|err| ApiError::upstream(format!("GitHub API request failed: {err}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(ApiError::upstream(format!(
            "GitHub API returned HTTP {status}"
        )));
    }
    response
        .json()
        .await
        .map_err(|err| ApiError::upstream(format!("GitHub API response was invalid: {err}")))
}

fn github_account_snapshot_from_api(
    login: &str,
    user: &Value,
    repos: &Value,
    events: &Value,
    fetched_at: u64,
) -> GithubAccountSnapshot {
    let repos = repos.as_array();
    let total_stars_sampled = repos
        .into_iter()
        .flatten()
        .filter_map(|repo| repo.get("stargazers_count").and_then(Value::as_u64))
        .sum();
    let source_repos_sampled = repos
        .into_iter()
        .flatten()
        .filter(|repo| !repo.get("fork").and_then(Value::as_bool).unwrap_or(false))
        .count() as u64;
    let fork_repos_sampled = repos
        .into_iter()
        .flatten()
        .filter(|repo| repo.get("fork").and_then(Value::as_bool).unwrap_or(false))
        .count() as u64;
    let recent_activity_year = repos
        .into_iter()
        .flatten()
        .filter_map(|repo| repo.get("pushed_at").and_then(Value::as_str))
        .filter_map(github_year_from_timestamp)
        .max();

    GithubAccountSnapshot {
        login: login.to_string(),
        account_created_at: user
            .get("created_at")
            .and_then(Value::as_str)
            .map(str::to_string),
        public_repos: user.get("public_repos").and_then(Value::as_u64).unwrap_or(0),
        public_gists: user.get("public_gists").and_then(Value::as_u64).unwrap_or(0),
        followers: user.get("followers").and_then(Value::as_u64).unwrap_or(0),
        following: user.get("following").and_then(Value::as_u64).unwrap_or(0),
        total_stars_sampled,
        source_repos_sampled,
        fork_repos_sampled,
        recent_events_sampled: events.as_array().map(|items| items.len() as u64).unwrap_or(0),
        recent_activity_year,
        fetched_at,
    }
}

fn github_reputation_seed_from_snapshot(snapshot: GithubAccountSnapshot) -> GithubReputationSeed {
    let current_year = unix_approx_year(snapshot.fetched_at);
    let account_year = snapshot
        .account_created_at
        .as_deref()
        .and_then(github_year_from_timestamp);
    let account_age_years = account_year
        .map(|year| current_year.saturating_sub(year).max(0) as f64)
        .unwrap_or(0.0);
    let source_repo_ratio = if snapshot.public_repos > 0 {
        snapshot.source_repos_sampled as f64
            / (snapshot.source_repos_sampled + snapshot.fork_repos_sampled).max(1) as f64
    } else {
        0.0
    };
    let recent_activity_bonus = snapshot
        .recent_activity_year
        .map(|year| current_year.saturating_sub(year) <= 1)
        .unwrap_or(snapshot.recent_events_sampled > 0);

    let mut score = 4.0;
    score += (account_age_years * 0.16).min(1.6);
    score += log_signal(snapshot.public_repos, 35.0, 1.3);
    score += log_signal(snapshot.followers, 80.0, 1.2);
    score += log_signal(snapshot.total_stars_sampled, 60.0, 1.0);
    score += source_repo_ratio * 0.6;
    if recent_activity_bonus {
        score += 0.5;
    }
    if snapshot.public_repos == 0 && snapshot.followers == 0 {
        score -= 0.7;
    }

    GithubReputationSeed {
        score: score.clamp(2.0, 9.5),
        weight: 2.0,
        snapshot,
    }
}

fn github_seed_as_reputation_seed(seed: &GithubReputationSeed) -> ReputationSeed {
    ReputationSeed {
        source: "github".to_string(),
        score: seed.score,
        weight: seed.weight,
        details: json!({
            "github": seed.snapshot,
            "algorithm": {
                "version": 1,
                "signals": [
                    "account_age",
                    "public_repos",
                    "followers",
                    "sampled_repo_stars",
                    "source_repo_ratio",
                    "recent_public_activity"
                ]
            }
        }),
    }
}

fn effective_github_api_token(state: &AppState) -> Option<String> {
    normalize_optional_value(state.config.github_api_token.as_deref()).or_else(|| {
        state
            .admin_settings
            .lock()
            .ok()
            .and_then(|settings| normalize_optional_value(settings.github_api_token.as_deref()))
    })
}

fn normalize_github_login(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 39
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

fn normalize_github_login_key(value: &str) -> String {
    normalize_github_login(value).unwrap_or_else(|| value.trim().to_ascii_lowercase())
}

fn github_year_from_timestamp(value: &str) -> Option<i64> {
    value.get(..4)?.parse().ok()
}

fn unix_approx_year(unix: u64) -> i64 {
    1970 + (unix / 31_557_600) as i64
}

fn log_signal(value: u64, scale: f64, max_points: f64) -> f64 {
    if value == 0 {
        return 0.0;
    }
    (((value as f64 + 1.0).ln()) / scale.ln()).min(1.0) * max_points
}
