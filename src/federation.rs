fn load_federation(path: &str) -> anyhow::Result<FederationRegistry> {
    let Some(path) = normalize_optional_value(Some(path)) else {
        return Ok(FederationRegistry::default());
    };
    let raw = fs::read_to_string(&path).with_context(|| format!("read federation file {path}"))?;
    let mut registry: FederationRegistry = if path.ends_with(".json") {
        serde_json::from_str(&raw).with_context(|| format!("parse federation JSON {path}"))?
    } else {
        toml::from_str(&raw).with_context(|| format!("parse federation TOML {path}"))?
    };
    for node in &mut registry.nodes {
        node.node_id = node.node_id.trim().to_string();
        node.endpoint = node.endpoint.trim().trim_end_matches('/').to_string();
        node.agent_secret = node.agent_secret.trim().to_string();
        node.tags = normalize_tag_list(std::mem::take(&mut node.tags));
        if node.enabled
            && (node.node_id.is_empty() || node.endpoint.is_empty() || node.agent_secret.is_empty())
        {
            anyhow::bail!(
                "enabled federation nodes require node_id, endpoint, and agent_secret"
            );
        }
    }
    Ok(registry)
}

fn normalize_tag_list(values: Vec<String>) -> Vec<String> {
    let mut tags = values
        .into_iter()
        .filter_map(|value| normalize_optional_value(Some(&value)))
        .map(|value| {
            if value.starts_with('#') {
                value
            } else {
                format!("#{value}")
            }
        })
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();
    tags
}

impl FederationRegistry {
    fn enabled_nodes(&self) -> impl Iterator<Item = &FederationNode> {
        self.nodes.iter().filter(|node| node.enabled)
    }

    fn summary(&self) -> Vec<FederationNodeSummary> {
        self.enabled_nodes().map(FederationNode::summary).collect()
    }

    fn find(&self, node_id: &str) -> Option<&FederationNode> {
        let node_id = node_id.trim();
        self.enabled_nodes().find(|node| node.node_id == node_id)
    }

    fn choose(&self, route_tags: &[String], request_tags: &[String]) -> Option<&FederationNode> {
        let wanted = normalize_tag_list(
            route_tags
                .iter()
                .chain(request_tags.iter())
                .cloned()
                .collect(),
        );
        self.enabled_nodes()
            .find(|node| {
                !wanted.is_empty()
                    && node
                        .tags
                        .iter()
                        .any(|tag| wanted.iter().any(|wanted| wanted == tag))
            })
            .or_else(|| self.enabled_nodes().next())
    }
}

impl FederationNode {
    fn summary(&self) -> FederationNodeSummary {
        FederationNodeSummary {
            node_id: self.node_id.clone(),
            endpoint: self.endpoint.clone(),
            enabled: self.enabled,
            description: self.description.clone(),
            tags: self.tags.clone(),
            trust_level: self.trust_level.clone(),
            max_hops: self.max_hops,
        }
    }
}

async fn list_humen_nodes(state: &AppState, id: Option<Value>) -> Result<Json<Value>, ApiError> {
    Ok(Json(mcp_text_result(
        id,
        json!({
            "node_id": state.config.node_id,
            "ledger_head": db_federation_ledger_head(state)?,
            "nodes": state.federation.summary()
        }),
    )))
}

async fn read_humen_network_ledger(
    state: &AppState,
    id: Option<Value>,
    args: ReadNetworkLedgerArgs,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(mcp_text_result(
        id,
        json!({
            "node_id": state.config.node_id,
            "ledger_head": db_federation_ledger_head(state)?,
            "entries": db_list_federation_ledger_entries(state, args.limit.unwrap_or(50))?
        }),
    )))
}

async fn search_humen_network(
    state: &AppState,
    agent: &AgentContext,
    id: Option<Value>,
    args: NetworkSearchArgs,
) -> Result<Json<Value>, ApiError> {
    let local_users = if args.include_local {
        agent_visible_profiles(&state, agent, args.q.as_deref(), args.tag.as_deref())?
    } else {
        Vec::new()
    };
    let mut remote_nodes = Vec::new();
    for node in state.federation.enabled_nodes() {
        let result = match call_remote_mcp_tool(
            &state,
            node,
            "search_humen_profiles",
            json!({
                "q": args.q,
                "tag": args.tag
            }),
        )
        .await
        {
            Ok(value) => json!({ "ok": true, "result": value }),
            Err(err) => json!({ "ok": false, "error": err.message }),
        };
        remote_nodes.push(json!({
            "node": node.summary(),
            "search": result
        }));
    }
    Ok(Json(mcp_text_result(
        id,
        json!({
            "node_id": state.config.node_id,
            "local_users": local_users,
            "remote_nodes": remote_nodes
        }),
    )))
}

async fn ask_humen_network_async(
    state: AppState,
    agent: AgentContext,
    id: Option<Value>,
    mut args: NetworkAskHumanRequest,
) -> Result<Json<Value>, ApiError> {
    let local_node_id = state.config.node_id.trim();
    if args
        .target_node_id
        .as_deref()
        .is_some_and(|node_id| node_id == local_node_id)
    {
        return create_humen_request(state, agent, id, args.request, true).await;
    }
    if args.path.iter().any(|node_id| node_id == local_node_id) {
        return Err(ApiError::bad_request("federation route loop detected"));
    }
    if args.hop_limit == 0 {
        return Err(ApiError::bad_request("federation hop_limit is exhausted"));
    }

    args.request.title = args.request.title.trim().to_string();
    args.request.prompt = args.request.prompt.trim().to_string();
    if args.request.title.is_empty() {
        return Err(ApiError::bad_request("title is required"));
    }
    if args.request.prompt.is_empty() {
        return Err(ApiError::bad_request("prompt is required"));
    }
    args.request.choices = normalize_request_choices(&args.request.kind, args.request.choices)?;
    let mut tag_sources = vec![args.request.title.as_str(), args.request.prompt.as_str()];
    tag_sources.extend(args.request.steps.iter().map(String::as_str));
    let request_tags = extract_tags(&tag_sources);
    let node = if let Some(target_node_id) = args.target_node_id.as_deref() {
        state
            .federation
            .find(target_node_id)
            .ok_or_else(|| ApiError::bad_request("target_node_id is not configured"))?
    } else {
        match state.federation.choose(&args.route_tags, &request_tags) {
            Some(node) => node,
            None if !args.path.is_empty() => {
                return create_humen_request(state, agent, id, args.request, true).await;
            }
            None => {
                return Err(ApiError::bad_request(
                    "no enabled federation nodes are configured",
                ));
            }
        }
    };
    if args.path.iter().any(|node_id| node_id == &node.node_id) {
        return Err(ApiError::bad_request("federation route loop detected"));
    }
    let timeout_seconds = args.request.timeout_seconds.clamp(30, 86400);
    args.request.timeout_seconds = timeout_seconds;
    args.request.background = true;
    let (image_base64, image_mime_type) =
        normalize_image_payload(args.request.image_base64, args.request.image_mime_type);
    args.request.image_base64 = image_base64;
    args.request.image_mime_type = image_mime_type;

    let mut forwarded_args = args.clone();
    forwarded_args.path.push(local_node_id.to_string());
    forwarded_args.hop_limit = forwarded_args.hop_limit.saturating_sub(1);
    let remote_result = if forwarded_args.hop_limit > 0 {
        let remote_arguments = serde_json::to_value(&forwarded_args).map_err(|err| {
            ApiError::internal(format!("serialize federated network request: {err}"))
        })?;
        match call_remote_mcp_tool(&state, node, "ask_humen_network_async", remote_arguments).await
        {
            Ok(value) => value,
            Err(err) if err.message.contains("unknown tool") => {
                let remote_arguments = serde_json::to_value(&args.request).map_err(|err| {
                    ApiError::internal(format!("serialize federated request: {err}"))
                })?;
                call_remote_mcp_tool(&state, node, "ask_humen_async", remote_arguments).await?
            }
            Err(err) => return Err(err),
        }
    } else {
        let remote_arguments = serde_json::to_value(&args.request)
            .map_err(|err| ApiError::internal(format!("serialize federated request: {err}")))?;
        call_remote_mcp_tool(&state, node, "ask_humen_async", remote_arguments).await?
    };
    let remote_request_id = remote_result
        .get("request_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| ApiError::upstream("remote node did not return request_id"))?;

    let now = now_unix();
    let local_request = HumanRequest {
        id: Uuid::new_v4(),
        kind: args.request.kind,
        title: args.request.title,
        prompt: args.request.prompt,
        choices: args.request.choices,
        image_url: args.request.image_url,
        image_base64: args.request.image_base64,
        image_mime_type: args.request.image_mime_type,
        steps: args.request.steps,
        created_at: now,
        timeout_seconds,
        expires_at: now.saturating_add(timeout_seconds),
        tags: request_tags,
        assigned_to: Some(format!("node:{}", node.node_id)),
        created_by: Some(agent.email.clone()),
        created_by_agent_id: normalize_optional_value(Some(agent.agent_id.as_str())),
    };
    db_insert_request(&state, &local_request)?;
    let ledger_path = forwarded_args.path.clone();
    db_insert_federated_request(
        &state,
        &FederatedRequest {
            local_request_id: local_request.id,
            origin_agent_email: agent.email.clone(),
            target_node_id: node.node_id.clone(),
            remote_request_id,
            path: forwarded_args.path,
            status: FederatedRequestStatus::Pending,
            created_at: now,
            expires_at: local_request.expires_at,
            updated_at: now,
        },
    )?;
    db_append_federation_ledger_event(
        &state,
        "federated_request_created",
        &local_request.id.to_string(),
        json!({
            "local_request_id": local_request.id,
            "origin_agent_email": agent.email,
            "target_node_id": node.node_id,
            "remote_request_id": remote_request_id,
            "path": ledger_path,
            "expires_at": local_request.expires_at
        }),
    )?;
    state.requests.insert(local_request.id, local_request.clone());

    Ok(Json(mcp_text_result(
        id,
        json!({
            "status": "pending",
            "request_id": local_request.id,
            "remote_request_id": remote_request_id,
            "target_node_id": node.node_id,
            "title": local_request.title,
            "timeout_seconds": timeout_seconds,
            "expires_at": local_request.expires_at,
            "poll_tool": "read_humen_replies"
        }),
    )))
}

async fn collect_federated_replies_for_agent(
    state: &AppState,
    agent: &AgentContext,
) -> Result<(), ApiError> {
    let requests = db_list_pending_federated_requests_for_agent(state, &agent.email)?;
    for federated in requests {
        if now_unix() >= federated.expires_at {
            if let Some(expired) = expire_request(
                state,
                federated.local_request_id,
                "Federated human request timed out".to_string(),
            ) {
                warn!(
                    request_id = %expired.request.id,
                    target_node_id = %federated.target_node_id,
                    "federated request expired"
                );
            }
            db_update_federated_request_status(
                state,
                federated.local_request_id,
                FederatedRequestStatus::Expired,
            )?;
            db_append_federation_ledger_event(
                state,
                "federated_request_expired",
                &federated.local_request_id.to_string(),
                json!({
                    "local_request_id": federated.local_request_id,
                    "target_node_id": federated.target_node_id,
                    "remote_request_id": federated.remote_request_id,
                    "expires_at": federated.expires_at
                }),
            )?;
            continue;
        }
        let Some(node) = state.federation.find(&federated.target_node_id) else {
            db_update_federated_request_status(
                state,
                federated.local_request_id,
                FederatedRequestStatus::Failed,
            )?;
            db_append_federation_ledger_event(
                state,
                "federated_request_failed",
                &federated.local_request_id.to_string(),
                json!({
                    "local_request_id": federated.local_request_id,
                    "target_node_id": federated.target_node_id,
                    "remote_request_id": federated.remote_request_id,
                    "reason": "target node is no longer configured"
                }),
            )?;
            continue;
        };
        let remote_result = match call_remote_mcp_tool(
            state,
            node,
            "read_humen_replies",
            json!({
                "request_id": federated.remote_request_id,
                "mark_read": true,
                "limit": 1
            }),
        )
        .await
        {
            Ok(value) => value,
            Err(err) => {
                warn!(
                    target_node_id = %federated.target_node_id,
                    remote_request_id = %federated.remote_request_id,
                    error = %err.message,
                    "failed to collect federated reply"
                );
                continue;
            }
        };
        let Some(reply) = remote_result
            .get("replies")
            .and_then(Value::as_array)
            .and_then(|replies| replies.first())
        else {
            continue;
        };
        let mut answer: HumanAnswer =
            serde_json::from_value(reply.get("answer").cloned().unwrap_or(Value::Null))
                .map_err(|err| ApiError::upstream(format!("parse remote answer: {err}")))?;
        answer.answered_by = format!("{}@{}", answer.answered_by, federated.target_node_id);
        let Some((local_request, _status)) = db_get_request(state, federated.local_request_id)?
        else {
            db_update_federated_request_status(
                state,
                federated.local_request_id,
                FederatedRequestStatus::Failed,
            )?;
            continue;
        };
        state.requests.remove(&federated.local_request_id);
        db_store_answer(state, &local_request, &answer, false)?;
        let answered_at = answer.answered_at;
        let answered_by = answer.answered_by.clone();
        let _ = state.events.send(ServerEvent::RequestAnswered {
            id: federated.local_request_id,
            request: local_request,
            answer,
            answered_late: false,
        });
        db_update_federated_request_status(
            state,
            federated.local_request_id,
            FederatedRequestStatus::Answered,
        )?;
        db_append_federation_ledger_event(
            state,
            "federated_reply_collected",
            &federated.local_request_id.to_string(),
            json!({
                "local_request_id": federated.local_request_id,
                "target_node_id": federated.target_node_id,
                "remote_request_id": federated.remote_request_id,
                "answered_at": answered_at,
                "answered_by": answered_by
            }),
        )?;
    }
    Ok(())
}

async fn call_remote_mcp_tool(
    state: &AppState,
    node: &FederationNode,
    tool_name: &str,
    arguments: Value,
) -> Result<Value, ApiError> {
    let response = state
        .http
        .post(&node.endpoint)
        .bearer_auth(&node.agent_secret)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": Uuid::new_v4().to_string(),
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            }
        }))
        .send()
        .await
        .map_err(|err| ApiError::upstream(format!("call federation node {}: {err}", node.node_id)))?;
    if !response.status().is_success() {
        return Err(ApiError::upstream(format!(
            "federation node {} returned HTTP {}",
            node.node_id,
            response.status()
        )));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|err| ApiError::upstream(format!("parse federation node response: {err}")))?;
    parse_mcp_text_result(value)
}

fn parse_mcp_text_result(value: Value) -> Result<Value, ApiError> {
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("remote MCP error");
        return Err(ApiError::upstream(message.to_string()));
    }
    let text = value
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::upstream("remote MCP response did not contain text result"))?;
    serde_json::from_str(text).map_err(|err| ApiError::upstream(format!("parse MCP text: {err}")))
}

fn db_insert_federated_request(
    state: &AppState,
    request: &FederatedRequest,
) -> Result<(), ApiError> {
    let path_json = serde_json::to_string(&request.path)
        .map_err(|err| ApiError::internal(format!("serialize federation path: {err}")))?;
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    db.execute(
        "INSERT OR REPLACE INTO federated_requests \
         (local_request_id, origin_agent_email, target_node_id, remote_request_id, path_json, status, created_at, expires_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            request.local_request_id.to_string(),
            normalize_email(&request.origin_agent_email),
            request.target_node_id,
            request.remote_request_id.to_string(),
            path_json,
            request.status.as_str(),
            request.created_at,
            request.expires_at,
            request.updated_at
        ],
    )
    .map_err(|err| ApiError::internal(format!("persist federated request: {err}")))?;
    Ok(())
}

fn db_list_pending_federated_requests_for_agent(
    state: &AppState,
    agent_email: &str,
) -> Result<Vec<FederatedRequest>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut stmt = db
        .prepare(
            "SELECT local_request_id, origin_agent_email, target_node_id, remote_request_id, path_json, status, created_at, expires_at, updated_at \
             FROM federated_requests \
             WHERE origin_agent_email = ?1 AND status = 'pending' \
             ORDER BY created_at ASC",
        )
        .map_err(|err| ApiError::internal(format!("prepare federated request query: {err}")))?;
    let rows = stmt
        .query_map(params![normalize_email(agent_email)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, u64>(6)?,
                row.get::<_, u64>(7)?,
                row.get::<_, u64>(8)?,
            ))
        })
        .map_err(|err| ApiError::internal(format!("query federated requests: {err}")))?;
    let mut requests = Vec::new();
    for row in rows {
        let (
            local_request_id,
            origin_agent_email,
            target_node_id,
            remote_request_id,
            path_json,
            status,
            created_at,
            expires_at,
            updated_at,
        ) = row.map_err(|err| ApiError::internal(format!("read federated request: {err}")))?;
        requests.push(FederatedRequest {
            local_request_id: Uuid::parse_str(&local_request_id)
                .map_err(|err| ApiError::internal(format!("parse local request id: {err}")))?,
            origin_agent_email,
            target_node_id,
            remote_request_id: Uuid::parse_str(&remote_request_id)
                .map_err(|err| ApiError::internal(format!("parse remote request id: {err}")))?,
            path: serde_json::from_str(&path_json)
                .map_err(|err| ApiError::internal(format!("parse federation path: {err}")))?,
            status: FederatedRequestStatus::from_str(&status),
            created_at,
            expires_at,
            updated_at,
        });
    }
    Ok(requests)
}

fn db_update_federated_request_status(
    state: &AppState,
    local_request_id: Uuid,
    status: FederatedRequestStatus,
) -> Result<(), ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    db.execute(
        "UPDATE federated_requests SET status = ?1, updated_at = ?2 WHERE local_request_id = ?3",
        params![status.as_str(), now_unix(), local_request_id.to_string()],
    )
    .map_err(|err| ApiError::internal(format!("update federated request: {err}")))?;
    Ok(())
}

fn db_append_federation_ledger_event(
    state: &AppState,
    event_type: &str,
    subject_id: &str,
    event: Value,
) -> Result<FederationLedgerEntry, ApiError> {
    let created_at = now_unix();
    let event_json = serde_json::to_string(&event)
        .map_err(|err| ApiError::internal(format!("serialize federation ledger event: {err}")))?;
    let node_id = state.config.node_id.trim().to_string();
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let (previous_sequence, previous_hash) = db
        .query_row(
            "SELECT sequence, event_hash FROM federation_ledger ORDER BY sequence DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|err| ApiError::internal(format!("read federation ledger head: {err}")))?
        .unwrap_or_else(|| (0, "genesis".to_string()));
    let sequence = previous_sequence.saturating_add(1);
    let event_hash = federation_event_hash(
        sequence,
        &node_id,
        event_type,
        subject_id,
        &previous_hash,
        &event_json,
        created_at,
    );
    db.execute(
        "INSERT INTO federation_ledger \
         (node_id, event_type, subject_id, previous_hash, event_hash, event_json, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            node_id,
            event_type,
            subject_id,
            previous_hash,
            event_hash,
            event_json,
            created_at
        ],
    )
    .map_err(|err| ApiError::internal(format!("append federation ledger event: {err}")))?;
    Ok(FederationLedgerEntry {
        sequence,
        node_id: state.config.node_id.trim().to_string(),
        event_type: event_type.to_string(),
        subject_id: subject_id.to_string(),
        previous_hash,
        event_hash,
        event_json: event,
        created_at,
    })
}

fn federation_event_hash(
    sequence: u64,
    node_id: &str,
    event_type: &str,
    subject_id: &str,
    previous_hash: &str,
    event_json: &str,
    created_at: u64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sequence.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(node_id.as_bytes());
    hasher.update(b"|");
    hasher.update(event_type.as_bytes());
    hasher.update(b"|");
    hasher.update(subject_id.as_bytes());
    hasher.update(b"|");
    hasher.update(previous_hash.as_bytes());
    hasher.update(b"|");
    hasher.update(created_at.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(event_json.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn db_federation_ledger_head(state: &AppState) -> Result<Option<FederationLedgerHead>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let head = db
        .query_row(
            "SELECT sequence, event_hash FROM federation_ledger ORDER BY sequence DESC LIMIT 1",
            [],
            |row| {
                Ok(FederationLedgerHead {
                    sequence: row.get(0)?,
                    event_hash: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|err| ApiError::internal(format!("read federation ledger head: {err}")))?;
    Ok(head)
}

fn db_list_federation_ledger_entries(
    state: &AppState,
    limit: u64,
) -> Result<Vec<FederationLedgerEntry>, ApiError> {
    let limit = limit.clamp(1, 200);
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::internal("sqlite lock poisoned"))?;
    let mut stmt = db
        .prepare(
            "SELECT sequence, node_id, event_type, subject_id, previous_hash, event_hash, event_json, created_at \
             FROM federation_ledger \
             ORDER BY sequence DESC \
             LIMIT ?1",
        )
        .map_err(|err| ApiError::internal(format!("prepare federation ledger query: {err}")))?;
    let rows = stmt
        .query_map(params![limit], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, u64>(7)?,
            ))
        })
        .map_err(|err| ApiError::internal(format!("query federation ledger: {err}")))?;
    let mut entries = Vec::new();
    for row in rows {
        let (
            sequence,
            node_id,
            event_type,
            subject_id,
            previous_hash,
            event_hash,
            event_json,
            created_at,
        ) = row.map_err(|err| ApiError::internal(format!("read federation ledger: {err}")))?;
        entries.push(FederationLedgerEntry {
            sequence,
            node_id,
            event_type,
            subject_id,
            previous_hash,
            event_hash,
            event_json: serde_json::from_str(&event_json)
                .map_err(|err| ApiError::internal(format!("parse federation ledger: {err}")))?,
            created_at,
        });
    }
    Ok(entries)
}
