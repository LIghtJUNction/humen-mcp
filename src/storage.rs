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
    let rater_reputation = reputation_summary_from_db(&db, &rater_email)?;
    let weight = reputation_feedback_weight(&rater_reputation);
    db.execute(
        "INSERT INTO human_ratings \
         (rated_email, rater_email, score, weight, note, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) \
         ON CONFLICT(rated_email, rater_email) DO UPDATE SET \
           score=excluded.score, weight=excluded.weight, note=excluded.note, updated_at=excluded.updated_at",
        params![rated_email, rater_email, score, weight, note, now],
    )
    .map_err(|err| ApiError::internal(format!("persist human rating: {err}")))?;
    reputation_summary_from_db(&db, &rated_email)
}

fn db_create_human_memo(
    state: &AppState,
    target_email: &str,
    author_email: &str,
    body: &str,
) -> Result<HumanMemo, ApiError> {
    let body = body.trim();
    if body.is_empty() {
        return Err(ApiError::bad_request("memo body is required"));
    }
    if body.chars().count() > 2000 {
        return Err(ApiError::bad_request("memo body is too long"));
    }
    let memo = HumanMemo {
        id: Uuid::new_v4(),
        target_email: normalize_email(target_email),
        author_email: normalize_email(author_email),
        body: body.to_string(),
        created_at: now_unix(),
    };
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    db.execute(
        "INSERT INTO human_memos (id, target_email, author_email, body, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            memo.id.to_string(),
            memo.target_email,
            memo.author_email,
            memo.body,
            memo.created_at
        ],
    )
    .map_err(|err| ApiError::internal(format!("persist human memo: {err}")))?;
    Ok(memo)
}

fn db_list_human_memos(
    state: &AppState,
    target_email: &str,
    limit: u64,
) -> Result<Vec<HumanMemo>, ApiError> {
    let target_email = normalize_email(target_email);
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut stmt = db
        .prepare(
            "SELECT id, target_email, author_email, body, created_at \
             FROM human_memos \
             WHERE target_email = ?1 \
             ORDER BY created_at DESC \
             LIMIT ?2",
        )
        .map_err(|err| ApiError::internal(format!("prepare human memos query: {err}")))?;
    let rows = stmt
        .query_map(params![target_email, limit.clamp(1, 100)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u64>(4)?,
            ))
        })
        .map_err(|err| ApiError::internal(format!("query human memos: {err}")))?;
    let mut memos = Vec::new();
    for row in rows {
        let (id, target_email, author_email, body, created_at) =
            row.map_err(|err| ApiError::internal(format!("read human memo: {err}")))?;
        let id = Uuid::parse_str(&id)
            .map_err(|err| ApiError::internal(format!("parse human memo id: {err}")))?;
        memos.push(HumanMemo {
            id,
            target_email,
            author_email,
            body,
            created_at,
        });
    }
    Ok(memos)
}

fn reputation_feedback_weight(rater_reputation: &ReputationSummary) -> f64 {
    (rater_reputation.reputation.clamp(0.0, 10.0) / 5.0).clamp(0.5, 2.0)
}

fn db_upsert_reputation_seed(
    state: &AppState,
    email: &str,
    seed: ReputationSeed,
) -> Result<ReputationSummary, ApiError> {
    let email = normalize_email(email);
    let source = seed.source.trim();
    if source.is_empty() {
        return Err(ApiError::bad_request("reputation seed source is required"));
    }
    let score = seed.score.clamp(0.0, 10.0);
    let weight = seed.weight.clamp(0.0, 20.0);
    let details_json = serde_json::to_string(&seed.details)
        .map_err(|err| ApiError::internal(format!("serialize reputation seed: {err}")))?;
    let now = now_unix();
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    db.execute(
        "INSERT INTO reputation_seeds \
         (email, source, score, weight, details_json, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) \
         ON CONFLICT(email) DO UPDATE SET \
           source=excluded.source, score=excluded.score, weight=excluded.weight, \
           details_json=excluded.details_json, updated_at=excluded.updated_at",
        params![email, source, score, weight, details_json, now],
    )
    .map_err(|err| ApiError::internal(format!("persist reputation seed: {err}")))?;
    reputation_summary_from_db(&db, &email)
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
    let email = normalize_email(email);
    let rating_row = db
        .query_row(
            "SELECT SUM(score * weight), SUM(weight), COUNT(*) FROM human_ratings WHERE rated_email = ?1",
            params![email],
            |row| {
                Ok((
                    row.get::<_, Option<f64>>(0)?,
                    row.get::<_, Option<f64>>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            },
        )
        .map_err(|err| ApiError::internal(format!("read reputation: {err}")))?;
    let seed = reputation_seed_from_db(db, &email)?;
    Ok(reputation_summary_from_parts(
        seed.as_ref(),
        rating_row.0.unwrap_or(0.0),
        rating_row.1.unwrap_or(0.0),
        rating_row.2,
    ))
}

fn db_reputation_map(state: &AppState) -> Result<HashMap<String, ReputationSummary>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut parts: HashMap<String, (Option<ReputationSeed>, f64, f64, u64)> = HashMap::new();

    let mut stmt = db
        .prepare(
            "SELECT rated_email, SUM(score * weight), SUM(weight), COUNT(*) \
             FROM human_ratings \
             GROUP BY rated_email",
        )
        .map_err(|err| ApiError::internal(format!("prepare reputation map: {err}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<f64>>(1)?,
                row.get::<_, Option<f64>>(2)?,
                row.get::<_, u64>(3)?,
            ))
        })
        .map_err(|err| ApiError::internal(format!("query reputation map: {err}")))?;
    for row in rows {
        let (email, rating_weighted_sum, rating_weight_total, ratings_count) =
            row.map_err(|err| ApiError::internal(format!("read reputation map: {err}")))?;
        let entry = parts
            .entry(normalize_email(&email))
            .or_insert_with(|| (None, 0.0, 0.0, 0));
        entry.1 = rating_weighted_sum.unwrap_or(0.0);
        entry.2 = rating_weight_total.unwrap_or(0.0);
        entry.3 = ratings_count;
    }

    let mut stmt = db
        .prepare("SELECT email, source, score, weight, details_json FROM reputation_seeds")
        .map_err(|err| ApiError::internal(format!("prepare reputation seed map: {err}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|err| ApiError::internal(format!("query reputation seed map: {err}")))?;
    for row in rows {
        let (email, source, score, weight, details_json) =
            row.map_err(|err| ApiError::internal(format!("read reputation seed map: {err}")))?;
        let details = serde_json::from_str(&details_json)
            .map_err(|err| ApiError::internal(format!("parse reputation seed details: {err}")))?;
        let entry = parts
            .entry(normalize_email(&email))
            .or_insert_with(|| (None, 0.0, 0.0, 0));
        entry.0 = Some(ReputationSeed {
            source,
            score,
            weight,
            details,
        });
    }

    let mut map = HashMap::new();
    for (email, (seed, rating_weighted_sum, rating_weight_total, ratings_count)) in parts {
        map.insert(
            email,
            reputation_summary_from_parts(
                seed.as_ref(),
                rating_weighted_sum,
                rating_weight_total,
                ratings_count,
            ),
        );
    }
    Ok(map)
}

fn reputation_seed_from_db(
    db: &Connection,
    email: &str,
) -> Result<Option<ReputationSeed>, ApiError> {
    let row = db
        .query_row(
            "SELECT source, score, weight, details_json FROM reputation_seeds WHERE email = ?1",
            params![normalize_email(email)],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|err| ApiError::internal(format!("read reputation seed: {err}")))?;
    let Some((source, score, weight, details_json)) = row else {
        return Ok(None);
    };
    let details = serde_json::from_str(&details_json)
        .map_err(|err| ApiError::internal(format!("parse reputation seed details: {err}")))?;
    Ok(Some(ReputationSeed {
        source,
        score,
        weight,
        details,
    }))
}

fn reputation_summary_from_parts(
    seed: Option<&ReputationSeed>,
    rating_weighted_sum: f64,
    rating_weight_total: f64,
    ratings_count: u64,
) -> ReputationSummary {
    let seed = seed.filter(|seed| seed.weight > 0.0);
    let seed_weight = seed.map(|seed| seed.weight).unwrap_or(0.0);
    let total_weight = seed_weight + rating_weight_total;
    let reputation = if let Some(seed) = seed {
        let denominator = total_weight;
        if denominator > 0.0 {
            ((seed.score * seed.weight) + rating_weighted_sum) / denominator
        } else {
            5.0
        }
    } else if rating_weight_total > 0.0 {
        rating_weighted_sum / rating_weight_total
    } else {
        5.0
    };
    ReputationSummary {
        reputation: reputation.clamp(0.0, 10.0),
        ratings_count,
        reputation_breakdown: ReputationBreakdown {
            seed_source: seed.map(|seed| seed.source.clone()),
            seed_score: seed.map(|seed| seed.score.clamp(0.0, 10.0)),
            seed_weight,
            feedback_weight: rating_weight_total,
            total_weight,
            confidence: reputation_confidence(total_weight),
        },
    }
}

fn reputation_confidence(total_weight: f64) -> f64 {
    if total_weight <= 0.0 {
        return 0.0;
    }
    (total_weight / 8.0).clamp(0.0, 1.0)
}

fn db_get_fresh_github_account_snapshot(
    state: &AppState,
    login: &str,
    now: u64,
) -> Result<Option<GithubAccountSnapshot>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let row = db
        .query_row(
            "SELECT account_json FROM github_account_cache WHERE login = ?1 AND expires_at > ?2",
            params![normalize_github_login_key(login), now],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| ApiError::internal(format!("read GitHub account cache: {err}")))?;
    let Some(raw) = row else {
        return Ok(None);
    };
    let snapshot = serde_json::from_str(&raw)
        .map_err(|err| ApiError::internal(format!("parse GitHub account cache: {err}")))?;
    Ok(Some(snapshot))
}

fn db_store_github_account_snapshot(
    state: &AppState,
    snapshot: &GithubAccountSnapshot,
    ttl_seconds: u64,
) -> Result<(), ApiError> {
    let account_json = serde_json::to_string(snapshot)
        .map_err(|err| ApiError::internal(format!("serialize GitHub account cache: {err}")))?;
    let fetched_at = snapshot.fetched_at;
    let expires_at = fetched_at.saturating_add(ttl_seconds);
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    db.execute(
        "INSERT INTO github_account_cache (login, account_json, fetched_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(login) DO UPDATE SET \
           account_json=excluded.account_json, fetched_at=excluded.fetched_at, expires_at=excluded.expires_at",
        params![
            normalize_github_login_key(&snapshot.login),
            account_json,
            fetched_at,
            expires_at
        ],
    )
    .map_err(|err| ApiError::internal(format!("persist GitHub account cache: {err}")))?;
    Ok(())
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
