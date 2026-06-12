const HUMEN_REPLY_AVAILABLE_NOTIFICATION: &str = "notifications/humen/reply_available";

async fn mcp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<McpRequest>,
) -> Result<Response, ApiError> {
    if payload.jsonrpc.as_deref() != Some("2.0") {
        return Ok(Json(mcp_error(
            payload.id,
            -32600,
            "expected JSON-RPC 2.0 request",
        ))
        .into_response());
    }
    let agent = match require_agent_access(&state, &headers) {
        Ok(agent) => agent,
        Err(err) => {
            if payload.id.is_none() {
                return Ok(StatusCode::ACCEPTED.into_response());
            }
            return Ok(Json(mcp_error(payload.id, -32003, err.message)).into_response());
        }
    };
    let agent = db_touch_agent_connection(&state, &agent, &headers, &payload)?;

    let id = payload.id.clone();
    let result = match payload.method.as_str() {
        "initialize" => json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {
                "tools": {},
                "experimental": {
                    "humenNotifications": {
                        "transport": "streamable-http-sse",
                        "replyAvailableMethod": HUMEN_REPLY_AVAILABLE_NOTIFICATION,
                        "fallbackTool": "read_humen_replies"
                    }
                }
            },
            "serverInfo": {
                "name": "humen-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
        "notifications/initialized" => return Ok(StatusCode::ACCEPTED.into_response()),
        "tools/list" => json!({
            "tools": [
                {
                    "name": "approve",
                    "description": "Ask the attached human to approve or deny a proposed action and wait for the answer.",
                    "inputSchema": approve_schema()
                },
                {
                    "name": "judge",
                    "description": "Ask the attached human for a yes/no judgment and wait for the answer.",
                    "inputSchema": judge_schema()
                },
                {
                    "name": "feedback",
                    "description": "Ask the attached human for short free-form feedback and wait for the answer.",
                    "inputSchema": feedback_schema()
                },
                {
                    "name": "ask_humen",
                    "description": "Ask a logged-in human to complete a simple task and wait for the answer. For non-blocking background calls, prefer the ask_humen_*_async tools.",
                    "inputSchema": ask_humen_schema()
                },
                {
                    "name": "ask_humen_async",
                    "description": "Create a human request and return immediately with request_id. Prefer GET /mcp notifications for reply_available; fall back to read_humen_replies polling when notifications are unavailable.",
                    "inputSchema": ask_humen_async_schema()
                },
                {
                    "name": "ask_humen_text_async",
                    "description": "Create a non-blocking text-answer human request.",
                    "inputSchema": ask_humen_text_async_schema()
                },
                {
                    "name": "ask_humen_choice_async",
                    "description": "Create a non-blocking choice human request.",
                    "inputSchema": ask_humen_choice_async_schema()
                },
                {
                    "name": "ask_humen_judgment_async",
                    "description": "Create a non-blocking yes/no judgment request. The human UI renders this as check and X buttons.",
                    "inputSchema": ask_humen_judgment_async_schema()
                },
                {
                    "name": "read_humen_replies",
                    "description": "Read completed human replies for the user attached to this agent secret, optionally filtered by request_id. Use after reply_available notifications or as a polling fallback.",
                    "inputSchema": read_humen_replies_schema()
                },
                {
                    "name": "list_humen_nodes",
                    "description": "List configured child humen-mcp federation nodes visible from this server. Secrets are never returned.",
                    "inputSchema": json!({ "type": "object", "properties": {} })
                },
                {
                    "name": "search_humen_network",
                    "description": "Search visible human profiles across configured child humen-mcp nodes.",
                    "inputSchema": search_humen_network_schema()
                },
                {
                    "name": "ask_humen_network_async",
                    "description": "Create a non-blocking human request on a configured child humen-mcp node and collect the answer through read_humen_replies.",
                    "inputSchema": ask_humen_network_async_schema()
                },
                {
                    "name": "read_humen_network_ledger",
                    "description": "Read recent local federation ledger entries. The ledger is hash-chained for tamper-evident cross-node request auditing.",
                    "inputSchema": read_humen_network_ledger_schema()
                },
                {
                    "name": "list_humen_plugins",
                    "description": "List loaded community plugins, including request templates, route strategies, scoring rules, and third-party channels.",
                    "inputSchema": list_humen_plugins_schema()
                },
                {
                    "name": "create_humen_request_from_template",
                    "description": "Create a human request from a community plugin request template and return immediately with request_id.",
                    "inputSchema": create_humen_request_from_template_schema()
                },
                {
                    "name": "create_humen_task",
                    "description": "Create a visible AI task for the human account attached to this agent secret.",
                    "inputSchema": create_humen_task_schema()
                },
                {
                    "name": "list_humen_tasks",
                    "description": "List AI-created tasks for the human account attached to this agent secret.",
                    "inputSchema": list_humen_tasks_schema()
                },
                {
                    "name": "leave_humen_memo",
                    "description": "Leave an offline memo on a visible human's memo board.",
                    "inputSchema": leave_humen_memo_schema()
                },
                {
                    "name": "list_agent_inbox",
                    "description": "List pending human-to-agent messages, including friend requests and requests asking this agent to ask that human.",
                    "inputSchema": list_agent_inbox_schema()
                },
                {
                    "name": "request_human_friend",
                    "description": "Send a friend request from this connected agent to a visible human.",
                    "inputSchema": request_human_friend_schema()
                },
                {
                    "name": "accept_human_friend",
                    "description": "Accept a human's pending friend request to this connected agent.",
                    "inputSchema": request_human_friend_schema()
                },
                {
                    "name": "list_online_humens",
                    "description": "List online human operators and their public profiles.",
                    "inputSchema": json!({ "type": "object", "properties": {} })
                },
                {
                    "name": "search_humen_profiles",
                    "description": "Search human profiles by text or #tag.",
                    "inputSchema": json!({
                        "type": "object",
                        "properties": {
                            "q": { "type": "string" },
                            "tag": { "type": "string" }
                        }
                    })
                },
                {
                    "name": "list_humen_tags",
                    "description": "List known #tags and their usage counts.",
                    "inputSchema": json!({ "type": "object", "properties": {} })
                },
                {
                    "name": "rate_humen",
                    "description": "Rate a human from 0 to 10. Reputation blends GitHub seed priors with feedback weighted by the rater's own reputation; unrated humans start at 5.",
                    "inputSchema": rate_humen_schema()
                },
                {
                    "name": "report_humen",
                    "description": "Report a human to the administrator mailbox and apply a zero-score feedback signal weighted by this actor's reputation.",
                    "inputSchema": report_humen_schema()
                }
            ]
        }),
        "tools/call" => return Ok(call_tool(state, agent, payload).await?.into_response()),
        _ => {
            if id.is_none() {
                return Ok(StatusCode::ACCEPTED.into_response());
            }
            return Ok(Json(mcp_error(id, -32601, "method not found")).into_response());
        }
    };

    Ok(Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    }))
    .into_response())
}

struct McpSseStreamState {
    state: AppState,
    agent_email: String,
    stream_id: Uuid,
    events: broadcast::Receiver<ServerEvent>,
    shutdown: broadcast::Receiver<()>,
}

impl Drop for McpSseStreamState {
    fn drop(&mut self) {
        cleanup_mcp_stream(&self.state, &self.agent_email, self.stream_id);
    }
}

fn mcp_accepts_sse(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| {
            accept
                .split(',')
                .any(|part| part.trim().starts_with("text/event-stream"))
        })
}

fn mcp_sse(
    state: AppState,
    agent: AgentContext,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let agent_email = normalize_email(&agent.email);
    let stream_id = Uuid::new_v4();
    state.mcp_streams.insert(agent_email.clone(), stream_id);
    let stream_state = McpSseStreamState {
        events: state.events.subscribe(),
        shutdown: state.shutdown.subscribe(),
        state,
        agent_email,
        stream_id,
    };
    let stream = stream::unfold(stream_state, |mut stream_state| async move {
        loop {
            tokio::select! {
                _ = stream_state.shutdown.recv() => {
                    cleanup_mcp_stream(&stream_state.state, &stream_state.agent_email, stream_state.stream_id);
                    return None;
                }
                event = stream_state.events.recv() => {
                    match event {
                        Ok(event) => {
                            if !is_active_mcp_stream(&stream_state.state, &stream_state.agent_email, stream_state.stream_id) {
                                continue;
                            }
                            let Some(event) = mcp_sse_event(&stream_state.state, &stream_state.agent_email, &event) else {
                                continue;
                            };
                            return Some((Ok(event), stream_state));
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => {
                            cleanup_mcp_stream(&stream_state.state, &stream_state.agent_email, stream_state.stream_id);
                            return None;
                        }
                    }
                }
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn is_active_mcp_stream(state: &AppState, agent_email: &str, stream_id: Uuid) -> bool {
    state
        .mcp_streams
        .get(agent_email)
        .is_some_and(|active| *active.value() == stream_id)
}

fn cleanup_mcp_stream(state: &AppState, agent_email: &str, stream_id: Uuid) {
    let active = state
        .mcp_streams
        .get(agent_email)
        .map(|entry| *entry.value());
    if active == Some(stream_id) {
        state.mcp_streams.remove(agent_email);
    }
}

fn mcp_sse_event(state: &AppState, agent_email: &str, event: &ServerEvent) -> Option<Event> {
    match event {
        ServerEvent::RequestAnswered {
            id,
            request,
            answered_late,
            ..
        } if can_access_request(state, agent_email, request)
            || agent_created_request(state, agent_email, request) =>
        {
            let message = mcp_reply_available_notification(*id, request, *answered_late);
            Some(
                Event::default()
                    .event("message")
                    .id(format!("reply-available-{id}"))
                    .data(message.to_string()),
            )
        }
        _ => None,
    }
}

fn mcp_reply_available_notification(id: Uuid, request: &HumanRequest, answered_late: bool) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": HUMEN_REPLY_AVAILABLE_NOTIFICATION,
        "params": {
            "request_id": id,
            "title": request.title.clone(),
            "answered_late": answered_late,
            "reply_tool": "read_humen_replies",
            "fallback_tool": "read_humen_replies"
        }
    })
}

async fn call_tool(
    state: AppState,
    agent: AgentContext,
    payload: McpRequest,
) -> Result<Json<Value>, ApiError> {
    let id = payload.id.clone();
    let name = payload
        .params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("tools/call params.name is required"))?;
    match name {
        "approve" => {
            call_blocking_request_shortcut(state, agent, id, &payload, name, TaskKind::Judgment)
                .await
        }
        "judge" => {
            call_blocking_request_shortcut(state, agent, id, &payload, name, TaskKind::Judgment)
                .await
        }
        "feedback" => {
            call_blocking_request_shortcut(state, agent, id, &payload, name, TaskKind::Text).await
        }
        "ask_humen" => {
            let create = parse_humen_request_arguments(&payload, name, None)?;
            create_humen_request(state, agent, id, create, false).await
        }
        "ask_humen_async" => {
            let create = parse_humen_request_arguments(&payload, name, None)?;
            create_humen_request(state, agent, id, create, true).await
        }
        "ask_humen_text_async" => {
            let create = parse_humen_request_arguments(&payload, name, Some(TaskKind::Text))?;
            create_humen_request(state, agent, id, create, true).await
        }
        "ask_humen_choice_async" => {
            let create = parse_humen_request_arguments(&payload, name, Some(TaskKind::Choice))?;
            create_humen_request(state, agent, id, create, true).await
        }
        "ask_humen_judgment_async" => {
            let create = parse_humen_request_arguments(&payload, name, Some(TaskKind::Judgment))?;
            create_humen_request(state, agent, id, create, true).await
        }
        "read_humen_replies" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: ReadLateRepliesArgs = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid read_humen_replies arguments: {err}"))
            })?;
            collect_federated_replies_for_agent(&state, &agent).await?;
            Ok(Json(mcp_text_result(
                id,
                json!({ "replies": db_read_humen_replies(&state, &agent.email, args)? }),
            )))
        }
        "list_humen_nodes" => list_humen_nodes(&state, id).await,
        "search_humen_network" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: NetworkSearchArgs = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid search_humen_network arguments: {err}"))
            })?;
            search_humen_network(&state, &agent, id, args).await
        }
        "ask_humen_network_async" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: NetworkAskHumanRequest = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid ask_humen_network_async arguments: {err}"))
            })?;
            ask_humen_network_async(state, agent, id, args).await
        }
        "read_humen_network_ledger" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: ReadNetworkLedgerArgs = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid read_humen_network_ledger arguments: {err}"))
            })?;
            read_humen_network_ledger(&state, id, args).await
        }
        "list_humen_plugins" => Ok(Json(mcp_text_result(id, state.plugins.plugin_summary()))),
        "create_humen_request_from_template" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: TemplateRequestArgs = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!(
                    "invalid create_humen_request_from_template arguments: {err}"
                ))
            })?;
            let create = create_request_from_template_args(&state.plugins, args)?;
            create_humen_request(state, agent, id, create, true).await
        }
        "create_humen_task" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: CreateAgentTask = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid create_humen_task arguments: {err}"))
            })?;
            let task = create_agent_task_from_agent(&state, &agent, args)?;
            Ok(Json(mcp_text_result(id, json!({ "task": task }))))
        }
        "list_humen_tasks" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let query: AgentTaskQuery = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid list_humen_tasks arguments: {err}"))
            })?;
            let tasks = db_list_agent_tasks(
                &state,
                &agent.email,
                query.status.as_ref(),
                query.include_archived,
                200,
            )?;
            Ok(Json(mcp_text_result(id, json!({ "tasks": tasks }))))
        }
        "leave_humen_memo" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: LeaveHumanMemoArgs = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid leave_humen_memo arguments: {err}"))
            })?;
            let target = resolve_visible_human_for_agent(&state, &agent, &args.target_human_email)?;
            let memo = db_create_human_memo_with_agent(
                &state,
                &target,
                &agent.email,
                Some(&agent.agent_id),
                Some(&agent.agent_name),
                &args.body,
            )?;
            let _ = state.events.send(ServerEvent::MemoCreated { memo: memo.clone() });
            Ok(Json(mcp_text_result(
                id,
                json!({ "memo": memo, "target_human_email": target }),
            )))
        }
        "list_agent_inbox" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: ListAgentInboxArgs = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid list_agent_inbox arguments: {err}"))
            })?;
            Ok(Json(mcp_text_result(
                id,
                json!({ "messages": db_list_agent_inbox(
                    &state,
                    &agent.agent_id,
                    args.unread_only,
                    args.mark_read,
                    args.limit.unwrap_or(100)
                )? }),
            )))
        }
        "request_human_friend" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: AgentFriendRequestArgs = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid request_human_friend arguments: {err}"))
            })?;
            let human = resolve_visible_human_for_agent(&state, &agent, &args.human_email)?;
            let status = db_request_human_friend_from_agent(
                &state,
                &agent.agent_id,
                &human,
                &args.message,
            )?;
            Ok(Json(mcp_text_result(
                id,
                json!({ "ok": true, "human_email": human, "relation_status": status }),
            )))
        }
        "accept_human_friend" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: AgentFriendRequestArgs = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid accept_human_friend arguments: {err}"))
            })?;
            let human = resolve_human_for_agent_friend_accept(&state, &agent, &args.human_email)?;
            let status = db_accept_agent_friend(&state, &agent.agent_id, &human)?;
            Ok(Json(mcp_text_result(
                id,
                json!({ "ok": true, "human_email": human, "relation_status": status }),
            )))
        }
        "list_online_humens" => {
            let users: Vec<_> = agent_visible_profiles(&state, &agent, None, None)?
                .into_iter()
                .filter(|profile| profile.online)
                .collect();
            Ok(Json(mcp_text_result(id, json!({ "users": users }))))
        }
        "search_humen_profiles" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let q = arguments.get("q").and_then(Value::as_str);
            let tag = arguments.get("tag").and_then(Value::as_str);
            let users = agent_visible_profiles(&state, &agent, q, tag)?;
            Ok(Json(mcp_text_result(id, json!({ "users": users }))))
        }
        "list_humen_tags" => {
            Ok(Json(mcp_text_result(
                id,
                json!({ "tags": agent_visible_tag_counts(&state, &agent)? }),
            )))
        }
        "rate_humen" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: RateHumanRequest = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid rate_humen arguments: {err}"))
            })?;
            let reputation = rate_human_from_agent(&state, &agent, args)?;
            Ok(Json(mcp_text_result(id, json!({ "reputation": reputation }))))
        }
        "report_humen" => {
            let arguments = payload
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let args: ReportHumanRequest = serde_json::from_value(arguments).map_err(|err| {
                ApiError::bad_request(format!("invalid report_humen arguments: {err}"))
            })?;
            let report = report_human_from_actor(&state, &agent.email, args)?;
            Ok(Json(mcp_text_result(id, json!({ "report": report }))))
        }
        _ => Ok(Json(mcp_error(id, -32602, "unknown tool"))),
    }
}

async fn call_blocking_request_shortcut(
    state: AppState,
    agent: AgentContext,
    id: Option<Value>,
    payload: &McpRequest,
    tool_name: &str,
    kind: TaskKind,
) -> Result<Json<Value>, ApiError> {
    let create = parse_blocking_shortcut_arguments(payload, tool_name, kind)?;
    create_humen_request(state, agent, id, create, false).await
}

fn parse_blocking_shortcut_arguments(
    payload: &McpRequest,
    tool_name: &str,
    kind: TaskKind,
) -> Result<CreateHumanRequest, ApiError> {
    let mut create = parse_humen_request_arguments(payload, tool_name, Some(kind))?;
    create.background = false;
    Ok(create)
}

fn parse_humen_request_arguments(
    payload: &McpRequest,
    tool_name: &str,
    kind_override: Option<TaskKind>,
) -> Result<CreateHumanRequest, ApiError> {
    let arguments = payload
        .params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Null);
    let mut create: CreateHumanRequest = serde_json::from_value(arguments)
        .map_err(|err| ApiError::bad_request(format!("invalid {tool_name} arguments: {err}")))?;
    if let Some(kind) = kind_override {
        create.kind = kind;
    }
    Ok(create)
}

async fn create_humen_request(
    state: AppState,
    agent: AgentContext,
    id: Option<Value>,
    mut create: CreateHumanRequest,
    force_background: bool,
) -> Result<Json<Value>, ApiError> {
    create.title = create.title.trim().to_string();
    create.prompt = create.prompt.trim().to_string();
    if create.title.is_empty() {
        return Err(ApiError::bad_request("title is required"));
    }
    if create.prompt.is_empty() {
        return Err(ApiError::bad_request("prompt is required"));
    }
    create.choices = normalize_request_choices(&create.kind, create.choices)?;
    let now = now_unix();
    let timeout_seconds = create.timeout_seconds.clamp(30, 86400);
    let background = force_background || create.background;
    let mut tag_sources = vec![create.title.as_str(), create.prompt.as_str()];
    tag_sources.extend(create.steps.iter().map(String::as_str));
    let tags = extract_tags(&tag_sources);
    let (image_base64, image_mime_type) =
        normalize_image_payload(create.image_base64, create.image_mime_type);
    let assigned_to =
        resolve_request_target_human(&state, &agent, create.target_human_email.as_deref())?;
    let request = HumanRequest {
        id: Uuid::new_v4(),
        kind: create.kind,
        title: create.title,
        prompt: create.prompt,
        choices: create.choices,
        image_url: create.image_url,
        image_base64,
        image_mime_type,
        steps: create.steps,
        created_at: now,
        timeout_seconds,
        expires_at: now.saturating_add(timeout_seconds),
        tags,
        assigned_to: Some(assigned_to),
        created_by: Some(agent.email.clone()),
        created_by_agent_id: normalize_optional_value(Some(agent.agent_id.as_str())),
    };
    let current_task = format!("{}: {}", request.title, request.prompt);
    db_update_agent_current_task(&state, &agent, &current_task)?;
    let timeout = Duration::from_secs(request.timeout_seconds);
    db_insert_request(&state, &request)?;
    let rx = if background {
        None
    } else {
        let (tx, rx) = oneshot::channel();
        state.waiters.insert(request.id, tx);
        Some(rx)
    };
    state.requests.insert(request.id, request.clone());
    let _ = state.events.send(ServerEvent::RequestCreated {
        request: request.clone(),
    });
    dispatch_webhooks(&state, "request_created", "mcp", &request, None);

    if background {
        let request_id = request.id;
        let title = request.title.clone();
        let expires_at = request.expires_at;
        let state_for_expiry = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            if state_for_expiry.requests.get(&request_id).is_some() {
                let _ = expire_request(
                    &state_for_expiry,
                    request_id,
                    format!("Human request timed out after {timeout_seconds} seconds"),
                );
            }
        });
        return Ok(Json(mcp_text_result(
            id,
            json!({
                "status": "pending",
                "request_id": request_id,
                "title": title,
                "timeout_seconds": timeout_seconds,
                "expires_at": expires_at,
                "poll_tool": "read_humen_replies"
            }),
        )));
    }

    let rx = rx.expect("blocking ask_humen must have an answer receiver");
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(answer)) => Ok(Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&answer).unwrap_or_else(|_| answer.answer.clone())
                }]
            }
        }))),
        Ok(Err(_)) => Ok(Json(mcp_error(id, -32000, "human answer channel closed"))),
        Err(_) => {
            state.waiters.remove(&request.id);
            let expired = expire_request(
                &state,
                request.id,
                format!(
                    "Human request timed out after {} seconds",
                    request.timeout_seconds
                ),
            )
            .unwrap_or_else(|| ExpiredRequest {
                request: request.clone(),
                expired_at: now_unix(),
                reason: format!(
                    "Human request timed out after {} seconds",
                    request.timeout_seconds
                ),
            });
            Ok(Json(mcp_error_with_data(
                id,
                -32001,
                &expired.reason,
                json!({
                    "request_id": expired.request.id,
                    "title": expired.request.title,
                    "timeout_seconds": expired.request.timeout_seconds,
                    "expired_at": expired.expired_at,
                    "suggestion": "Try again with a longer timeout or simplify the request."
                }),
            )))
        }
    }
}

fn normalize_request_choices(
    kind: &TaskKind,
    choices: Vec<String>,
) -> Result<Vec<String>, ApiError> {
    if *kind == TaskKind::Judgment {
        return Ok(vec!["yes".to_string(), "no".to_string()]);
    }
    let choices: Vec<_> = choices
        .into_iter()
        .map(|choice| choice.trim().to_string())
        .filter(|choice| !choice.is_empty())
        .collect();
    if *kind == TaskKind::Choice && choices.is_empty() {
        return Err(ApiError::bad_request(
            "choice requests require at least one choice",
        ));
    }
    Ok(choices)
}

fn create_agent_task_from_agent(
    state: &AppState,
    agent: &AgentContext,
    create: CreateAgentTask,
) -> Result<AgentTask, ApiError> {
    let title = create.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::bad_request("task title is required"));
    }
    let description = create.description.trim().to_string();
    let steps: Vec<_> = create
        .steps
        .into_iter()
        .map(|step| step.trim().to_string())
        .filter(|step| !step.is_empty())
        .collect();
    let mut sources = vec![title.as_str(), description.as_str()];
    sources.extend(steps.iter().map(String::as_str));
    let mut tags = normalize_tags(create.tags);
    tags.extend(extract_tags(&sources));
    let tags = normalize_tags(tags);
    let now = now_unix();
    let task = AgentTask {
        id: Uuid::new_v4(),
        title,
        description,
        steps,
        tags,
        created_by: agent.email.clone(),
        assigned_to: agent.email.clone(),
        created_at: now,
        updated_at: now,
        due_at: create.due_at,
        status: AgentTaskStatus::Open,
        human_note: None,
        completed_at: None,
    };
    db_store_agent_task(state, &task)?;
    db_update_agent_current_task(state, agent, &task.title)?;
    let _ = state
        .events
        .send(ServerEvent::TaskCreated { task: task.clone() });
    Ok(task)
}

fn mcp_text_result(id: Option<Value>, value: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
            }]
        }
    })
}

fn approve_schema() -> Value {
    human_shortcut_schema(
        "Describe the action that needs approval, including relevant context and risk.",
    )
}

fn judge_schema() -> Value {
    human_shortcut_schema("Describe the yes/no judgment the human should make.")
}

fn feedback_schema() -> Value {
    human_shortcut_schema("Describe the work or decision that needs human feedback.")
}

fn human_shortcut_schema(prompt_description: &'static str) -> Value {
    json!({
        "type": "object",
        "required": ["title", "prompt"],
        "properties": {
            "title": { "type": "string" },
            "prompt": {
                "type": "string",
                "description": prompt_description
            },
            "steps": {
                "type": "array",
                "items": { "type": "string" }
            },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 30,
                "maximum": 86400,
                "default": 60
            },
            "target_human_email": {
                "type": "string",
                "description": "Optional visible human profile email/key to ask. Omit to ask the human attached to this agent secret."
            }
        }
    })
}

fn ask_humen_schema() -> Value {
    json!({
        "type": "object",
        "required": ["title", "prompt"],
        "properties": {
            "kind": {
                "type": "string",
                "enum": ["choice", "judgment", "text", "image_review", "steps"],
                "default": "text"
            },
            "title": { "type": "string" },
            "prompt": { "type": "string" },
            "choices": {
                "type": "array",
                "items": { "type": "string" }
            },
            "image_url": { "type": "string" },
            "image_base64": {
                "type": "string",
                "description": "Raw base64 image bytes, or a data:image/...;base64,... URL."
            },
            "image_mime_type": {
                "type": "string",
                "default": "image/png",
                "description": "MIME type for image_base64, e.g. image/png or image/jpeg."
            },
            "steps": {
                "type": "array",
                "items": { "type": "string" }
            },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 30,
                "maximum": 86400,
                "default": 60
            },
            "target_human_email": {
                "type": "string",
                "description": "Optional visible human profile email/key to ask. Omit to ask the human attached to this agent secret."
            },
            "background": {
                "type": "boolean",
                "default": false,
                "description": "Compatibility mode. Prefer ask_humen_*_async tools for non-blocking background calls."
            }
        }
    })
}

fn ask_humen_async_schema() -> Value {
    let mut schema = ask_humen_schema();
    schema["properties"]["background"] = json!({
        "type": "boolean",
        "default": true,
        "description": "Ignored by this tool; async tools always return immediately."
    });
    schema
}

fn ask_humen_text_async_schema() -> Value {
    json!({
        "type": "object",
        "required": ["title", "prompt"],
        "properties": {
            "title": { "type": "string" },
            "prompt": { "type": "string" },
            "steps": {
                "type": "array",
                "items": { "type": "string" }
            },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 30,
                "maximum": 86400,
                "default": 60
            },
            "target_human_email": {
                "type": "string",
                "description": "Optional visible human profile email/key to ask. Omit to ask the human attached to this agent secret."
            }
        }
    })
}

fn ask_humen_choice_async_schema() -> Value {
    json!({
        "type": "object",
        "required": ["title", "prompt", "choices"],
        "properties": {
            "title": { "type": "string" },
            "prompt": { "type": "string" },
            "choices": {
                "type": "array",
                "minItems": 1,
                "items": { "type": "string" }
            },
            "steps": {
                "type": "array",
                "items": { "type": "string" }
            },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 30,
                "maximum": 86400,
                "default": 60
            },
            "target_human_email": {
                "type": "string",
                "description": "Optional visible human profile email/key to ask. Omit to ask the human attached to this agent secret."
            }
        }
    })
}

fn ask_humen_judgment_async_schema() -> Value {
    json!({
        "type": "object",
        "required": ["title", "prompt"],
        "properties": {
            "title": { "type": "string" },
            "prompt": { "type": "string" },
            "steps": {
                "type": "array",
                "items": { "type": "string" }
            },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 30,
                "maximum": 86400,
                "default": 60
            },
            "target_human_email": {
                "type": "string",
                "description": "Optional visible human profile email/key to ask. Omit to ask the human attached to this agent secret."
            }
        }
    })
}

fn read_humen_replies_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "request_id": {
                "type": "string",
                "description": "Optional UUID returned by ask_humen when background=true."
            },
            "since": {
                "type": "integer",
                "description": "Optional Unix timestamp; only replies answered at or after this time are returned."
            },
            "unread_only": {
                "type": "boolean",
                "default": false
            },
            "mark_read": {
                "type": "boolean",
                "default": false
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": 200,
                "default": 50
            }
        }
    })
}

fn search_humen_network_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "q": {
                "type": "string",
                "description": "Optional text query sent to child nodes."
            },
            "tag": {
                "type": "string",
                "description": "Optional #tag query sent to child nodes."
            },
            "include_local": {
                "type": "boolean",
                "default": false,
                "description": "Also include profiles visible on this local node."
            }
        }
    })
}

fn ask_humen_network_async_schema() -> Value {
    let mut schema = ask_humen_async_schema();
    schema["properties"]["target_node_id"] = json!({
        "type": "string",
        "description": "Optional configured federation node id. If omitted, the server chooses by route_tags or request #tags."
    });
    schema["properties"]["route_tags"] = json!({
        "type": "array",
        "items": { "type": "string" },
        "description": "Optional routing hints matched against configured federation node tags."
    });
    schema["properties"]["hop_limit"] = json!({
        "type": "integer",
        "minimum": 1,
        "maximum": 16,
        "default": 3,
        "description": "Loop guard carried with the federated request."
    });
    schema
}

fn read_humen_network_ledger_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": 200,
                "default": 50
            }
        }
    })
}

fn list_agent_inbox_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "unread_only": {
                "type": "boolean",
                "default": false
            },
            "mark_read": {
                "type": "boolean",
                "default": false
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": AGENT_INBOX_LIMIT_MAX,
                "default": 100
            }
        }
    })
}

fn create_humen_task_schema() -> Value {
    json!({
        "type": "object",
        "required": ["title"],
        "properties": {
            "title": { "type": "string" },
            "description": {
                "type": "string",
                "description": "Task details for the human task view."
            },
            "steps": {
                "type": "array",
                "items": { "type": "string" }
            },
            "tags": {
                "type": "array",
                "items": { "type": "string" },
                "description": "#admin is reserved and will be ignored."
            },
            "due_at": {
                "type": "integer",
                "description": "Optional Unix timestamp deadline."
            }
        }
    })
}

fn list_humen_tasks_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": ["open", "in_progress", "done", "archived"]
            },
            "include_archived": {
                "type": "boolean",
                "default": false
            }
        }
    })
}

fn request_human_friend_schema() -> Value {
    json!({
        "type": "object",
        "required": ["human_email"],
        "properties": {
            "human_email": { "type": "string" },
            "message": {
                "type": "string",
                "maxLength": MEMO_BODY_MAX_CHARS
            }
        }
    })
}

fn leave_humen_memo_schema() -> Value {
    json!({
        "type": "object",
        "required": ["target_human_email", "body"],
        "properties": {
            "target_human_email": {
                "type": "string",
                "description": "Visible human profile email/key."
            },
            "body": {
                "type": "string",
                "maxLength": MEMO_BODY_MAX_CHARS,
                "description": "Offline memo or short-term context."
            }
        }
    })
}

fn resolve_request_target_human(
    state: &AppState,
    agent: &AgentContext,
    target_human_email: Option<&str>,
) -> Result<String, ApiError> {
    let Some(target) = target_human_email.and_then(|value| normalize_optional_value(Some(value)))
    else {
        return Ok(agent.email.clone());
    };
    resolve_visible_human_for_agent(state, agent, &target)
}

fn resolve_visible_human_for_agent(
    state: &AppState,
    agent: &AgentContext,
    human_email: &str,
) -> Result<String, ApiError> {
    let target = normalize_email(human_email);
    if target.is_empty() {
        return Err(ApiError::bad_request("human_email is required"));
    }
    agent_visible_profiles(state, agent, None, None)?
        .into_iter()
        .map(|profile| normalize_email(&profile.email))
        .find(|email| email == &target)
        .ok_or_else(|| ApiError::unauthorized("target human is not visible to this agent"))
}

fn resolve_human_for_agent_friend_accept(
    state: &AppState,
    agent: &AgentContext,
    human_email: &str,
) -> Result<String, ApiError> {
    let target = normalize_email(human_email);
    if target.is_empty() {
        return Err(ApiError::bad_request("human_email is required"));
    }
    if resolve_visible_human_for_agent(state, agent, &target).is_ok() {
        return Ok(target);
    }
    let status = db_agent_relation_status(state, &agent.agent_id, &target)?;
    if matches!(
        status,
        AgentRelationStatus::HumanRequested | AgentRelationStatus::Friends
    ) {
        return Ok(target);
    }
    Err(ApiError::unauthorized(
        "target human is not visible to this agent",
    ))
}

fn rate_human_from_agent(
    state: &AppState,
    agent: &AgentContext,
    payload: RateHumanRequest,
) -> Result<ReputationSummary, ApiError> {
    if !payload.score.is_finite() || !(0.0..=10.0).contains(&payload.score) {
        return Err(ApiError::bad_request("score must be a number from 0 to 10"));
    }
    let target = resolve_visible_human_for_agent(state, agent, &payload.rated_email)?;
    if same_user_identity(state, &agent.email, &target) {
        return Err(ApiError::bad_request("cannot rate your own human profile"));
    }
    db_store_human_rating(
        state,
        &target,
        &format!("agent:{}", agent.agent_id),
        payload.score,
        payload.note.as_deref(),
    )
}

fn rate_humen_schema() -> Value {
    json!({
        "type": "object",
        "required": ["rated_email", "score"],
        "properties": {
            "rated_email": {
                "type": "string",
                "description": "Email or stable human id to rate."
            },
            "score": {
                "type": "number",
                "minimum": 0,
                "maximum": 10
            },
            "note": {
                "type": "string"
            }
        }
    })
}

fn report_humen_schema() -> Value {
    json!({
        "type": "object",
        "required": ["reported_email", "reason"],
        "properties": {
            "reported_email": {
                "type": "string",
                "description": "Email or stable human id to report."
            },
            "reason": {
                "type": "string"
            }
        }
    })
}
