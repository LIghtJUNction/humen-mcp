fn expire_request(state: &AppState, id: Uuid, reason: String) -> Option<ExpiredRequest> {
    let (_, request) = state.requests.remove(&id)?;
    state.waiters.remove(&id);
    let expired = ExpiredRequest {
        request,
        expired_at: now_unix(),
        reason,
    };
    state.trash.insert(id, expired.clone());
    if let Err(err) = db_mark_expired(state, &expired) {
        warn!(%id, error = %err.message, "failed to persist expired request");
    }
    let _ = state.events.send(ServerEvent::RequestExpired {
        id,
        expired_request: expired.clone(),
    });
    Some(expired)
}

fn db_insert_request(state: &AppState, request: &HumanRequest) -> Result<(), ApiError> {
    let request_json = serde_json::to_string(request)
        .map_err(|err| ApiError::internal(format!("serialize request: {err}")))?;
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    db.execute(
        "INSERT OR REPLACE INTO human_requests \
         (id, status, request_json, created_at, expires_at, expired_at, expire_reason, answer_json, answered_at, answered_late, read_at) \
         VALUES (?1, 'pending', ?2, ?3, ?4, NULL, NULL, NULL, NULL, 0, NULL)",
        params![
            request.id.to_string(),
            request_json,
            request.created_at,
            request.expires_at
        ],
    )
    .map_err(|err| ApiError::internal(format!("persist request: {err}")))?;
    Ok(())
}

fn db_mark_expired(state: &AppState, expired: &ExpiredRequest) -> Result<(), ApiError> {
    let request_json = serde_json::to_string(&expired.request)
        .map_err(|err| ApiError::internal(format!("serialize expired request: {err}")))?;
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    db.execute(
        "INSERT INTO human_requests \
         (id, status, request_json, created_at, expires_at, expired_at, expire_reason, answered_late) \
         VALUES (?1, 'expired', ?2, ?3, ?4, ?5, ?6, 0) \
         ON CONFLICT(id) DO UPDATE SET \
           status='expired', request_json=excluded.request_json, expired_at=excluded.expired_at, expire_reason=excluded.expire_reason",
        params![
            expired.request.id.to_string(),
            request_json,
            expired.request.created_at,
            expired.request.expires_at,
            expired.expired_at,
            expired.reason
        ],
    )
    .map_err(|err| ApiError::internal(format!("persist expired request: {err}")))?;
    Ok(())
}

fn db_store_answer(
    state: &AppState,
    request: &HumanRequest,
    answer: &HumanAnswer,
    late: bool,
) -> Result<(), ApiError> {
    let request_json = serde_json::to_string(request)
        .map_err(|err| ApiError::internal(format!("serialize request: {err}")))?;
    let answer_json = serde_json::to_string(answer)
        .map_err(|err| ApiError::internal(format!("serialize answer: {err}")))?;
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    db.execute(
        "INSERT INTO human_requests \
         (id, status, request_json, created_at, expires_at, answer_json, answered_at, answered_late) \
         VALUES (?1, 'answered', ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(id) DO UPDATE SET \
           status='answered', request_json=excluded.request_json, answer_json=excluded.answer_json, \
           answered_at=excluded.answered_at, answered_late=excluded.answered_late",
        params![
            request.id.to_string(),
            request_json,
            request.created_at,
            request.expires_at,
            answer_json,
            answer.answered_at,
            if late { 1 } else { 0 }
        ],
    )
    .map_err(|err| ApiError::internal(format!("persist answer: {err}")))?;
    Ok(())
}

fn db_get_request(state: &AppState, id: Uuid) -> Result<Option<(HumanRequest, String)>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let row = db
        .query_row(
            "SELECT request_json, status FROM human_requests WHERE id = ?1",
            params![id.to_string()],
            |row| {
                let request_json: String = row.get(0)?;
                let status: String = row.get(1)?;
                Ok((request_json, status))
            },
        )
        .optional()
        .map_err(|err| ApiError::internal(format!("read request from sqlite: {err}")))?;
    let Some((request_json, status)) = row else {
        return Ok(None);
    };
    let request: HumanRequest = serde_json::from_str(&request_json)
        .map_err(|err| ApiError::internal(format!("parse request from sqlite: {err}")))?;
    Ok(Some((request, status)))
}

#[allow(dead_code)]
fn db_list_pending_requests(state: &AppState) -> Result<Vec<HumanRequest>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut stmt = db
        .prepare("SELECT request_json FROM human_requests WHERE status = 'pending'")
        .map_err(|err| ApiError::internal(format!("prepare pending query: {err}")))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|err| ApiError::internal(format!("query pending requests: {err}")))?;
    let mut requests = Vec::new();
    for row in rows {
        let raw = row.map_err(|err| ApiError::internal(format!("read pending request: {err}")))?;
        requests.push(
            serde_json::from_str(&raw)
                .map_err(|err| ApiError::internal(format!("parse pending request: {err}")))?,
        );
    }
    Ok(requests)
}

#[allow(dead_code)]
fn db_list_expired_requests(state: &AppState) -> Result<Vec<ExpiredRequest>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut stmt = db
        .prepare(
            "SELECT request_json, expired_at, expire_reason FROM human_requests WHERE status = 'expired'",
        )
        .map_err(|err| ApiError::internal(format!("prepare expired query: {err}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<u64>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|err| ApiError::internal(format!("query expired requests: {err}")))?;
    let mut expired = Vec::new();
    for row in rows {
        let (raw, expired_at, reason) =
            row.map_err(|err| ApiError::internal(format!("read expired request: {err}")))?;
        let request: HumanRequest = serde_json::from_str(&raw)
            .map_err(|err| ApiError::internal(format!("parse expired request: {err}")))?;
        expired.push(ExpiredRequest {
            request,
            expired_at: expired_at.unwrap_or_else(now_unix),
            reason: reason.unwrap_or_else(|| "Human request timed out".to_string()),
        });
    }
    Ok(expired)
}

fn db_list_answered_requests(
    state: &AppState,
    limit: u64,
) -> Result<Vec<AnsweredRequest>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut stmt = db
        .prepare(
            "SELECT request_json, answer_json, answered_late \
             FROM human_requests \
             WHERE status = 'answered' \
             ORDER BY answered_at DESC \
             LIMIT ?1",
        )
        .map_err(|err| ApiError::internal(format!("prepare answered query: {err}")))?;
    let rows = stmt
        .query_map(params![limit.clamp(1, 200)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })
        .map_err(|err| ApiError::internal(format!("query answered requests: {err}")))?;
    let mut answered = Vec::new();
    for row in rows {
        let (request_json, answer_json, answered_late) =
            row.map_err(|err| ApiError::internal(format!("read answered request: {err}")))?;
        let request: HumanRequest = serde_json::from_str(&request_json)
            .map_err(|err| ApiError::internal(format!("parse answered request: {err}")))?;
        let answer: HumanAnswer = serde_json::from_str(&answer_json)
            .map_err(|err| ApiError::internal(format!("parse answered answer: {err}")))?;
        answered.push(AnsweredRequest {
            request,
            answer,
            answered_late: answered_late != 0,
        });
    }
    Ok(answered)
}

fn db_list_human_leaderboard(state: &AppState) -> Result<Vec<HumanLeaderboardStat>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut stmt = db
        .prepare(
            "SELECT request_json, answer_json, answered_at \
             FROM human_requests \
             WHERE status = 'answered' AND answer_json IS NOT NULL",
        )
        .map_err(|err| ApiError::internal(format!("prepare leaderboard query: {err}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<u64>>(2)?,
            ))
        })
        .map_err(|err| ApiError::internal(format!("query leaderboard requests: {err}")))?;
    let mut by_email: HashMap<String, HumanLeaderboardStat> = HashMap::new();
    for row in rows {
        let (request_json, answer_json, answered_at) =
            row.map_err(|err| ApiError::internal(format!("read leaderboard request: {err}")))?;
        let request: HumanRequest = serde_json::from_str(&request_json)
            .map_err(|err| ApiError::internal(format!("parse leaderboard request: {err}")))?;
        let answer: HumanAnswer = serde_json::from_str(&answer_json)
            .map_err(|err| ApiError::internal(format!("parse leaderboard answer: {err}")))?;
        let email = request
            .assigned_to
            .as_deref()
            .map(normalize_email)
            .filter(|email| !email.is_empty())
            .or_else(|| {
                let answered_by = normalize_email(&answer.answered_by);
                if answered_by.is_empty() || answered_by.contains(':') {
                    None
                } else {
                    Some(answered_by)
                }
            });
        let Some(email) = email else {
            continue;
        };
        let mut answer_tokens = estimate_text_tokens(&answer.answer);
        if let Some(note) = answer.note.as_deref() {
            answer_tokens = answer_tokens.saturating_add(estimate_text_tokens(note));
        }
        let stat = by_email
            .entry(email.clone())
            .or_insert_with(|| HumanLeaderboardStat {
                email,
                requests_handled: 0,
                sent_tokens: 0,
                latest_answered_at: None,
            });
        stat.requests_handled = stat.requests_handled.saturating_add(1);
        stat.sent_tokens = stat.sent_tokens.saturating_add(answer_tokens);
        if let Some(answered_at) = answered_at {
            stat.latest_answered_at = Some(
                stat.latest_answered_at
                    .map(|current| current.max(answered_at))
                    .unwrap_or(answered_at),
            );
        }
    }
    let mut stats: Vec<_> = by_email.into_values().collect();
    stats.sort_by(|left, right| {
        right
            .requests_handled
            .cmp(&left.requests_handled)
            .then_with(|| right.sent_tokens.cmp(&left.sent_tokens))
            .then_with(|| left.email.cmp(&right.email))
    });
    Ok(stats)
}

fn estimate_text_tokens(value: &str) -> u64 {
    let mut tokens = 0u64;
    let mut ascii_run = 0u64;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            ascii_run = ascii_run.saturating_add(1);
            continue;
        }
        if ascii_run > 0 {
            tokens = tokens.saturating_add(ascii_run.saturating_add(3) / 4);
            ascii_run = 0;
        }
        if ch.is_whitespace() {
            continue;
        }
        tokens = tokens.saturating_add(1);
    }
    if ascii_run > 0 {
        tokens = tokens.saturating_add(ascii_run.saturating_add(3) / 4);
    }
    tokens
}

fn db_read_humen_replies(
    state: &AppState,
    agent_email: &str,
    args: ReadLateRepliesArgs,
) -> Result<Vec<LateHumanReply>, ApiError> {
    let request_id = args.request_id.map(|id| id.to_string());
    let unread_only = if args.unread_only { 1 } else { 0 };
    let limit = args.limit.unwrap_or(50).clamp(1, 200);
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut stmt = db
        .prepare(
            "SELECT id, request_json, answer_json, expired_at, read_at \
                    , answered_late \
             FROM human_requests \
             WHERE status = 'answered' \
               AND (?1 IS NULL OR id = ?1) \
               AND (?2 IS NULL OR answered_at >= ?2) \
               AND (?3 = 0 OR read_at IS NULL) \
             ORDER BY answered_at DESC \
             LIMIT ?4",
        )
        .map_err(|err| ApiError::internal(format!("prepare late replies query: {err}")))?;
    let rows = stmt
        .query_map(params![request_id, args.since, unread_only, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<u64>>(3)?,
                row.get::<_, Option<u64>>(4)?,
                row.get::<_, u64>(5)?,
            ))
        })
        .map_err(|err| ApiError::internal(format!("query late replies: {err}")))?;
    let mut ids = Vec::new();
    let mut replies = Vec::new();
    for row in rows {
        let (id, request_json, answer_json, expired_at, read_at, answered_late) =
            row.map_err(|err| ApiError::internal(format!("read late reply: {err}")))?;
        let request: HumanRequest = serde_json::from_str(&request_json)
            .map_err(|err| ApiError::internal(format!("parse late request: {err}")))?;
        if !can_access_request(agent_email, &request) {
            continue;
        }
        let answer: HumanAnswer = serde_json::from_str(&answer_json)
            .map_err(|err| ApiError::internal(format!("parse late answer: {err}")))?;
        ids.push(id);
        replies.push(LateHumanReply {
            request,
            answer,
            expired_at,
            answered_late: answered_late != 0,
            read_at,
        });
    }
    if args.mark_read && !ids.is_empty() {
        let read_at = now_unix();
        for id in ids {
            db.execute(
                "UPDATE human_requests SET read_at = COALESCE(read_at, ?1) WHERE id = ?2",
                params![read_at, id],
            )
            .map_err(|err| ApiError::internal(format!("mark late reply read: {err}")))?;
        }
    }
    Ok(replies)
}

fn db_store_agent_task(state: &AppState, task: &AgentTask) -> Result<(), ApiError> {
    let task_json = serde_json::to_string(task)
        .map_err(|err| ApiError::internal(format!("serialize task: {err}")))?;
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    db.execute(
        "INSERT INTO agent_tasks \
         (id, task_json, created_by, assigned_to, status, created_at, updated_at, due_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(id) DO UPDATE SET \
           task_json=excluded.task_json, created_by=excluded.created_by, assigned_to=excluded.assigned_to, \
           status=excluded.status, updated_at=excluded.updated_at, due_at=excluded.due_at",
        params![
            task.id.to_string(),
            task_json,
            normalize_email(&task.created_by),
            normalize_email(&task.assigned_to),
            task.status.as_str(),
            task.created_at,
            task.updated_at,
            task.due_at
        ],
    )
    .map_err(|err| ApiError::internal(format!("persist task: {err}")))?;
    Ok(())
}

fn db_get_agent_task(state: &AppState, id: Uuid) -> Result<Option<AgentTask>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let row = db
        .query_row(
            "SELECT task_json FROM agent_tasks WHERE id = ?1",
            params![id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| ApiError::internal(format!("read task from sqlite: {err}")))?;
    let Some(task_json) = row else {
        return Ok(None);
    };
    let task: AgentTask = serde_json::from_str(&task_json)
        .map_err(|err| ApiError::internal(format!("parse task from sqlite: {err}")))?;
    Ok(Some(task))
}

fn db_list_agent_tasks(
    state: &AppState,
    assigned_to: &str,
    status: Option<&AgentTaskStatus>,
    include_archived: bool,
    limit: u64,
) -> Result<Vec<AgentTask>, ApiError> {
    let assigned_to = normalize_email(assigned_to);
    let status = status.map(AgentTaskStatus::as_str);
    let include_archived = if include_archived { 1 } else { 0 };
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut stmt = db
        .prepare(
            "SELECT task_json \
             FROM agent_tasks \
             WHERE assigned_to = ?1 \
               AND (?2 IS NULL OR status = ?2) \
               AND (?3 = 1 OR status != 'archived') \
             ORDER BY updated_at DESC \
             LIMIT ?4",
        )
        .map_err(|err| ApiError::internal(format!("prepare task query: {err}")))?;
    let rows = stmt
        .query_map(
            params![assigned_to, status, include_archived, limit.clamp(1, 500)],
            |row| row.get::<_, String>(0),
        )
        .map_err(|err| ApiError::internal(format!("query tasks: {err}")))?;
    let mut tasks = Vec::new();
    for row in rows {
        let raw = row.map_err(|err| ApiError::internal(format!("read task: {err}")))?;
        tasks.push(
            serde_json::from_str(&raw)
                .map_err(|err| ApiError::internal(format!("parse task: {err}")))?,
        );
    }
    Ok(tasks)
}

fn db_store_human_rating(
    state: &AppState,
    rated_email: &str,
    rater_email: &str,
    score: f64,
    note: Option<&str>,
) -> Result<ReputationSummary, ApiError> {
    let rated_email = normalize_email(rated_email);
    let rater_email = normalize_email(rater_email);
    let score = score.clamp(0.0, 10.0);
    let note = normalize_optional_value(note);
    let now = now_unix();
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    db.execute(
        "INSERT INTO human_ratings \
         (rated_email, rater_email, score, note, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?5) \
         ON CONFLICT(rated_email, rater_email) DO UPDATE SET \
           score=excluded.score, note=excluded.note, updated_at=excluded.updated_at",
        params![rated_email, rater_email, score, note, now],
    )
    .map_err(|err| ApiError::internal(format!("persist human rating: {err}")))?;
    reputation_summary_from_db(&db, &rated_email)
}

fn db_reputation_summary_for(
    state: &AppState,
    email: &str,
) -> Result<ReputationSummary, ApiError> {
    let email = normalize_email(email);
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    reputation_summary_from_db(&db, &email)
}

fn reputation_summary_from_db(
    db: &Connection,
    email: &str,
) -> Result<ReputationSummary, ApiError> {
    let row = db
        .query_row(
            "SELECT AVG(score), COUNT(*) FROM human_ratings WHERE rated_email = ?1",
            params![normalize_email(email)],
            |row| Ok((row.get::<_, Option<f64>>(0)?, row.get::<_, u64>(1)?)),
        )
        .map_err(|err| ApiError::internal(format!("read reputation: {err}")))?;
    Ok(ReputationSummary {
        reputation: row.0.unwrap_or(5.0),
        ratings_count: row.1,
    })
}

fn db_reputation_map(state: &AppState) -> Result<HashMap<String, ReputationSummary>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut stmt = db
        .prepare(
            "SELECT rated_email, AVG(score), COUNT(*) \
             FROM human_ratings \
             GROUP BY rated_email",
        )
        .map_err(|err| ApiError::internal(format!("prepare reputation map: {err}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<f64>>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })
        .map_err(|err| ApiError::internal(format!("query reputation map: {err}")))?;
    let mut map = HashMap::new();
    for row in rows {
        let (email, reputation, ratings_count) =
            row.map_err(|err| ApiError::internal(format!("read reputation map: {err}")))?;
        map.insert(
            normalize_email(&email),
            ReputationSummary {
                reputation: reputation.unwrap_or(5.0),
                ratings_count,
            },
        );
    }
    Ok(map)
}

fn db_create_human_report(
    state: &AppState,
    reporter_email: &str,
    reported_email: &str,
    reason: &str,
) -> Result<HumanReport, ApiError> {
    let reporter_email = normalize_email(reporter_email);
    let reported_email = normalize_email(reported_email);
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(ApiError::bad_request("report reason is required"));
    }
    let report = HumanReport {
        id: Uuid::new_v4(),
        reporter_email,
        reported_email,
        reason: reason.to_string(),
        created_at: now_unix(),
        status: "open".to_string(),
    };
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    db.execute(
        "INSERT INTO human_reports \
         (id, reporter_email, reported_email, reason, created_at, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            report.id.to_string(),
            report.reporter_email,
            report.reported_email,
            report.reason,
            report.created_at,
            report.status
        ],
    )
    .map_err(|err| ApiError::internal(format!("persist human report: {err}")))?;
    Ok(report)
}

fn db_list_human_reports(state: &AppState, limit: u64) -> Result<Vec<HumanReport>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut stmt = db
        .prepare(
            "SELECT id, reporter_email, reported_email, reason, created_at, status \
             FROM human_reports \
             ORDER BY created_at DESC \
             LIMIT ?1",
        )
        .map_err(|err| ApiError::internal(format!("prepare reports query: {err}")))?;
    let rows = stmt
        .query_map(params![limit.clamp(1, 500)], |row| {
            let id: String = row.get(0)?;
            Ok(HumanReport {
                id: Uuid::parse_str(&id).map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(err),
                    )
                })?,
                reporter_email: row.get(1)?,
                reported_email: row.get(2)?,
                reason: row.get(3)?,
                created_at: row.get(4)?,
                status: row.get(5)?,
            })
        })
        .map_err(|err| ApiError::internal(format!("query human reports: {err}")))?;
    let mut reports = Vec::new();
    for row in rows {
        reports.push(row.map_err(|err| ApiError::internal(format!("read human report: {err}")))?);
    }
    Ok(reports)
}

async fn trash_cleanup_loop(state: AppState) {
    let interval_seconds = state.config.cleanup_interval_seconds.max(1);
    let retention_seconds = state.config.trash_retention_seconds;
    let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
    let mut shutdown = state.shutdown.subscribe();
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown.recv() => return,
        }
        let cutoff = now_unix().saturating_sub(retention_seconds);
        let before = state.trash.len();
        state
            .trash
            .retain(|_, expired| expired.expired_at >= cutoff);
        let removed_count = before.saturating_sub(state.trash.len());
        if removed_count > 0 {
            let _ = state
                .events
                .send(ServerEvent::TrashCleaned { removed_count });
        }
    }
}

fn begin_active_period(state: &AppState, email: &str) -> Option<usize> {
    let email = normalize_email(email);
    let mut users = state.users.lock().ok()?;
    let now = now_unix();
    let record = users
        .users
        .entry(email.clone())
        .or_insert_with(|| new_user_record(email.clone(), now, String::new()));
    record.active_periods.push(ActivePeriod {
        user_id: email.clone(),
        connected_at: now,
        disconnected_at: None,
        duration_seconds: None,
    });
    let index = record.active_periods.len().saturating_sub(1);
    if let Err(err) = users.save(&state.config.users_file) {
        warn!(%err, "failed to save active period start");
    }
    Some(index)
}

fn end_active_period(state: &AppState, email: &str, active_index: Option<usize>) {
    let Some(active_index) = active_index else {
        return;
    };
    let email = normalize_email(email);
    let Ok(mut users) = state.users.lock() else {
        return;
    };
    let now = now_unix();
    if let Some(record) = users.users.get_mut(&email) {
        if let Some(period) = record.active_periods.get_mut(active_index) {
            if period.disconnected_at.is_none() {
                period.disconnected_at = Some(now);
                period.duration_seconds = Some(now.saturating_sub(period.connected_at));
            }
        }
    }
    if let Err(err) = users.save(&state.config.users_file) {
        warn!(%err, "failed to save active period end");
    }
}
