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

    let id = payload.id.clone();
    let result = match payload.method.as_str() {
        "initialize" => json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {
                "tools": {}
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
                    "name": "ask_humen",
                    "description": "Ask a logged-in human to complete a simple task and wait for the answer. For non-blocking background calls, prefer the ask_humen_*_async tools.",
                    "inputSchema": ask_humen_schema()
                },
                {
                    "name": "ask_humen_async",
                    "description": "Create a human request and return immediately with request_id. Poll read_humen_replies for the answer.",
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
                    "description": "Read completed human replies for the user attached to this agent secret, optionally filtered by request_id.",
                    "inputSchema": read_humen_replies_schema()
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
                    "description": "Rate a human from 0 to 10. Reputation is the average of ratings; unrated humans start at 5.",
                    "inputSchema": rate_humen_schema()
                },
                {
                    "name": "report_humen",
                    "description": "Report a human to the administrator mailbox and reduce their reputation through a zero rating from this agent's owner.",
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
            Ok(Json(mcp_text_result(
                id,
                json!({ "replies": db_read_humen_replies(&state, &agent.email, args)? }),
            )))
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
            let reputation = rate_human_from_actor(&state, &agent.email, args)?;
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
        assigned_to: Some(agent.email.clone()),
    };
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
