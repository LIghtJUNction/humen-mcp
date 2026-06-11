async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let session = if let Some(token) = query.token.as_deref() {
        if let Some(session) = state.session_from_token(token) {
            ensure_user_allowed(&state, &session.user.email)?;
            session
        } else {
            require_session(&state, &headers)?
        }
    } else {
        require_session(&state, &headers)?
    };
    Ok(ws.on_upgrade(move |socket| websocket(socket, state, session)))
}

async fn websocket(mut socket: WebSocket, state: AppState, session: Session) {
    let active_index = begin_active_period(&state, &session.user.email);
    let online_count = online_user_count(&state);
    broadcast_presence_changed(&state);
    let session_email = normalize_email(&session.user.email);

    let initial: Vec<_> = state
        .requests
        .iter()
        .filter(|entry| can_access_request(&state, &session_email, entry.value()))
        .map(|entry| entry.value().clone())
        .collect();
    let initial_tasks = db_list_agent_tasks(&state, &session_email, None, false, 200)
        .unwrap_or_else(|err| {
            warn!(error = %err.message, "failed to load websocket task snapshot");
            Vec::new()
        });
    if socket
        .send(Message::Text(
            json!({
                "type": "snapshot",
                "requests": initial,
                "tasks": initial_tasks,
                "online_count": online_count
            })
            .to_string()
            .into(),
        ))
        .await
        .is_err()
    {
        end_active_period(&state, &session.user.email, active_index);
        broadcast_presence_changed(&state);
        return;
    }

    let mut rx = state.events.subscribe();
    let mut shutdown = state.shutdown.subscribe();
    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                let _ = socket.send(Message::Close(None)).await;
                break;
            }
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        if !can_receive_event(&state, &session_email, &event) {
                            continue;
                        }
                        let Ok(text) = serde_json::to_string(&event) else {
                            continue;
                        };
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }

    end_active_period(&state, &session.user.email, active_index);
    broadcast_presence_changed(&state);
}

fn broadcast_presence_changed(state: &AppState) {
    let online_count = online_user_count(state);
    let _ = state
        .events
        .send(ServerEvent::PresenceChanged { online_count });
}

fn online_user_count(state: &AppState) -> usize {
    online_presence_sources(state).len()
}

fn can_access_request(state: &AppState, email: &str, request: &HumanRequest) -> bool {
    request
        .assigned_to
        .as_deref()
        .is_none_or(|assigned_to| same_user_identity(state, assigned_to, email))
}

fn can_receive_event(state: &AppState, email: &str, event: &ServerEvent) -> bool {
    match event {
        ServerEvent::RequestCreated { request } => can_access_request(state, email, request),
        ServerEvent::RequestAnswered { request, .. } => can_access_request(state, email, request),
        ServerEvent::RequestExpired {
            expired_request, ..
        } => can_access_request(state, email, &expired_request.request),
        ServerEvent::MemoCreated { memo } => same_user_identity(state, &memo.target_email, email),
        ServerEvent::TaskCreated { task } | ServerEvent::TaskUpdated { task } => {
            same_user_identity(state, &task.assigned_to, email)
        }
        ServerEvent::TrashCleaned { .. } | ServerEvent::PresenceChanged { .. } => true,
    }
}
