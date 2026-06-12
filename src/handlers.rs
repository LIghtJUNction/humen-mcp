async fn list_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<HumanRequest>>, ApiError> {
    let session = require_session(&state, &headers)?;
    let email = normalize_email(&session.user.email);
    let hidden = db_hidden_request_ids(&state, &email)?;
    let mut requests: Vec<_> = state
        .requests
        .iter()
        .filter(|entry| {
            can_access_request(&state, &email, entry.value()) && !hidden.contains(entry.key())
        })
        .map(|entry| entry.value().clone())
        .collect();
    requests.sort_by_key(|request| request.created_at);
    Ok(Json(requests))
}

async fn list_trash(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ExpiredRequest>>, ApiError> {
    let session = require_session(&state, &headers)?;
    let email = normalize_email(&session.user.email);
    let hidden = db_hidden_request_ids(&state, &email)?;
    let mut trash: Vec<_> = state
        .trash
        .iter()
        .filter(|entry| {
            can_access_request(&state, &email, &entry.value().request) && !hidden.contains(entry.key())
        })
        .map(|entry| entry.value().clone())
        .collect();
    trash.sort_by_key(|entry| entry.expired_at);
    Ok(Json(trash))
}

async fn list_sent_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AnsweredRequest>>, ApiError> {
    let session = require_session(&state, &headers)?;
    let email = normalize_email(&session.user.email);
    let hidden = db_hidden_request_ids(&state, &email)?;
    Ok(Json(
        db_list_answered_requests(&state, 100)?
            .into_iter()
            .filter(|entry| {
                can_access_request(&state, &email, &entry.request)
                    && !hidden.contains(&entry.request.id)
            })
            .collect(),
    ))
}

async fn list_leaderboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<HumanLeaderboardEntry>>, ApiError> {
    let session = require_session(&state, &headers)?;
    let visible_profiles = visible_user_profiles_for_session(&state, &session.user.email, None, None)?;
    Ok(Json(leaderboard_entries_for_profiles(
        &state,
        visible_profiles,
    )?))
}

async fn list_public_leaderboard(
    State(state): State<AppState>,
) -> Result<Json<Vec<HumanLeaderboardEntry>>, ApiError> {
    let public_profiles: Vec<_> = user_profiles(&state, None, None)?
        .into_iter()
        .filter(|profile| profile.is_public)
        .collect();
    Ok(Json(leaderboard_entries_for_profiles(
        &state,
        public_profiles,
    )?))
}

async fn list_public_agents(
    State(state): State<AppState>,
) -> Result<Json<Vec<PublicConnectedAgent>>, ApiError> {
    let agents = db_list_connected_agents(&state, "", 24)?
        .into_iter()
        .map(|agent| PublicConnectedAgent {
            owner_platform_name: agent.owner_platform_name,
            name: agent.name,
            description: agent.description,
            current_task: agent.current_task,
            last_tool: agent.last_tool,
            last_seen_at: agent.last_seen_at,
            last_request_at: agent.last_request_at,
            request_count: agent.request_count,
            reputation: agent.reputation,
            ratings_count: agent.ratings_count,
            online: agent.online,
        })
        .collect();
    Ok(Json(agents))
}

fn leaderboard_entries_for_profiles(
    state: &AppState,
    profiles: Vec<PublicUserProfile>,
) -> Result<Vec<HumanLeaderboardEntry>, ApiError> {
    let mut stats_by_email = HashMap::new();
    for stat in db_list_human_leaderboard(state)? {
        stats_by_email.insert(normalize_email(&stat.email), stat);
    }

    let mut entries: Vec<_> = profiles
        .into_iter()
        .map(|profile| {
            let stat = stats_by_email.remove(&normalize_email(&profile.email));
            HumanLeaderboardEntry {
                email: profile.email.clone(),
                platform_name: profile.platform_name.clone(),
                login: profile.login.clone(),
                requests_handled: stat.as_ref().map(|stat| stat.requests_handled).unwrap_or(0),
                sent_tokens: stat.as_ref().map(|stat| stat.sent_tokens).unwrap_or(0),
                latest_answered_at: stat.and_then(|stat| stat.latest_answered_at),
                reputation: profile.reputation,
                ratings_count: profile.ratings_count,
                reputation_breakdown: profile.reputation_breakdown,
                profile: profile.profile,
                tags: profile.tags,
                online: profile.online,
                online_sources: profile.online_sources,
            }
        })
        .collect();
    entries.sort_by(|left, right| {
        right
            .requests_handled
            .cmp(&left.requests_handled)
            .then_with(|| right.sent_tokens.cmp(&left.sent_tokens))
            .then_with(|| {
                right
                    .reputation
                    .partial_cmp(&left.reputation)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| right.ratings_count.cmp(&left.ratings_count))
            .then_with(|| left.email.cmp(&right.email))
    });
    Ok(entries)
}

async fn list_agent_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AgentTaskQuery>,
) -> Result<Json<Vec<AgentTask>>, ApiError> {
    let session = require_session(&state, &headers)?;
    let email = normalize_email(&session.user.email);
    Ok(Json(db_list_agent_tasks(
        &state,
        &email,
        query.status.as_ref(),
        query.include_archived,
        200,
    )?))
}

async fn update_agent_task_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(payload): Json<AgentTaskUpdate>,
) -> Result<Json<AgentTask>, ApiError> {
    let session = require_session(&state, &headers)?;
    let actor = normalize_email(&session.user.email);
    let mut task =
        db_get_agent_task(&state, id)?.ok_or_else(|| ApiError::bad_request("task not found"))?;
    if !same_user_identity(&state, &task.assigned_to, &actor) {
        return Err(ApiError::unauthorized("task is assigned to another user"));
    }

    let now = now_unix();
    task.status = payload.status;
    task.updated_at = now;
    task.human_note = normalize_optional_value(payload.note.as_deref());
    task.completed_at = if task.status == AgentTaskStatus::Done {
        Some(now)
    } else if task.status == AgentTaskStatus::Archived {
        task.completed_at
    } else {
        None
    };
    db_store_agent_task(&state, &task)?;
    let _ = state
        .events
        .send(ServerEvent::TaskUpdated { task: task.clone() });
    Ok(Json(task))
}

async fn clear_trash(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let email = normalize_email(&session.user.email);
    let hidden = db_hidden_request_ids(&state, &email)?;
    let ids: Vec<_> = state
        .trash
        .iter()
        .filter(|entry| {
            can_access_request(&state, &email, &entry.value().request) && !hidden.contains(entry.key())
        })
        .map(|entry| *entry.key())
        .collect();
    for id in &ids {
        db_hide_human_request(&state, &email, *id)?;
    }
    let removed_count = ids.len();
    let _ = state
        .events
        .send(ServerEvent::TrashCleaned { removed_count });
    Ok(Json(json!({ "ok": true, "removed_count": removed_count })))
}

async fn hide_request(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let email = normalize_email(&session.user.email);
    let request = if let Some(request) = state.requests.get(&id).map(|entry| entry.clone()) {
        request
    } else if let Some(expired) = state.trash.get(&id).map(|entry| entry.request.clone()) {
        expired
    } else {
        db_get_request(&state, id)?
            .map(|(request, _)| request)
            .ok_or_else(|| ApiError::bad_request("request not found"))?
    };
    if !can_access_request(&state, &email, &request) {
        return Err(ApiError::unauthorized("request belongs to another user"));
    }
    let changed = db_hide_human_request(&state, &email, id)?;
    Ok(Json(json!({ "ok": true, "hidden": changed })))
}

async fn list_online_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PublicUserProfile>>, ApiError> {
    let session = require_session(&state, &headers)?;
    Ok(Json(
        visible_user_profiles_for_session(&state, &session.user.email, None, None)?
            .into_iter()
            .filter(|profile| profile.online)
            .collect(),
    ))
}

async fn search_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<PublicUserProfile>>, ApiError> {
    let session = require_session(&state, &headers)?;
    Ok(Json(visible_user_profiles_for_session(
        &state,
        &session.user.email,
        query.q.as_deref(),
        query.tag.as_deref(),
    )?))
}

async fn public_user_profile(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<PublicUserProfile>, ApiError> {
    let profile = public_profile_by_platform_name(&state, &username)?
        .ok_or_else(|| ApiError::bad_request("profile not found"))?;
    Ok(Json(profile))
}

async fn list_tags(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    Ok(Json(
        json!({ "tags": visible_tag_counts_for_session(&state, &session.user.email)? }),
    ))
}

async fn list_connected_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ConnectedAgent>>, ApiError> {
    let session = require_session(&state, &headers)?;
    Ok(Json(db_list_connected_agents(
        &state,
        &session.user.email,
        100,
    )?))
}

async fn create_agent_friend_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<AgentPanelMessageCreate>,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    if !db_agent_exists(&state, &id)? {
        return Err(ApiError::bad_request("agent not found"));
    }
    let status = db_request_agent_friend(&state, &id, &session.user.email, &payload.body)?;
    Ok(Json(json!({ "ok": true, "relation_status": status })))
}

async fn accept_agent_friend_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    if !db_agent_exists(&state, &id)? {
        return Err(ApiError::bad_request("agent not found"));
    }
    let status = db_agent_relation_status(&state, &id, &session.user.email)?;
    if status != AgentRelationStatus::AgentRequested && status != AgentRelationStatus::Friends {
        return Err(ApiError::bad_request("agent friend request not found"));
    }
    let status = db_accept_agent_friend(&state, &id, &session.user.email)?;
    Ok(Json(json!({ "ok": true, "relation_status": status })))
}

async fn create_agent_ask_me_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<AgentAskMeArgs>,
) -> Result<Json<AgentHumanMessage>, ApiError> {
    let session = require_session(&state, &headers)?;
    if !db_agent_exists(&state, &id)? {
        return Err(ApiError::bad_request("agent not found"));
    }
    let body = normalize_optional_value(Some(payload.body.as_str()))
        .or_else(|| normalize_optional_value(Some(payload.prompt.as_str())))
        .or_else(|| normalize_optional_value(Some(payload.title.as_str())))
        .ok_or_else(|| ApiError::bad_request("memo body is required"))?;
    let message = db_create_agent_memo_from_human(&state, &id, &session.user.email, &body)?;
    Ok(Json(message))
}

async fn rate_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<RateAgentRequest>,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    if !payload.score.is_finite() || !(0.0..=10.0).contains(&payload.score) {
        return Err(ApiError::bad_request("score must be a number from 0 to 10"));
    }
    let reputation = db_store_agent_rating(
        &state,
        &id,
        &session.user.email,
        payload.score,
        payload.note.as_deref(),
    )?;
    Ok(Json(json!({ "ok": true, "reputation": reputation })))
}

async fn rate_human(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RateHumanRequest>,
) -> Result<Json<Value>, ApiError> {
    let _session = require_session(&state, &headers)?;
    let _payload = payload;
    Err(ApiError::bad_request(
        "humans can rate agents; humans are rated through agent feedback",
    ))
}

async fn report_human(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ReportHumanRequest>,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let report = report_human_from_actor(&state, &session.user.email, payload)?;
    Ok(Json(json!({ "ok": true, "report": report })))
}

async fn list_human_memos(
    State(state): State<AppState>,
    Path(email): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<HumanMemo>>, ApiError> {
    let session = require_session(&state, &headers)?;
    let target = resolve_visible_human_memo_target(&state, &session.user.email, &email)?;
    let memos = db_list_human_memos(&state, &target, 50)?;
    if same_user_identity(&state, &session.user.email, &target) {
        let _ = db_mark_human_memos_read(&state, &target, &session.user.email)?;
    }
    Ok(Json(memos))
}

async fn unread_human_memos(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<HumanMemoUnreadSummary>, ApiError> {
    let session = require_session(&state, &headers)?;
    Ok(Json(db_unread_human_memo_summary(
        &state,
        &session.user.email,
        &session.user.email,
    )?))
}

async fn create_human_memo(
    State(state): State<AppState>,
    Path(email): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateHumanMemo>,
) -> Result<Json<HumanMemo>, ApiError> {
    let session = require_session(&state, &headers)?;
    let actor = normalize_email(&session.user.email);
    let target = resolve_visible_human_memo_target(&state, &actor, &email)?;
    let memo = db_create_human_memo(
        &state,
        &target,
        &actor,
        &payload.body,
    )?;
    let _ = state.events.send(ServerEvent::MemoCreated { memo: memo.clone() });
    Ok(Json(memo))
}

fn resolve_visible_human_memo_target(
    state: &AppState,
    viewer_email: &str,
    target_email: &str,
) -> Result<String, ApiError> {
    let target = normalize_email(target_email);
    if target.is_empty() {
        return Err(ApiError::bad_request("target human is required"));
    }
    if normalize_email(viewer_email) == target {
        return Ok(target);
    }
    let visible = visible_user_profiles_for_session(state, viewer_email, None, None)?;
    visible
        .into_iter()
        .map(|profile| normalize_email(&profile.email))
        .find(|email| email == &target)
        .ok_or_else(|| ApiError::unauthorized("target human is not visible to this user"))
}

async fn list_friends(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let email = canonical_user_key_from_email(&state, &session.user.email);
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let Some(record) = users.users.get(&email) else {
        return Ok(Json(
            json!({ "friends": [], "incoming": [], "outgoing": [] }),
        ));
    };
    let friend_emails = record.friends.clone();
    let incoming_emails = record.friend_requests.clone();
    let outgoing_emails: Vec<_> = users
        .users
        .values()
        .filter(|candidate| {
            candidate
                .friend_requests
                .iter()
                .any(|requester| normalize_email(requester) == email)
        })
        .map(canonical_user_key_from_record)
        .collect();
    drop(users);
    let friends = relation_profiles(&state, &friend_emails, &email)?;
    let incoming = relation_profiles(&state, &incoming_emails, &email)?;
    let outgoing = relation_profiles(&state, &outgoing_emails, &email)?;
    Ok(Json(json!({
        "friends": friends,
        "incoming": incoming,
        "outgoing": outgoing
    })))
}

async fn create_friend_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<FriendRequestCreate>,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let requester = canonical_user_key_from_email(&state, &session.user.email);
    let target = find_friend_target(
        &state,
        payload.email.as_deref(),
        payload.intro_code.as_deref(),
    )?;
    if target == requester {
        return Err(ApiError::bad_request("cannot add yourself as a friend"));
    }
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let requester_exists = users.users.contains_key(&requester);
    if !requester_exists {
        let now = now_unix();
        users.insert(new_user_record(
            requester.clone(),
            now,
            default_profile_template(&requester),
        ));
    }
    let target_record = users
        .users
        .get_mut(&target)
        .ok_or_else(|| ApiError::bad_request("target user not found"))?;
    prepare_user_record(target_record);
    if target_record
        .friends
        .iter()
        .any(|friend| friend == &requester)
    {
        return Ok(Json(json!({ "ok": true, "status": "already_friends" })));
    }
    if !target_record
        .friend_requests
        .iter()
        .any(|candidate| candidate == &requester)
    {
        target_record.friend_requests.push(requester);
        target_record.friend_requests =
            normalize_email_list(std::mem::take(&mut target_record.friend_requests));
    }
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save friend request: {err}")))?;
    Ok(Json(
        json!({ "ok": true, "status": "requested", "target": target }),
    ))
}

async fn accept_friend_request(
    State(state): State<AppState>,
    Path(email): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let admin_email = normalize_email(&state.config.admin_email);
    let current = canonical_user_key_from_identifier(&users, &session.user.email, &admin_email)
        .unwrap_or_else(|| normalize_email(&session.user.email));
    let requester = canonical_user_key_from_identifier(&users, &email, &admin_email)
        .ok_or_else(|| ApiError::bad_request("requester not found"))?;
    if !users.users.contains_key(&requester) {
        return Err(ApiError::bad_request("requester not found"));
    }
    {
        let current_record = users
            .users
            .get_mut(&current)
            .ok_or_else(|| ApiError::bad_request("current user not found"))?;
        current_record
            .friend_requests
            .retain(|candidate| normalize_email(candidate) != requester);
        push_unique_email(&mut current_record.friends, &requester);
    }
    {
        let requester_record = users
            .users
            .get_mut(&requester)
            .ok_or_else(|| ApiError::bad_request("requester not found"))?;
        push_unique_email(&mut requester_record.friends, &current);
    }
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to accept friend request: {err}")))?;
    Ok(Json(json!({ "ok": true, "friend": requester })))
}

async fn remove_friend(
    State(state): State<AppState>,
    Path(email): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let admin_email = normalize_email(&state.config.admin_email);
    let current = canonical_user_key_from_identifier(&users, &session.user.email, &admin_email)
        .unwrap_or_else(|| normalize_email(&session.user.email));
    let other = canonical_user_key_from_identifier(&users, &email, &admin_email)
        .unwrap_or_else(|| normalize_email(&email));
    if let Some(record) = users.users.get_mut(&current) {
        remove_email(&mut record.friends, &other);
        remove_email(&mut record.friend_requests, &other);
    }
    if let Some(record) = users.users.get_mut(&other) {
        remove_email(&mut record.friends, &current);
        remove_email(&mut record.friend_requests, &current);
    }
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to remove friend: {err}")))?;
    Ok(Json(json!({ "ok": true, "friend": other })))
}

#[cfg(test)]
fn rate_human_from_actor(
    state: &AppState,
    actor_email: &str,
    payload: RateHumanRequest,
) -> Result<ReputationSummary, ApiError> {
    let actor = canonical_user_key_from_email(state, actor_email);
    let target = resolve_human_interaction_target(state, &actor, &payload.rated_email)?;
    if !payload.score.is_finite() || !(0.0..=10.0).contains(&payload.score) {
        return Err(ApiError::bad_request("score must be a number from 0 to 10"));
    }
    db_store_human_rating(
        state,
        &target,
        &actor,
        payload.score,
        payload.note.as_deref(),
    )
}

fn report_human_from_actor(
    state: &AppState,
    actor_email: &str,
    payload: ReportHumanRequest,
) -> Result<HumanReport, ApiError> {
    let actor = canonical_user_key_from_email(state, actor_email);
    let target = resolve_human_interaction_target(state, &actor, &payload.reported_email)?;
    let reason = payload.reason.trim();
    if reason.is_empty() {
        return Err(ApiError::bad_request("report reason is required"));
    }
    if reason.chars().count() > 2000 {
        return Err(ApiError::bad_request("report reason is too long"));
    }
    let report = db_create_human_report(state, &actor, &target, reason)?;
    let note = format!("Report: {reason}");
    match db_store_human_rating(state, &target, &actor, 0.0, Some(&note)) {
        Ok(_) => {}
        Err(err) if err.status == StatusCode::CONFLICT => {}
        Err(err) => return Err(err),
    }
    Ok(report)
}

fn resolve_human_interaction_target(
    state: &AppState,
    actor_email: &str,
    target_email: &str,
) -> Result<String, ApiError> {
    let actor = normalize_email(actor_email);
    let target = normalize_email(target_email);
    if target.is_empty() {
        return Err(ApiError::bad_request("target human is required"));
    }
    if actor == target {
        return Err(ApiError::bad_request("cannot rate or report yourself"));
    }
    if target == normalize_email(&state.config.admin_email) {
        return Ok(target);
    }
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let admin_email = normalize_email(&state.config.admin_email);
    if let Some(target) = canonical_user_key_from_identifier(&users, &target, &admin_email) {
        if actor == target {
            return Err(ApiError::bad_request("cannot rate or report yourself"));
        }
        Ok(target)
    } else {
        Err(ApiError::bad_request("target human not found"))
    }
}

fn relation_profiles(
    state: &AppState,
    emails: &[String],
    viewer_email: &str,
) -> Result<Vec<PublicUserProfile>, ApiError> {
    let online = online_presence_sources(state);
    let reputations = db_reputation_map(state)?;
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let viewer_email = normalize_email(viewer_email);
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
    let mut profiles = Vec::new();
    for email in emails {
        let Some(key) = canonical_user_key_from_identifier(&users, email, &admin_email) else {
            continue;
        };
        if let Some(record) = users.users.get(&key) {
            profiles.push(public_profile_from_record_for_viewer(
                record,
                &admin_email,
                &online,
                reputation_for(&reputations, &profile_user_key(record, &admin_email)),
                &viewer_key,
                &friends,
                &incoming,
            ));
        }
    }
    sort_profiles_by_reputation(&mut profiles);
    Ok(profiles)
}

fn find_friend_target(
    state: &AppState,
    email: Option<&str>,
    intro_code: Option<&str>,
) -> Result<String, ApiError> {
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let admin_email = normalize_email(&state.config.admin_email);
    if let Some(email) = email.map(normalize_email).filter(|email| !email.is_empty()) {
        if let Some(key) = canonical_user_key_from_identifier(&users, &email, &admin_email) {
            return Ok(key);
        }
    }
    if let Some(intro_code) = intro_code.map(str::trim).filter(|code| !code.is_empty()) {
        if let Some(record) = users
            .users
            .values()
            .find(|record| record.intro_code.eq_ignore_ascii_case(intro_code))
        {
            return Ok(canonical_user_key_from_record(record));
        }
    }
    Err(ApiError::bad_request("target user not found"))
}

fn push_unique_email(values: &mut Vec<String>, email: &str) {
    let email = normalize_email(email);
    if !values.iter().any(|value| normalize_email(value) == email) {
        values.push(email);
    }
    *values = normalize_email_list(std::mem::take(values));
}

fn remove_email(values: &mut Vec<String>, email: &str) {
    let email = normalize_email(email);
    values.retain(|value| normalize_email(value) != email);
}

async fn admin_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PublicUserProfile>>, ApiError> {
    require_admin(&state, &headers)?;
    Ok(Json(user_profiles(&state, None, None)?))
}

async fn admin_add_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AdminUserRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let email = normalize_email(&payload.email);
    validate_email_like_identifier(&email)?;
    let now = now_unix();
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let mut record = new_user_record(email.clone(), now, payload.profile);
    record.last_login_at = 0;
    record.tags = normalize_tags(payload.tags);
    prepare_user_record(&mut record);
    let key = user_record_key(&record);
    if users.users.contains_key(&key) {
        return Err(ApiError::conflict("platform name is already in use"));
    }
    users.users.insert(key, record);
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save user: {err}")))?;
    Ok(Json(json!({ "ok": true, "email": email })))
}

async fn admin_update_user(
    State(state): State<AppState>,
    Path(email): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<AdminUserUpdate>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let email = normalize_email(&email);
    let is_admin = email == normalize_email(&state.config.admin_email);
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let now = now_unix();
    let record = if is_admin {
        users.users.entry(email.clone()).or_insert_with(|| {
            let mut record = new_user_record(
                state.config.admin_email.clone(),
                now,
                "Administrator".to_string(),
            );
            record.last_login_at = 0;
            record
        })
    } else {
        users
            .users
            .get_mut(&email)
            .ok_or_else(|| ApiError::bad_request("user not found"))?
    };
    if let Some(profile) = payload.profile {
        record.profile = profile;
    }
    if let Some(tags) = payload.tags {
        record.tags = normalize_tags(tags);
    }
    if is_admin {
        record.ban_expires_at = None;
    } else if let Some(ban_expires_at) = payload.ban_expires_at {
        record.ban_expires_at = ban_expires_at;
    }
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save user: {err}")))?;
    Ok(Json(json!({ "ok": true })))
}

async fn admin_kick_user(
    State(state): State<AppState>,
    Path(email): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let email = normalize_email(&email);
    if email == normalize_email(&state.config.admin_email) {
        return Err(ApiError::bad_request("admin user cannot be kicked"));
    }
    let mut removed_count = 0;
    state.sessions.retain(|_, session| {
        let keep = normalize_email(&session.user.email) != email;
        if !keep {
            removed_count += 1;
        }
        keep
    });
    Ok(Json(json!({ "ok": true, "removed_count": removed_count })))
}

async fn admin_reports(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<HumanReport>>, ApiError> {
    require_admin(&state, &headers)?;
    Ok(Json(db_list_human_reports(&state, 200)?))
}

async fn admin_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminSettings>, ApiError> {
    require_admin(&state, &headers)?;
    let settings = state
        .admin_settings
        .lock()
        .map_err(|_| ApiError::internal("settings lock poisoned"))?
        .clone();
    Ok(Json(admin_settings_response(&state, settings)))
}

async fn admin_update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AdminSettings>,
) -> Result<Json<AdminSettings>, ApiError> {
    require_admin(&state, &headers)?;
    let mut payload = payload;
    merge_admin_secret_settings(&state, &mut payload)?;
    let sanitized = sanitize_admin_settings(payload);
    {
        let mut settings = state
            .admin_settings
            .lock()
            .map_err(|_| ApiError::internal("settings lock poisoned"))?;
        *settings = sanitized.clone();
    }
    persist_admin_settings(&state, &sanitized)?;
    Ok(Json(admin_settings_response(&state, sanitized)))
}

async fn answer_request(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(payload): Json<AnswerRequest>,
) -> Result<Json<Value>, ApiError> {
    let session = require_session(&state, &headers)?;
    let actor_email = normalize_email(&session.user.email);
    let answer = HumanAnswer {
        answer: payload.answer,
        note: payload.note,
        answered_by: session.user.email,
        answered_at: now_unix(),
    };
    let answered = answer_request_internal(&state, id, Some(&actor_email), answer)?;
    Ok(Json(json!({
        "ok": true,
        "request": answered.request,
        "answer": answered.answer,
        "late": answered.answered_late
    })))
}

fn answer_request_internal(
    state: &AppState,
    id: Uuid,
    actor_email: Option<&str>,
    answer: HumanAnswer,
) -> Result<AnsweredRequest, ApiError> {
    let now = answer.answered_at;

    let mut late = false;
    let request = if let Some((_, request)) = state.requests.remove(&id) {
        if actor_email.is_some_and(|email| !can_access_request(state, email, &request)) {
            state.requests.insert(id, request);
            return Err(ApiError::unauthorized(
                "request is assigned to another user",
            ));
        }
        if now > request.expires_at {
            late = true;
        }
        request
    } else if let Some(expired) = state.trash.get(&id).map(|entry| entry.value().clone()) {
        if actor_email.is_some_and(|email| !can_access_request(state, email, &expired.request)) {
            return Err(ApiError::unauthorized(
                "request is assigned to another user",
            ));
        }
        state.trash.remove(&id);
        late = true;
        expired.request
    } else if let Some((request, status)) = db_get_request(state, id)? {
        if actor_email.is_some_and(|email| !can_access_request(state, email, &request)) {
            return Err(ApiError::unauthorized(
                "request is assigned to another user",
            ));
        }
        late = status == "expired" || now > request.expires_at;
        request
    } else {
        return Err(ApiError::bad_request("request not found"));
    };

    if let Some((_, waiter)) = state.waiters.remove(&id) {
        if waiter.send(answer.clone()).is_err() {
            late = true;
            warn!(%id, "MCP caller already disconnected before human answer");
        }
    }
    db_store_answer(state, &request, &answer, late)?;
    let _ = state.events.send(ServerEvent::RequestAnswered {
        id,
        request: request.clone(),
        answer: answer.clone(),
        answered_late: late,
    });
    Ok(AnsweredRequest {
        request,
        answer,
        answered_late: late,
    })
}
