#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> AppState {
        test_state_with_public_base_url("http://127.0.0.1:8787")
    }

    fn test_state_with_public_base_url(public_base_url: &str) -> AppState {
        AppState::new(Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            public_base_url: public_base_url.to_string(),
            web_dist: "./humen-mcp-webui/dist".to_string(),
            users_file: std::env::temp_dir()
                .join(format!("humen-mcp-test-{}.json", Uuid::new_v4())),
            db_file: std::env::temp_dir()
                .join(format!("humen-mcp-test-{}.sqlite3", Uuid::new_v4())),
            admin_email: "admin-local".to_string(),
            admin_password: "secret".to_string(),
            session_secret: "test-session-secret".to_string(),
            github_client_id: None,
            github_client_secret: None,
            github_api_token: None,
            trash_retention_seconds: 60,
            cleanup_interval_seconds: 60,
            self_update_command: String::new(),
            self_update_timeout_seconds: 120,
            plugin_dir: String::new(),
            node_id: "test-root".to_string(),
            federation_file: String::new(),
        })
        .unwrap()
    }

    #[test]
    fn ask_humen_schema_exposes_simple_task_kinds() {
        let schema = ask_humen_schema();
        let kinds = schema["properties"]["kind"]["enum"].as_array().unwrap();
        assert!(kinds.contains(&json!("choice")));
        assert!(kinds.contains(&json!("judgment")));
        assert!(kinds.contains(&json!("text")));
        assert!(kinds.contains(&json!("image_review")));
        assert!(kinds.contains(&json!("steps")));
        assert_eq!(
            schema["properties"]["timeout_seconds"]["default"],
            json!(60)
        );
        assert_eq!(
            schema["properties"]["image_mime_type"]["default"],
            json!("image/png")
        );
        assert!(schema["properties"].get("image_base64").is_some());
        assert!(schema["properties"].get("target_human_email").is_some());
        assert!(
            ask_humen_text_async_schema()["properties"]
                .get("target_human_email")
                .is_some()
        );
        assert_eq!(
            ask_humen_judgment_async_schema()["required"],
            json!(["title", "prompt"])
        );
        assert_eq!(
            ask_humen_choice_async_schema()["required"],
            json!(["title", "prompt", "choices"])
        );
        assert_eq!(default_timeout(), 60);
    }

    #[test]
    fn shortcut_schemas_expose_blocking_human_request_inputs() {
        for schema in [approve_schema(), judge_schema(), feedback_schema()] {
            assert_eq!(schema["required"], json!(["title", "prompt"]));
            assert_eq!(
                schema["properties"]["timeout_seconds"]["default"],
                json!(60)
            );
            assert!(schema["properties"].get("background").is_none());
        }
    }

    #[test]
    fn shortcut_tools_force_their_fixed_request_kinds_and_blocking_mode() {
        let payload = McpRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(1)),
            method: "tools/call".to_string(),
            params: json!({
                "arguments": {
                    "kind": "text",
                    "title": "Check rollout",
                    "prompt": "Can this deploy continue?",
                    "background": true
                }
            }),
        };

        let approval =
            parse_blocking_shortcut_arguments(&payload, "approve", TaskKind::Judgment).unwrap();
        assert_eq!(approval.kind, TaskKind::Judgment);
        assert!(!approval.background);

        let feedback = parse_blocking_shortcut_arguments(&payload, "feedback", TaskKind::Text)
            .unwrap();
        assert_eq!(feedback.kind, TaskKind::Text);
        assert!(!feedback.background);
    }

    #[test]
    fn report_and_rating_schemas_expose_public_interfaces() {
        let rating_schema = rate_humen_schema();
        assert_eq!(rating_schema["required"], json!(["rated_email", "score"]));
        assert_eq!(rating_schema["properties"]["score"]["minimum"], json!(0));
        assert_eq!(rating_schema["properties"]["score"]["maximum"], json!(10));

        let report_schema = report_humen_schema();
        assert_eq!(report_schema["required"], json!(["reported_email", "reason"]));
        assert!(report_schema["properties"].get("reported_email").is_some());
        assert!(report_schema["properties"].get("reason").is_some());
    }

    #[tokio::test]
    async fn mcp_tools_list_exposes_reputation_and_shortcut_tools() {
        let state = test_state();
        {
            let mut settings = state.admin_settings.lock().unwrap();
            settings.agent_secret_prefix = Some("prefix-".to_string());
        }
        {
            let mut users = state.users.lock().unwrap();
            users.admin_settings.agent_secret_prefix = Some("prefix-".to_string());
            let mut alice = new_user_record("alice@example.com", 1, "Alice");
            alice.agent_secret = Some("alice-secret-123".to_string());
            users.insert(alice);
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-humen-agent-secret",
            "prefix-alice-secret-123".parse().unwrap(),
        );
        let response = mcp(
            State(state),
            headers,
            Json(McpRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!(1)),
                method: "tools/list".to_string(),
                params: Value::Null,
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        let tool_names: Vec<_> = payload["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();

        assert!(tool_names.contains(&"rate_humen"));
        assert!(tool_names.contains(&"report_humen"));
        assert!(tool_names.contains(&"approve"));
        assert!(tool_names.contains(&"judge"));
        assert!(tool_names.contains(&"feedback"));
        assert!(tool_names.contains(&"list_humen_nodes"));
        assert!(tool_names.contains(&"search_humen_network"));
        assert!(tool_names.contains(&"ask_humen_network_async"));
        assert!(tool_names.contains(&"read_humen_network_ledger"));
        assert!(tool_names.contains(&"list_humen_plugins"));
        assert!(tool_names.contains(&"create_humen_request_from_template"));
        assert!(tool_names.contains(&"leave_humen_memo"));
    }

    #[tokio::test]
    async fn mcp_initialize_advertises_reply_notifications() {
        let state = test_state();
        {
            let mut settings = state.admin_settings.lock().unwrap();
            settings.agent_secret_prefix = Some("prefix-".to_string());
        }
        {
            let mut users = state.users.lock().unwrap();
            users.admin_settings.agent_secret_prefix = Some("prefix-".to_string());
            let mut alice = new_user_record("alice@example.com", 1, "Alice");
            alice.agent_secret = Some("alice-secret-123".to_string());
            users.insert(alice);
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-humen-agent-secret",
            "prefix-alice-secret-123".parse().unwrap(),
        );
        let response = mcp(
            State(state),
            headers,
            Json(McpRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!(1)),
                method: "initialize".to_string(),
                params: json!({
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "test-agent",
                        "version": "0.1.0"
                    }
                }),
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            payload["result"]["capabilities"]["experimental"]["humenNotifications"]
                ["replyAvailableMethod"],
            json!(HUMEN_REPLY_AVAILABLE_NOTIFICATION)
        );
        assert_eq!(
            payload["result"]["capabilities"]["experimental"]["humenNotifications"]
                ["agentInboxChangedMethod"],
            json!(HUMEN_AGENT_INBOX_CHANGED_NOTIFICATION)
        );
        assert_eq!(
            payload["result"]["capabilities"]["experimental"]["humenNotifications"]["fallbackTool"],
            json!("read_humen_replies")
        );
    }

    #[tokio::test]
    async fn web_answers_record_web_channel_source() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            users.insert(new_user_record("bob@example.com", 1, "Bob"));
        }
        let auth = state.create_session("bob@example.com", AuthProvider::Password);
        let mut request = test_human_request();
        request.assigned_to = Some("bob@example.com".to_string());
        state.requests.insert(request.id, request.clone());

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", auth.token).parse().unwrap(),
        );
        let response = answer_request(
            State(state.clone()),
            Path(request.id),
            headers,
            Json(AnswerRequest {
                answer: "web ok".to_string(),
                note: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(response["answer"]["answered_by"], "web:bob@example.com");

        let replies = db_list_answered_requests(&state, 10).unwrap();
        assert_eq!(replies[0].answer.answered_by, "web:bob@example.com");
    }

    #[tokio::test]
    async fn mcp_get_without_sse_accept_keeps_method_warning() {
        let response = mcp_get(State(test_state()), HeaderMap::new()).await.unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn legacy_web_panel_route_redirects_to_root() {
        let response = web_panel_legacy_redirect().await.into_response();
        assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/");
    }

    #[test]
    fn reply_available_notification_points_to_read_replies() {
        let request = test_human_request();
        let payload = mcp_reply_available_notification(request.id, &request, false);

        assert_eq!(
            payload["method"],
            json!(HUMEN_REPLY_AVAILABLE_NOTIFICATION)
        );
        assert_eq!(payload["params"]["request_id"], json!(request.id));
        assert_eq!(payload["params"]["reply_tool"], json!("read_humen_replies"));
        assert!(payload["params"].get("answer").is_none());
    }

    #[test]
    fn reply_available_notification_reaches_agent_created_federated_request() {
        let state = test_state();
        let mut request = test_human_request();
        request.assigned_to = Some("node:branch-cn".to_string());
        request.created_by = Some("alice@example.com".to_string());

        let event = ServerEvent::RequestAnswered {
            id: request.id,
            request,
            answer: HumanAnswer {
                answer: "done".to_string(),
                note: None,
                answered_by: "remote@branch-cn".to_string(),
                answered_at: now_unix(),
            },
            answered_late: false,
        };

        assert!(mcp_sse_event(&state, "alice@example.com", "agent-1", &event).is_some());
    }

    #[test]
    fn agent_inbox_notification_reaches_target_agent_stream() {
        let state = test_state();
        let message = AgentHumanMessage {
            id: Uuid::new_v4(),
            agent_id: "agent-1".to_string(),
            human_email: "bob@example.com".to_string(),
            direction: "human_to_agent".to_string(),
            kind: "friend_request".to_string(),
            body: "please connect".to_string(),
            status: "pending".to_string(),
            created_at: now_unix(),
            resolved_at: None,
            read_at: None,
        };
        let event = ServerEvent::AgentInboxChanged {
            message: message.clone(),
        };

        assert!(mcp_sse_event(&state, "alice@example.com", "agent-1", &event).is_some());
        assert!(mcp_sse_event(&state, "alice@example.com", "other-agent", &event).is_none());
    }

    #[test]
    fn federation_config_normalizes_nodes_without_exposing_secrets() {
        let federation_file =
            std::env::temp_dir().join(format!("humen-mcp-federation-{}.toml", Uuid::new_v4()));
        fs::write(
            &federation_file,
            r##"
[[nodes]]
node_id = "branch-cn"
endpoint = "https://branch.example/mcp/"
agent_secret = "secret-token"
description = "CN branch"
tags = ["ops", "#cn"]
"##,
        )
        .unwrap();

        let registry = load_federation(federation_file.to_str().unwrap()).unwrap();
        let summary = registry.summary();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].node_id, "branch-cn");
        assert_eq!(summary[0].endpoint, "https://branch.example/mcp");
        assert_eq!(summary[0].tags, vec!["#cn", "#ops"]);
        let visible = serde_json::to_value(&summary[0]).unwrap();
        assert!(visible.get("agent_secret").is_none());
    }

    #[test]
    fn federation_ledger_entries_are_hash_chained() {
        let state = test_state();
        let first = db_append_federation_ledger_event(
            &state,
            "federated_request_created",
            "request-1",
            json!({ "target_node_id": "branch-cn" }),
        )
        .unwrap();
        let second = db_append_federation_ledger_event(
            &state,
            "federated_reply_collected",
            "request-1",
            json!({ "answered_by": "human@branch-cn" }),
        )
        .unwrap();

        assert_eq!(first.sequence, 1);
        assert_eq!(first.previous_hash, "genesis");
        assert_eq!(second.sequence, 2);
        assert_eq!(second.previous_hash, first.event_hash);
        assert_ne!(second.event_hash, first.event_hash);

        let head = db_federation_ledger_head(&state).unwrap().unwrap();
        assert_eq!(head.sequence, 2);
        assert_eq!(head.event_hash, second.event_hash);

        let entries = db_list_federation_ledger_entries(&state, 10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence, 2);
        assert_eq!(entries[1].sequence, 1);
    }

    #[test]
    fn human_memo_unread_summary_excludes_self_and_clears_on_read() {
        let state = test_state();
        db_create_human_memo(&state, "alice@example.com", "bob@example.com", "one").unwrap();
        db_create_human_memo(&state, "alice@example.com", "bob@example.com", "two").unwrap();
        db_create_human_memo(&state, "alice@example.com", "alice@example.com", "self").unwrap();
        db_create_human_memo_with_agent(
            &state,
            "alice@example.com",
            "carol@example.com",
            Some("agent-1"),
            Some("Review Agent"),
            "agent memo",
        )
        .unwrap();

        let summary =
            db_unread_human_memo_summary(&state, "alice@example.com", "alice@example.com").unwrap();

        assert_eq!(summary.total, 3);
        assert_eq!(summary.sources.len(), 2);
        let carol = summary
            .sources
            .iter()
            .find(|source| source.author_email == "carol@example.com")
            .unwrap();
        let bob = summary
            .sources
            .iter()
            .find(|source| source.author_email == "bob@example.com")
            .unwrap();
        assert_eq!(carol.author_agent_id.as_deref(), Some("agent-1"));
        assert_eq!(carol.count, 1);
        assert_eq!(bob.count, 2);

        let marked =
            db_mark_human_memos_read(&state, "alice@example.com", "alice@example.com").unwrap();
        assert_eq!(marked, 3);

        let summary =
            db_unread_human_memo_summary(&state, "alice@example.com", "alice@example.com").unwrap();
        assert_eq!(summary.total, 0);
        assert!(summary.sources.is_empty());
    }

    #[tokio::test]
    async fn network_ask_requires_an_enabled_federation_node() {
        let state = test_state();
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
            agent_id: "agent-1".to_string(),
            agent_name: "Agent".to_string(),
            directory_visibility: AgentDirectoryVisibility::SelfOnly,
            directory_min_reputation: default_agent_directory_min_reputation(),
        };

        let err = ask_humen_network_async(
            state,
            agent,
            Some(json!(1)),
            NetworkAskHumanRequest {
                request: CreateHumanRequest {
                    kind: TaskKind::Text,
                    title: "Remote help".to_string(),
                    prompt: "Please check this.".to_string(),
                    choices: Vec::new(),
                    image_url: None,
                    image_base64: None,
                    image_mime_type: None,
                    steps: Vec::new(),
                    timeout_seconds: 60,
                    background: true,
                    target_human_email: None,
                },
                target_node_id: None,
                route_tags: Vec::new(),
                hop_limit: 3,
                path: Vec::new(),
            },
        )
        .await
        .unwrap_err();

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("no enabled federation nodes"));
    }

    #[tokio::test]
    async fn forwarded_network_ask_falls_back_to_leaf_local_request() {
        let state = test_state();
        let agent = AgentContext {
            email: "branch-owner@example.com".to_string(),
            agent_id: "parent-agent".to_string(),
            agent_name: "Parent".to_string(),
            directory_visibility: AgentDirectoryVisibility::SelfOnly,
            directory_min_reputation: default_agent_directory_min_reputation(),
        };

        let response = ask_humen_network_async(
            state.clone(),
            agent,
            Some(json!(1)),
            NetworkAskHumanRequest {
                request: CreateHumanRequest {
                    kind: TaskKind::Text,
                    title: "Leaf help".to_string(),
                    prompt: "Please answer locally.".to_string(),
                    choices: Vec::new(),
                    image_url: None,
                    image_base64: None,
                    image_mime_type: None,
                    steps: Vec::new(),
                    timeout_seconds: 60,
                    background: true,
                    target_human_email: None,
                },
                target_node_id: None,
                route_tags: Vec::new(),
                hop_limit: 2,
                path: vec!["root".to_string()],
            },
        )
        .await
        .unwrap();

        let text = response.0["result"]["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        let request_id = Uuid::parse_str(payload["request_id"].as_str().unwrap()).unwrap();
        let (request, status) = db_get_request(&state, request_id).unwrap().unwrap();
        assert_eq!(status, "pending");
        assert_eq!(request.created_by.as_deref(), Some("branch-owner@example.com"));
        assert_eq!(
            request.assigned_to.as_deref(),
            Some("branch-owner@example.com")
        );
    }

    #[test]
    fn plugin_registry_loads_templates_and_renders_request_arguments() {
        let registry = PluginRegistry {
            plugins: vec![LoadedPlugin {
                source: "memory".to_string(),
                manifest: HumenPluginManifest {
                    id: "release".to_string(),
                    name: "Release".to_string(),
                    request_templates: vec![RequestTemplate {
                        id: "ship-check".to_string(),
                        title: "Ship {{version}}".to_string(),
                        description: "Release gate".to_string(),
                        kind: HumenTaskKind::Judgment,
                        prompt_template: "Can {{version}} ship for {{project}}?".to_string(),
                        steps: vec!["Check {{project}} tests".to_string()],
                        timeout_seconds: Some(120),
                        ..Default::default()
                    }],
                    route_strategies: vec![RouteStrategy {
                        id: "online".to_string(),
                        title: "Online".to_string(),
                        ..Default::default()
                    }],
                    scoring_rules: vec![ScoringRule {
                        id: "risk".to_string(),
                        title: "Risk".to_string(),
                        ..Default::default()
                    }],
                    channels: vec![ThirdPartyChannel {
                        id: "webhook".to_string(),
                        title: "Webhook".to_string(),
                        kind: "webhook".to_string(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            }],
        };

        let create = create_request_from_template_args(
            &registry,
            TemplateRequestArgs {
                template: "release/ship-check".to_string(),
                variables: HashMap::from([
                    ("version".to_string(), json!("v1.2.3")),
                    ("project".to_string(), json!("humen-mcp")),
                ]),
                title: None,
                prompt: None,
                choices: Vec::new(),
                steps: Vec::new(),
                timeout_seconds: None,
                background: true,
            },
        )
        .unwrap();

        assert_eq!(create.kind, TaskKind::Judgment);
        assert_eq!(create.title, "Ship v1.2.3");
        assert_eq!(create.prompt, "Can v1.2.3 ship for humen-mcp?");
        assert_eq!(create.steps, vec!["Check humen-mcp tests"]);
        assert_eq!(create.timeout_seconds, 120);
        assert!(create.background);
        assert!(registry.plugin_summary()["counts"]["channels"].as_u64().unwrap() == 1);
    }

    #[test]
    fn passkey_start_response_uses_public_key_wrapped_options() {
        let state = test_state_with_public_base_url("http://localhost:8787");
        let (email, user_id, display_name, exclude_credentials) =
            ensure_passkey_registration_user(&state, "alice@example.com").unwrap();
        let webauthn = require_webauthn(&state).unwrap();
        let (options, _) = webauthn
            .start_passkey_registration(
                user_id,
                &email,
                &display_name,
                (!exclude_credentials.is_empty()).then_some(exclude_credentials),
            )
            .unwrap();
        let raw = serde_json::to_value(PasskeyRegistrationStartResponse {
            registration_id: Uuid::new_v4(),
            options,
        })
        .unwrap();

        assert!(raw["options"]["publicKey"]["challenge"].as_str().is_some());
        assert!(raw["options"]["challenge"].as_str().is_none());
        assert!(raw["options"]["publicKey"]["user"]["id"].as_str().is_some());
    }

    #[test]
    fn judgment_requests_use_fixed_yes_no_choices() {
        assert_eq!(
            normalize_request_choices(
                &TaskKind::Judgment,
                vec!["maybe".to_string(), "ignored".to_string()]
            )
            .unwrap(),
            vec!["yes".to_string(), "no".to_string()]
        );
        assert!(normalize_request_choices(&TaskKind::Choice, Vec::new()).is_err());
    }

    #[test]
    fn normalize_image_payload_accepts_raw_base64_and_data_urls() {
        let (data, mime) = normalize_image_payload(
            Some(" iVBOR\nw0KGgo= ".to_string()),
            Some("image/jpeg".to_string()),
        );
        assert_eq!(data.as_deref(), Some("iVBORw0KGgo="));
        assert_eq!(mime.as_deref(), Some("image/jpeg"));

        let (data, mime) = normalize_image_payload(
            Some("data:image/webp;base64, AAAA ".to_string()),
            Some("image/png".to_string()),
        );
        assert_eq!(data.as_deref(), Some("AAAA"));
        assert_eq!(mime.as_deref(), Some("image/webp"));
    }

    #[test]
    fn expiring_request_moves_it_to_trash_and_emits_event() {
        let state = test_state();
        let request = HumanRequest {
            id: Uuid::new_v4(),
            kind: TaskKind::Text,
            title: "Check status".to_string(),
            prompt: "Say ok".to_string(),
            choices: Vec::new(),
            image_url: None,
            image_base64: None,
            image_mime_type: None,
            steps: Vec::new(),
            created_at: 100,
            timeout_seconds: 60,
            expires_at: 160,
            tags: Vec::new(),
            assigned_to: None,
            created_by: None,
            created_by_agent_id: None,
        };
        let (tx, _rx) = oneshot::channel();
        let mut events = state.events.subscribe();
        state.requests.insert(request.id, request.clone());
        state.waiters.insert(request.id, tx);

        let expired = expire_request(&state, request.id, "timeout".to_string()).unwrap();
        let event = events.try_recv().unwrap();

        assert!(state.requests.get(&request.id).is_none());
        assert!(state.waiters.get(&request.id).is_none());
        assert!(state.trash.get(&request.id).is_some());
        assert_eq!(expired.request.id, request.id);
        match event {
            ServerEvent::RequestExpired {
                id,
                expired_request,
            } => {
                assert_eq!(id, request.id);
                assert_eq!(expired_request.request.id, request.id);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn active_periods_persist_user_id_and_duration() {
        let state = test_state();
        let index = begin_active_period(&state, "user-one");
        end_active_period(&state, "user-one", index);

        let users = UserStore::load(&state.config.users_file).unwrap();
        let record = users.users.get("user-one").unwrap();
        let period = record.active_periods.first().unwrap();

        assert_eq!(period.user_id, "user-one");
        assert!(period.disconnected_at.is_some());
        assert!(period.duration_seconds.is_some());
    }

    #[test]
    fn bearer_session_round_trips() {
        let state = test_state();
        let auth = state.create_session("admin-local", AuthProvider::Password);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", auth.token).parse().unwrap(),
        );
        let session = state.session_from_headers(&headers).unwrap();

        assert_eq!(session.user.email, "admin-local");
        assert!(state.session_from_token("not-a-token").is_none());
    }

    #[test]
    fn cookie_session_restores_from_sqlite_and_can_be_deleted() {
        let state = test_state();
        let auth = state.create_session("admin-cookie", AuthProvider::Password);
        state.sessions.clear();

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("theme=light; humen-mcp-token={}; other=1", auth.token)
                .parse()
                .unwrap(),
        );
        let session = state.session_from_headers(&headers).unwrap();
        assert_eq!(session.user.email, "admin-cookie");

        state.sessions.clear();
        state.destroy_session_token(&auth.token);
        assert!(state.session_from_headers(&headers).is_none());
    }

    #[test]
    fn online_count_tracks_unique_users_not_connections() {
        let state = test_state();
        let first_tab = begin_active_period(&state, "user-one");
        let second_tab = begin_active_period(&state, "user-one");
        let other_user = begin_active_period(&state, "user-two");

        assert_eq!(online_user_count(&state), 2);

        end_active_period(&state, "user-one", first_tab);
        assert_eq!(online_user_count(&state), 2);

        end_active_period(&state, "user-one", second_tab);
        assert_eq!(online_user_count(&state), 1);

        end_active_period(&state, "user-two", other_user);
        assert_eq!(online_user_count(&state), 0);
    }

    #[test]
    fn online_count_dedupes_migrated_github_identity() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            let mut email_record = new_user_record("alice@example.com", now_unix(), "");
            email_record.login = Some("alice".to_string());
            email_record.active_periods.push(ActivePeriod {
                user_id: "alice@example.com".to_string(),
                connected_at: now_unix(),
                disconnected_at: None,
                duration_seconds: None,
            });
            users.users.insert("alice@example.com".to_string(), email_record);

            let mut github_record = new_user_record("alice@example.com", now_unix(), "");
            github_record.github_id = Some("42".to_string());
            github_record.login = Some("alice".to_string());
            github_record.active_periods.push(ActivePeriod {
                user_id: "github:42".to_string(),
                connected_at: now_unix(),
                disconnected_at: None,
                duration_seconds: None,
            });
            users.users.insert("github:42".to_string(), github_record);
        }

        assert_eq!(online_user_count(&state), 1);
    }

    #[test]
    fn stale_active_periods_are_closed_on_startup() {
        let mut users = UserStore::default();
        let mut record = new_user_record("stale@example.com", 1, "");
        record.active_periods.push(ActivePeriod {
            user_id: "stale@example.com".to_string(),
            connected_at: 1,
            disconnected_at: None,
            duration_seconds: None,
        });
        users.users.insert("stale@example.com".to_string(), record);

        clear_stale_active_periods(&mut users);

        let record = users.users.get("stale@example.com").unwrap();
        assert!(record.active_periods[0].disconnected_at.is_some());
        assert!(record.active_periods[0].duration_seconds.is_some());
    }

    #[test]
    fn reserved_admin_tag_is_not_accepted_from_user_or_agent_input() {
        assert_eq!(
            normalize_tags(vec![
                "#Ops".to_string(),
                "#admin".to_string(),
                "admin".to_string()
            ]),
            vec!["#ops".to_string()]
        );
        assert_eq!(
            extract_tags(&["route #admin request to #ops"]),
            vec!["#ops".to_string()]
        );
    }

    #[test]
    fn admin_tag_is_derived_from_admin_identity_only() {
        let state = test_state();
        let now = now_unix();
        {
            let mut users = state.users.lock().unwrap();
            let mut admin = new_user_record("admin-local", now, "Admin profile");
            admin.tags = vec!["#ops".to_string()];
            users.insert(admin);
            let mut impostor =
                new_user_record("impostor@example.com", now, "Trying to look privileged");
            impostor.tags = vec!["#admin".to_string(), "#ops".to_string()];
            users.insert(impostor);
        }

        let profiles = user_profiles(&state, None, None).unwrap();
        let admin = profiles
            .iter()
            .find(|profile| normalize_email(&profile.email) == "admin-local")
            .unwrap();
        let impostor = profiles
            .iter()
            .find(|profile| normalize_email(&profile.email) == "impostor@example.com")
            .unwrap();
        assert!(admin.tags.iter().any(|tag| tag == ADMIN_TAG));
        assert!(!impostor.tags.iter().any(|tag| tag == ADMIN_TAG));

        let admin_matches = user_profiles(&state, None, Some(ADMIN_TAG)).unwrap();
        assert_eq!(admin_matches.len(), 1);
        assert_eq!(normalize_email(&admin_matches[0].email), "admin-local");

        let counts = tag_counts(&state).unwrap();
        assert_eq!(tag_count(&counts, ADMIN_TAG), Some(1));
        assert_eq!(tag_count(&counts, "#ops"), Some(2));
    }

    #[test]
    fn password_admin_control_account_is_not_public_talent() {
        let state = test_state();
        let now = now_unix();
        {
            let mut users = state.users.lock().unwrap();
            let mut admin = new_user_record("admin-local", now, "Admin control");
            admin.visibility = ProfileVisibility::Public;
            admin.is_public = true;
            users.insert(admin);
            let mut alice = new_user_record("alice@example.com", now, "Alice");
            alice.visibility = ProfileVisibility::Public;
            alice.is_public = true;
            users.insert(alice);
        }

        let admin_profiles = user_profiles(&state, None, None).unwrap();
        let admin = admin_profiles
            .iter()
            .find(|profile| normalize_email(&profile.email) == "admin-local")
            .unwrap();
        assert!(!admin.is_public);
        assert_eq!(admin.visibility, ProfileVisibility::Private);

        let visible = visible_user_profiles_for_session(&state, "alice@example.com", None, None)
            .unwrap()
            .into_iter()
            .map(|profile| normalize_email(&profile.email))
            .collect::<Vec<_>>();
        assert!(visible.contains(&"alice@example.com".to_string()));
        assert!(!visible.contains(&"admin-local".to_string()));
    }

    #[test]
    fn synthetic_admin_is_searchable_by_reserved_tag() {
        let state = test_state();

        let matches = user_profiles(&state, None, Some(ADMIN_TAG)).unwrap();
        let counts = tag_counts(&state).unwrap();

        assert_eq!(matches.len(), 1);
        assert_eq!(normalize_email(&matches[0].email), "admin-local");
        assert_eq!(tag_count(&counts, ADMIN_TAG), Some(1));
    }

    #[test]
    fn admin_record_stays_single_password_identity_after_github_login() {
        let state = test_state();
        let now = now_unix();
        {
            let mut users = state.users.lock().unwrap();
            let mut admin = new_user_record("admin-local", now, "Admin profile");
            admin.ban_expires_at = Some(now + 3600);
            users.insert(admin);
        }

        upsert_github_user(&state, "123", Some("admin-local"), "admin-local").unwrap();
        let profiles = user_profiles(&state, None, None).unwrap();

        assert_eq!(profiles.len(), 1);
        assert_eq!(normalize_email(&profiles[0].email), "admin-local");
        assert!(matches!(profiles[0].provider, AuthProvider::Password));
        assert!(profiles[0].tags.iter().any(|tag| tag == "#admin"));
        assert!(profiles[0].ban_expires_at.is_none());
    }

    #[test]
    fn agent_secret_is_required_and_maps_to_user_suffix() {
        let state = test_state();
        let now = now_unix();
        {
            let mut settings = state.admin_settings.lock().unwrap();
            settings.agent_secret_prefix = Some("prefix-".to_string());
            settings.allow_agent_directory = true;
            settings.agent_directory_visibility = AgentDirectoryVisibility::PublicUsers;
        }
        {
            let mut users = state.users.lock().unwrap();
            users.admin_settings.agent_secret_prefix = Some("prefix-".to_string());
            let mut alice = new_user_record("alice@example.com", now, "Alice");
            alice.agent_secret = Some("alice-secret-123".to_string());
            users.insert(alice);
        }

        let headers = HeaderMap::new();
        assert!(require_agent_access(&state, &headers).is_err());

        let mut headers = HeaderMap::new();
        headers.insert("x-humen-agent-secret", "wrong".parse().unwrap());
        assert!(require_agent_access(&state, &headers).is_err());

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-humen-agent-secret",
            "prefix-alice-secret-123".parse().unwrap(),
        );
        let agent = require_agent_access(&state, &headers).unwrap();

        assert_eq!(agent.email, "alice@example.com");
        assert_eq!(
            agent.directory_visibility,
            AgentDirectoryVisibility::PublicUsers
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer prefix-alice-secret-123".parse().unwrap(),
        );
        let agent = require_agent_access(&state, &headers).unwrap();
        assert_eq!(agent.email, "alice@example.com");
    }

    #[test]
    fn legacy_agent_directory_toggle_deserializes_to_public_policy() {
        let enabled: AdminSettings = serde_json::from_value(json!({
            "allow_registration": true,
            "oauth_channels": [],
            "agent_secret_prefix": "prefix-",
            "allow_agent_directory": true
        }))
        .unwrap();
        assert_eq!(
            enabled.agent_directory_visibility,
            AgentDirectoryVisibility::PublicUsers
        );
        assert!(enabled.allow_agent_directory);

        let disabled: AdminSettings = serde_json::from_value(json!({
            "allow_registration": true,
            "oauth_channels": [],
            "agent_secret_prefix": "prefix-",
            "allow_agent_directory": false
        }))
        .unwrap();
        assert_eq!(
            disabled.agent_directory_visibility,
            AgentDirectoryVisibility::SelfOnly
        );
        assert!(!disabled.allow_agent_directory);

        let stale_enabled: AdminSettings = serde_json::from_value(json!({
            "allow_registration": true,
            "oauth_channels": [],
            "agent_secret_prefix": "prefix-",
            "allow_agent_directory": true,
            "agent_directory_visibility": "self_only"
        }))
        .unwrap();
        assert_eq!(
            stale_enabled.agent_directory_visibility,
            AgentDirectoryVisibility::PublicUsers
        );
        assert!(stale_enabled.allow_agent_directory);

        let stale_disabled: AdminSettings = serde_json::from_value(json!({
            "allow_registration": true,
            "oauth_channels": [],
            "agent_secret_prefix": "prefix-",
            "allow_agent_directory": false,
            "agent_directory_visibility": "public_users"
        }))
        .unwrap();
        assert_eq!(
            stale_disabled.agent_directory_visibility,
            AgentDirectoryVisibility::SelfOnly
        );
        assert!(!stale_disabled.allow_agent_directory);

        let visibility_only: AdminSettings = serde_json::from_value(json!({
            "allow_registration": true,
            "oauth_channels": [],
            "agent_secret_prefix": "prefix-",
            "agent_directory_visibility": "reputation_at_least"
        }))
        .unwrap();
        assert_eq!(
            visibility_only.agent_directory_visibility,
            AgentDirectoryVisibility::ReputationAtLeast
        );
        assert!(visibility_only.allow_agent_directory);
    }

    #[test]
    fn visible_directory_is_self_friends_and_public_users() {
        let state = test_state();
        let now = now_unix();
        {
            let mut users = state.users.lock().unwrap();
            let mut alice = new_user_record("alice@example.com", now, "Alice #ops");
            alice.is_public = false;
            alice.friends = vec!["bob@example.com".to_string()];
            let mut bob = new_user_record("bob@example.com", now, "Bob #review");
            bob.is_public = false;
            bob.friends = vec!["alice@example.com".to_string()];
            let mut carol = new_user_record("carol@example.com", now, "Carol #qa");
            carol.is_public = true;
            let mut dave = new_user_record("dave@example.com", now, "Dave #secret");
            dave.is_public = false;
            users.insert(alice);
            users.insert(bob);
            users.insert(carol);
            users.insert(dave);
        }

        let visible = visible_user_profiles_for_session(&state, "alice@example.com", None, None)
            .unwrap()
            .into_iter()
            .map(|profile| normalize_email(&profile.email))
            .collect::<Vec<_>>();

        assert!(visible.contains(&"alice@example.com".to_string()));
        assert!(visible.contains(&"bob@example.com".to_string()));
        assert!(visible.contains(&"carol@example.com".to_string()));
        assert!(!visible.contains(&"dave@example.com".to_string()));
    }

    #[test]
    fn agent_directory_visibility_policy_filters_profiles() {
        let state = test_state();
        let now = now_unix();
        {
            let mut users = state.users.lock().unwrap();
            let mut alice = new_user_record("alice@example.com", now, "Alice #ops");
            alice.is_public = false;
            alice.friends = vec!["bob@example.com".to_string()];
            let mut bob = new_user_record("bob@example.com", now, "Bob #review");
            bob.is_public = false;
            bob.friends = vec!["alice@example.com".to_string()];
            let mut carol = new_user_record("carol@example.com", now, "Carol #qa");
            carol.is_public = true;
            let mut dave = new_user_record("dave@example.com", now, "Dave #support");
            dave.is_public = true;
            let mut erin = new_user_record("erin@example.com", now, "Erin #private");
            erin.is_public = false;
            users.insert(alice);
            users.insert(bob);
            users.insert(carol);
            users.insert(dave);
            users.insert(erin);
        }
        db_store_human_rating(&state, "carol@example.com", "alice@example.com", 8.0, None)
            .unwrap();
        db_store_human_rating(&state, "dave@example.com", "alice@example.com", 4.0, None)
            .unwrap();
        db_store_human_rating(&state, "erin@example.com", "alice@example.com", 10.0, None)
            .unwrap();

        let visible = |directory_visibility| {
            let agent = AgentContext {
                email: "alice@example.com".to_string(),
                agent_id: "agent-alice".to_string(),
                agent_name: "Alice Agent".to_string(),
                directory_visibility,
                directory_min_reputation: 6.0,
            };
            agent_visible_profiles(&state, &agent, None, None)
                .unwrap()
                .into_iter()
                .map(|profile| normalize_email(&profile.email))
                .collect::<Vec<_>>()
        };

        let self_only = visible(AgentDirectoryVisibility::SelfOnly);
        assert_eq!(self_only, vec!["alice@example.com".to_string()]);

        let self_and_friends = visible(AgentDirectoryVisibility::SelfAndFriends);
        assert!(self_and_friends.contains(&"alice@example.com".to_string()));
        assert!(self_and_friends.contains(&"bob@example.com".to_string()));
        assert_eq!(self_and_friends.len(), 2);

        let public_users = visible(AgentDirectoryVisibility::PublicUsers);
        assert!(public_users.contains(&"alice@example.com".to_string()));
        assert!(public_users.contains(&"bob@example.com".to_string()));
        assert!(public_users.contains(&"carol@example.com".to_string()));
        assert!(public_users.contains(&"dave@example.com".to_string()));
        assert!(!public_users.contains(&"erin@example.com".to_string()));

        let reputable = visible(AgentDirectoryVisibility::ReputationAtLeast);
        assert!(reputable.contains(&"alice@example.com".to_string()));
        assert!(reputable.contains(&"bob@example.com".to_string()));
        assert!(reputable.contains(&"carol@example.com".to_string()));
        assert!(!reputable.contains(&"dave@example.com".to_string()));
        assert!(!reputable.contains(&"erin@example.com".to_string()));
    }

    #[test]
    fn agent_created_tasks_are_scoped_to_secret_owner() {
        let state = test_state();
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
            agent_id: "agent-alice".to_string(),
            agent_name: "Alice Agent".to_string(),
            directory_visibility: AgentDirectoryVisibility::SelfOnly,
            directory_min_reputation: default_agent_directory_min_reputation(),
        };

        let task = create_agent_task_from_agent(
            &state,
            &agent,
            CreateAgentTask {
                title: "Review rollout #ops".to_string(),
                description: "Check the deployment and ignore #admin".to_string(),
                steps: vec!["Open dashboard".to_string(), "Confirm status".to_string()],
                tags: vec!["#qa".to_string(), "#admin".to_string()],
                due_at: Some(12345),
            },
        )
        .unwrap();

        assert_eq!(task.assigned_to, "alice@example.com");
        assert!(task.tags.contains(&"#ops".to_string()));
        assert!(task.tags.contains(&"#qa".to_string()));
        assert!(!task.tags.contains(&"#admin".to_string()));

        let alice_tasks = db_list_agent_tasks(&state, "alice@example.com", None, true, 20).unwrap();
        let bob_tasks = db_list_agent_tasks(&state, "bob@example.com", None, true, 20).unwrap();
        assert_eq!(alice_tasks.len(), 1);
        assert_eq!(alice_tasks[0].id, task.id);
        assert!(bob_tasks.is_empty());
    }

    #[test]
    fn agents_rate_visible_humans_with_agent_identity() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            users.insert(new_user_record("alice@example.com", 1, "Alice"));
            let mut bob = new_user_record("bob@example.com", 1, "Bob");
            bob.visibility = ProfileVisibility::Public;
            users.insert(bob);
        }
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
            agent_id: "agent-alice".to_string(),
            agent_name: "Alice Agent".to_string(),
            directory_visibility: AgentDirectoryVisibility::PublicUsers,
            directory_min_reputation: default_agent_directory_min_reputation(),
        };

        let rated = rate_human_from_agent(
            &state,
            &agent,
            RateHumanRequest {
                rated_email: "bob@example.com".to_string(),
                score: 8.0,
                note: None,
            },
        )
        .unwrap();
        assert_eq!(rated.ratings_count, 1);

        let repeated = rate_human_from_agent(
            &state,
            &agent,
            RateHumanRequest {
                rated_email: "bob@example.com".to_string(),
                score: 9.0,
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(repeated.status, StatusCode::CONFLICT);

        let own_profile = rate_human_from_agent(
            &state,
            &agent,
            RateHumanRequest {
                rated_email: "alice@example.com".to_string(),
                score: 10.0,
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(own_profile.message, "cannot rate your own human profile");
    }

    #[test]
    fn agent_can_read_replies_for_requests_it_created_for_visible_humans() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            users.insert(new_user_record("alice@example.com", 1, "Alice"));
            let mut bob = new_user_record("bob@example.com", 1, "Bob");
            bob.visibility = ProfileVisibility::Public;
            users.insert(bob);
        }
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
            agent_id: "agent-alice".to_string(),
            agent_name: "Alice Agent".to_string(),
            directory_visibility: AgentDirectoryVisibility::PublicUsers,
            directory_min_reputation: default_agent_directory_min_reputation(),
        };
        assert_eq!(
            resolve_request_target_human(&state, &agent, Some("bob@example.com")).unwrap(),
            "bob@example.com"
        );
        let mut request = test_human_request();
        request.assigned_to = Some("bob@example.com".to_string());
        request.created_by = Some("alice@example.com".to_string());
        request.created_by_agent_id = Some(agent.agent_id.clone());
        let answer = HumanAnswer {
            answer: "Looks good".to_string(),
            note: None,
            answered_by: "bob@example.com".to_string(),
            answered_at: now_unix(),
        };
        db_store_answer(&state, &request, &answer, false).unwrap();

        let replies = db_read_humen_replies(
            &state,
            "alice@example.com",
            ReadLateRepliesArgs {
                request_id: Some(request.id),
                since: None,
                unread_only: false,
                mark_read: false,
                limit: Some(10),
            },
        )
        .unwrap();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].request.assigned_to.as_deref(), Some("bob@example.com"));
        assert_eq!(replies[0].answer.answer, "Looks good");

        assert!(db_hide_human_request(&state, "bob@example.com", request.id).unwrap());
        assert!(!db_hide_human_request(&state, "bob@example.com", request.id).unwrap());
        assert!(db_hidden_request_ids(&state, "bob@example.com")
            .unwrap()
            .contains(&request.id));
        assert!(!db_hidden_request_ids(&state, "alice@example.com")
            .unwrap()
            .contains(&request.id));

        let replies = db_read_humen_replies(
            &state,
            "alice@example.com",
            ReadLateRepliesArgs {
                request_id: Some(request.id),
                since: None,
                unread_only: false,
                mark_read: false,
                limit: Some(10),
            },
        )
        .unwrap();
        assert_eq!(replies.len(), 1);
    }

    #[test]
    fn agent_connections_and_relation_requests_are_persisted() {
        let state = test_state();
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
            agent_id: String::new(),
            agent_name: String::new(),
            directory_visibility: AgentDirectoryVisibility::SelfOnly,
            directory_min_reputation: default_agent_directory_min_reputation(),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-humen-agent-id", "codex-local".parse().unwrap());
        headers.insert("x-humen-agent-name", "Codex".parse().unwrap());
        let payload = McpRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(1)),
            method: "initialize".to_string(),
            params: json!({
                "clientInfo": {
                    "name": "Codex",
                    "version": "5"
                }
            }),
        };

        let agent = db_touch_agent_connection(&state, &agent, &headers, &payload).unwrap();
        db_update_agent_current_task(&state, &agent, "Implement agents panel").unwrap();

        let listed = db_list_connected_agents(&state, "bob@example.com", 20).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Codex 5");
        assert_eq!(listed[0].current_task, "Implement agents panel");
        assert_eq!(listed[0].relation_status, AgentRelationStatus::None);

        let (status, message) = db_request_agent_friend(&state, &agent.agent_id, "bob@example.com", "hi")
            .unwrap();
        assert_eq!(status, AgentRelationStatus::HumanRequested);
        assert_eq!(message.direction, "human_to_agent");
        assert_eq!(message.read_at, None);
        let inbox = db_list_agent_inbox(&state, &agent.agent_id, false, false, 20).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].kind, "friend_request");
        assert_eq!(inbox[0].read_at, None);
        let read_inbox = db_list_agent_inbox(&state, &agent.agent_id, false, true, 20).unwrap();
        assert_eq!(read_inbox.len(), 1);
        assert!(read_inbox[0].read_at.is_some());
        let unread_inbox = db_list_agent_inbox(&state, &agent.agent_id, true, false, 20).unwrap();
        assert!(unread_inbox.is_empty());

        let agent_rating =
            db_store_agent_rating(&state, &agent.agent_id, "bob@example.com", 8.0, None).unwrap();
        assert_eq!(agent_rating.ratings_count, 1);
        assert_eq!(agent_rating.reputation, 8.0);
        let repeated =
            db_store_agent_rating(&state, &agent.agent_id, "bob@example.com", 9.0, None)
                .unwrap_err();
        assert_eq!(repeated.status, StatusCode::CONFLICT);
        assert_eq!(repeated.message, "you have already rated this agent");
        let own_agent =
            db_store_agent_rating(&state, &agent.agent_id, "alice@example.com", 10.0, None)
                .unwrap_err();
        assert_eq!(own_agent.message, "cannot rate your own agent");

        let (accepted, messages) =
            db_accept_agent_friend(&state, &agent.agent_id, "bob@example.com").unwrap();
        assert_eq!(accepted, AgentRelationStatus::Friends);
        assert!(messages
            .iter()
            .any(|message| message.kind == "friend_request" && message.status == "resolved"));
    }

    #[test]
    fn one_owner_keeps_only_one_connected_agent_card() {
        let state = test_state();
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
            agent_id: String::new(),
            agent_name: String::new(),
            directory_visibility: AgentDirectoryVisibility::SelfOnly,
            directory_min_reputation: default_agent_directory_min_reputation(),
        };
        let payload = McpRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(1)),
            method: "initialize".to_string(),
            params: json!({}),
        };
        let mut first_headers = HeaderMap::new();
        first_headers.insert("x-humen-agent-id", "first-agent".parse().unwrap());
        first_headers.insert("x-humen-agent-name", "First".parse().unwrap());
        let first = db_touch_agent_connection(&state, &agent, &first_headers, &payload).unwrap();

        let mut second_headers = HeaderMap::new();
        second_headers.insert("x-humen-agent-id", "second-agent".parse().unwrap());
        second_headers.insert("x-humen-agent-name", "Second".parse().unwrap());
        let second = db_touch_agent_connection(&state, &agent, &second_headers, &payload).unwrap();

        assert_ne!(first.agent_id, second.agent_id);
        let listed = db_list_connected_agents(&state, "bob@example.com", 20).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, second.agent_id);
        assert_eq!(listed[0].owner_email, "alice@example.com");
        assert_eq!(listed[0].name, "Second");
    }

    #[test]
    fn active_mcp_sse_stream_keeps_agent_online_after_request_window() {
        let state = test_state();
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
            agent_id: String::new(),
            agent_name: String::new(),
            directory_visibility: AgentDirectoryVisibility::SelfOnly,
            directory_min_reputation: default_agent_directory_min_reputation(),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-humen-agent-id", "codex-local".parse().unwrap());
        headers.insert("x-humen-agent-name", "Codex".parse().unwrap());
        let payload = McpRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(1)),
            method: "initialize".to_string(),
            params: json!({}),
        };
        db_touch_agent_connection(&state, &agent, &headers, &payload).unwrap();
        {
            let db = state.db.lock().unwrap();
            db.execute(
                "UPDATE agent_connections SET last_seen_at = ?1 WHERE owner_email = ?2",
                params![now_unix().saturating_sub(1000), "alice@example.com"],
            )
            .unwrap();
        }

        let listed = db_list_connected_agents(&state, "bob@example.com", 20).unwrap();
        assert!(!listed[0].online);

        state
            .mcp_streams
            .insert("alice@example.com".to_string(), Uuid::new_v4());
        let listed = db_list_connected_agents(&state, "bob@example.com", 20).unwrap();
        assert!(listed[0].online);
    }

    #[test]
    fn sse_presence_does_not_increment_agent_request_count() {
        let state = test_state();
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
            agent_id: String::new(),
            agent_name: String::new(),
            directory_visibility: AgentDirectoryVisibility::SelfOnly,
            directory_min_reputation: default_agent_directory_min_reputation(),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-humen-agent-id", "codex-local".parse().unwrap());
        headers.insert("x-humen-agent-name", "Codex".parse().unwrap());
        let touched = db_touch_agent_presence(&state, &agent, &headers).unwrap();
        assert_eq!(touched.agent_name, "Codex");

        let listed = db_list_connected_agents(&state, "bob@example.com", 20).unwrap();
        assert_eq!(listed[0].request_count, 0);
        assert_eq!(listed[0].last_request_at, None);
        assert_eq!(listed[0].last_tool, "mcp/sse");
    }

    #[test]
    fn agent_friend_request_to_human_can_be_accepted_by_human() {
        let state = test_state();
        let agent_id = "agent-123";
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
            agent_id: String::new(),
            agent_name: String::new(),
            directory_visibility: AgentDirectoryVisibility::SelfOnly,
            directory_min_reputation: default_agent_directory_min_reputation(),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-humen-agent-id", agent_id.parse().unwrap());
        let payload = McpRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(json!(1)),
            method: "tools/list".to_string(),
            params: json!({}),
        };
        let agent = db_touch_agent_connection(&state, &agent, &headers, &payload).unwrap();
        let (status, message) = db_request_human_friend_from_agent(
            &state,
            &agent.agent_id,
            "bob@example.com",
            "Can we connect?",
        )
        .unwrap();
        assert_eq!(status, AgentRelationStatus::AgentRequested);
        assert_eq!(message.direction, "agent_to_human");

        let listed = db_list_connected_agents(&state, "bob@example.com", 20).unwrap();
        assert_eq!(listed[0].relation_status, AgentRelationStatus::AgentRequested);
        assert_eq!(listed[0].pending_messages.len(), 1);
        assert_eq!(listed[0].pending_messages[0].direction, "agent_to_human");

        let (accepted, messages) =
            db_accept_agent_friend(&state, &agent.agent_id, "bob@example.com").unwrap();
        assert_eq!(accepted, AgentRelationStatus::Friends);
        assert!(messages
            .iter()
            .any(|message| message.direction == "agent_to_human" && message.status == "resolved"));
    }

    #[test]
    fn agent_can_accept_pending_human_friend_request_outside_directory_scope() {
        let state = test_state();
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
            agent_id: "agent-self-only".to_string(),
            agent_name: "Self only agent".to_string(),
            directory_visibility: AgentDirectoryVisibility::SelfOnly,
            directory_min_reputation: default_agent_directory_min_reputation(),
        };
        {
            let mut users = state.users.lock().unwrap();
            users.insert(new_user_record("alice@example.com", 1, "Alice"));
            users.insert(new_user_record("bob@example.com", 1, "Bob"));
        }

        assert!(resolve_visible_human_for_agent(&state, &agent, "bob@example.com").is_err());
        db_request_agent_friend(&state, &agent.agent_id, "bob@example.com", "please connect")
            .unwrap();

        let target =
            resolve_human_for_agent_friend_accept(&state, &agent, "bob@example.com").unwrap();
        assert_eq!(target, "bob@example.com");
        let (accepted, messages) = db_accept_agent_friend(&state, &agent.agent_id, &target).unwrap();
        assert_eq!(accepted, AgentRelationStatus::Friends);
        assert!(messages
            .iter()
            .any(|message| message.direction == "human_to_agent" && message.status == "resolved"));
    }

    #[test]
    fn webhook_help_prompt_renders_placeholders_and_can_be_blank() {
        let state = test_state();
        let request = test_human_request();
        let mut webhook =
            test_webhook("打开 {url}\n定位 {request_id} {short_id} {title}".to_string());

        let text = format_weixin_request_notification(&state, &webhook, &request);
        assert!(text.contains("打开 http://127.0.0.1:8787/mcp"));
        assert!(text.contains(&request.id.to_string()));
        assert!(text.contains(&request_short_id(request.id)));
        assert!(text.contains("Check release"));

        webhook.help_prompt = " \n ".to_string();
        let text = format_weixin_request_notification(&state, &webhook, &request);
        assert!(text.contains("标题：Check release"));
        assert!(!text.contains("打开 http://127.0.0.1:8787/mcp"));
        assert!(!text.contains("定位 "));
    }

    #[test]
    fn weixin_webhook_only_receives_requests_for_its_assigned_inbox() {
        let state = test_state();
        let mut webhook = test_webhook(String::new());
        webhook.assigned_to = Some("alice@example.com".to_string());

        let mut alice_request = test_human_request();
        alice_request.assigned_to = Some("alice@example.com".to_string());
        let mut bob_request = test_human_request();
        bob_request.assigned_to = Some("bob@example.com".to_string());
        let mut broadcast_request = test_human_request();
        broadcast_request.assigned_to = None;

        assert!(webhook_receives_request(&state, &webhook, &alice_request));
        assert!(!webhook_receives_request(&state, &webhook, &bob_request));
        assert!(!webhook_receives_request(&state, &webhook, &broadcast_request));
    }

    #[test]
    fn weixin_unmatched_messages_do_not_create_human_requests() {
        let state = test_state();
        let mut webhook = test_webhook(String::new());
        webhook.assigned_to = Some("Alice@Example.COM".to_string());

        handle_weixin_incoming_message(
            &state,
            &webhook,
            &json!({
                "from_user_id": "friend",
                "text": "ping"
            }),
        );

        assert!(state.requests.is_empty());
    }

    #[test]
    fn weixin_request_notification_echo_is_not_recorded_as_answer() {
        let state = test_state();
        let mut webhook = test_webhook(
            "直接回复本消息就是回答。\n请求ID：{request_id}\n短ID：{short_id}".to_string(),
        );
        webhook.assigned_to = Some("alice@example.com".to_string());
        let mut request = test_human_request();
        request.assigned_to = Some("alice@example.com".to_string());
        state.requests.insert(request.id, request.clone());

        let outgoing_text = format_weixin_request_notification(&state, &webhook, &request);
        let incoming = IncomingMessage {
            source: "wechat".to_string(),
            sender: "bot-account".to_string(),
            content: outgoing_text,
            raw: json!({}),
        };

        assert!(answer_weixin_message(&state, &webhook, &incoming)
            .unwrap()
            .is_none());
        assert!(state.requests.contains_key(&request.id));
        assert!(db_get_request(&state, request.id).unwrap().is_none());
    }

    #[test]
    fn non_admin_webhook_save_is_limited_to_own_weixin_connection() {
        let state = test_state();
        let mut generic = test_webhook(String::new());
        generic.kind = "generic".to_string();
        generic.url = "https://example.com/webhook".to_string();
        generic.assigned_to = Some("bob@example.com".to_string());

        let mut bob_weixin = test_webhook(String::new());
        bob_weixin.assigned_to = Some("bob@example.com".to_string());

        let mut alice_weixin = test_webhook(String::new());
        alice_weixin.assigned_to = Some("alice@example.com".to_string());

        let mut current = AdminSettings::default();
        current.webhooks = vec![generic.clone(), bob_weixin.clone(), alice_weixin.clone()];

        let visible =
            webhooks_visible_to_session(&state, "alice@example.com", false, &current.webhooks);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, alice_weixin.id);

        let mut alice_update = alice_weixin.clone();
        alice_update.enabled = false;
        let merged = merge_webhooks_for_session(
            &state,
            "alice@example.com",
            false,
            current,
            vec![alice_update.clone()],
        )
        .unwrap();

        assert!(merged.webhooks.iter().any(|webhook| webhook.id == generic.id));
        assert!(merged
            .webhooks
            .iter()
            .any(|webhook| webhook.id == bob_weixin.id));
        assert!(merged.webhooks.iter().any(|webhook| {
            webhook.id == alice_weixin.id && webhook.enabled == alice_update.enabled
        }));

        let mut cross_user_weixin = test_webhook(String::new());
        cross_user_weixin.assigned_to = Some("bob@example.com".to_string());
        let err = validate_webhook_for_session(
            &state,
            "alice@example.com",
            false,
            &cross_user_weixin,
        )
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn unbound_weixin_webhooks_do_not_receive_or_answer_messages() {
        let state = test_state();
        let webhook = test_webhook(String::new());
        let mut request = test_human_request();
        request.assigned_to = Some("alice@example.com".to_string());
        state.requests.insert(request.id, request.clone());
        let incoming = IncomingMessage {
            source: "wechat".to_string(),
            sender: "friend".to_string(),
            content: format!("answer {}", request.id),
            raw: json!({}),
        };

        assert!(!webhook_receives_request(&state, &webhook, &request));
        assert!(answer_weixin_message(&state, &webhook, &incoming)
            .unwrap()
            .is_none());
        assert!(state.requests.contains_key(&request.id));
    }

    #[test]
    fn weixin_ready_requires_user_and_context_tokens() {
        let mut webhook = test_webhook(String::new());
        webhook.weixin_bot_token = Some("bot-token".to_string());
        webhook.weixin_user_id = Some("user-id".to_string());
        assert!(!weixin_webhook_ready(&webhook));

        webhook.weixin_context_token = Some("context-token".to_string());
        assert!(weixin_webhook_ready(&webhook));
    }

    #[test]
    fn github_session_key_can_see_requests_assigned_to_email_alias() {
        let state = test_state();
        let mut github_record = new_user_record("user@example.com", 1, "GitHub user");
        github_record.github_id = Some("42".to_string());
        github_record.login = Some("octo".to_string());
        {
            let mut users = state.users.lock().unwrap();
            users.users.insert("github:42".to_string(), github_record);
        }
        let mut request = test_human_request();
        request.assigned_to = Some("user@example.com".to_string());

        assert!(can_access_request(&state, "github:42", &request));
        assert!(can_access_request(&state, "octo", &request));
        assert!(!can_access_request(&state, "other@example.com", &request));
    }

    #[test]
    fn weixin_answers_cannot_cross_webhook_inbox_boundaries() {
        let state = test_state();
        let now = now_unix();
        let mut alice_request = test_human_request();
        alice_request.assigned_to = Some("alice@example.com".to_string());
        alice_request.created_at = now;
        alice_request.expires_at = now.saturating_add(60);
        let mut bob_request = test_human_request();
        bob_request.id = Uuid::new_v4();
        bob_request.assigned_to = Some("bob@example.com".to_string());
        bob_request.created_at = now;
        bob_request.expires_at = now.saturating_add(60);
        state
            .requests
            .insert(alice_request.id, alice_request.clone());
        state.requests.insert(bob_request.id, bob_request.clone());

        let mut webhook = test_webhook(String::new());
        webhook.assigned_to = Some("bob@example.com".to_string());
        webhook.weixin_last_request_id = Some(alice_request.id);
        let incoming = IncomingMessage {
            source: "wechat".to_string(),
            sender: "friend".to_string(),
            content: format!("answer {}", alice_request.id),
            raw: json!({}),
        };

        assert!(answer_weixin_message(&state, &webhook, &incoming)
            .unwrap()
            .is_none());
        assert!(state.requests.contains_key(&alice_request.id));
        assert!(state.requests.contains_key(&bob_request.id));

        let bob_incoming = IncomingMessage {
            source: "wechat".to_string(),
            sender: "friend".to_string(),
            content: format!("answer {}", request_short_id(bob_request.id)),
            raw: json!({}),
        };
        let answered = answer_weixin_message(&state, &webhook, &bob_incoming)
            .unwrap()
            .unwrap();
        assert_eq!(answered.request.id, bob_request.id);
        assert!(state.requests.contains_key(&alice_request.id));
        assert!(!state.requests.contains_key(&bob_request.id));
    }

    #[test]
    fn init_admin_writes_env_file() {
        let env_file = std::env::temp_dir().join(format!("humen-mcp-env-{}.env", Uuid::new_v4()));

        init_admin(InitAdminArgs {
            env_file: env_file.clone(),
            email: Some("admin-local".to_string()),
            admin_pass: Some("fixed-admin-pass".to_string()),
        })
        .unwrap();

        let raw = fs::read_to_string(env_file).unwrap();
        assert!(raw.contains("HUMEN_ADMIN_EMAIL=admin-local"));
        assert!(raw.contains("HUMEN_ADMIN_PASSWORD=fixed-admin-pass"));
        assert!(raw.contains("HUMEN_USERS_FILE=/var/lib/humen-mcp/users.json"));
        assert!(!raw.contains("HUMEN_SESSION_SECRET=change-this-to-a-long-random-secret"));
    }

    #[test]
    fn ratings_and_reports_feed_reputation_and_admin_mailbox() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            users.insert(new_user_record("alice@example.com", 1, "Alice"));
            users.insert(new_user_record("bob@example.com", 1, "Bob"));
            users.insert(new_user_record("carol@example.com", 1, "Carol"));
            users.save(&state.config.users_file).unwrap();
        }

        let rated = rate_human_from_actor(
            &state,
            "alice@example.com",
            RateHumanRequest {
                rated_email: "bob@example.com".to_string(),
                score: 8.0,
                note: None,
            },
        )
        .unwrap();
        assert_eq!(rated.ratings_count, 1);
        assert_eq!(rated.reputation, 8.0);

        let report = report_human_from_actor(
            &state,
            "carol@example.com",
            ReportHumanRequest {
                reported_email: "bob@example.com".to_string(),
                reason: "Unsafe answer".to_string(),
            },
        )
        .unwrap();
        assert_eq!(report.reported_email, "bob@example.com");

        let reports = db_list_human_reports(&state, 10).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].reason, "Unsafe answer");

        let reputation = db_reputation_summary_for(&state, "bob@example.com").unwrap();
        assert_eq!(reputation.ratings_count, 2);
        assert_eq!(reputation.reputation, 4.0);
    }

    #[test]
    fn human_memos_are_scoped_to_visible_profiles() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            users.insert(new_user_record("alice@example.com", 1, "Alice"));
            let mut bob = new_user_record("bob@example.com", 1, "Bob");
            bob.is_public = true;
            users.insert(bob);
            let mut dave = new_user_record("dave@example.com", 1, "Dave");
            dave.is_public = false;
            users.insert(dave);
        }

        let target =
            resolve_visible_human_memo_target(&state, "alice@example.com", "bob@example.com")
                .unwrap();
        let memo = db_create_human_memo(
            &state,
            &target,
            "alice@example.com",
            "  Follow up after lunch.  ",
        )
        .unwrap();
        assert_eq!(memo.target_email, "bob@example.com");
        assert_eq!(memo.author_email, "alice@example.com");
        assert_eq!(memo.body, "Follow up after lunch.");

        let memos = db_list_human_memos(&state, "bob@example.com", 10).unwrap();
        assert_eq!(memos.len(), 1);
        assert_eq!(memos[0].id, memo.id);
        assert_eq!(memos[0].read_at, None);
        assert_eq!(
            db_mark_human_memos_read(&state, "bob@example.com", "bob@example.com").unwrap(),
            1
        );
        let memos = db_list_human_memos(&state, "bob@example.com", 10).unwrap();
        assert!(memos[0].read_at.is_some());

        let agent_memo = db_create_human_memo_with_agent(
            &state,
            "bob@example.com",
            "alice@example.com",
            Some("agent-alice"),
            Some("Alice Agent"),
            "Agent follow-up.",
        )
        .unwrap();
        assert_eq!(agent_memo.author_agent_id.as_deref(), Some("agent-alice"));
        let memos = db_list_human_memos(&state, "bob@example.com", 10).unwrap();
        assert_eq!(memos[0].author_agent_name.as_deref(), Some("Alice Agent"));

        let hidden =
            resolve_visible_human_memo_target(&state, "alice@example.com", "dave@example.com")
                .unwrap_err();
        assert_eq!(hidden.status, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn public_profile_lookup_uses_platform_name_slug() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            let mut record = new_user_record("alice@example.com", 1, "Alice");
            record.is_public = true;
            users.insert(record);
            users.save(&state.config.users_file).unwrap();
        }

        let profile = public_profile_by_platform_name(&state, "alice")
            .unwrap()
            .expect("profile exists");
        assert_eq!(profile.platform_name, "alice");
        assert_eq!(profile.email, "alice@example.com");
        assert_eq!(profile.profile, "Alice");
    }

    #[test]
    fn human_agent_memo_writes_to_agent_inbox() {
        let state = test_state();
        {
            let db = state.db.lock().unwrap();
            db.execute(
                "INSERT INTO agent_connections \
                 (id, owner_email, name, description, current_task, last_tool, first_seen_at, last_seen_at, last_request_at, request_count) \
                 VALUES (?1, ?2, ?3, ?4, '', ?5, 1, 1, 1, 1)",
                params!["agent-1", "alice@example.com", "Agent", "Desc", "tools/call"],
            )
            .unwrap();
        }

        let message = db_create_agent_memo_from_human(&state, "agent-1", "bob@example.com", "Ping the agent.")
            .unwrap();
        assert_eq!(message.agent_id, "agent-1");
        assert_eq!(message.human_email, "bob@example.com");
        assert_eq!(message.direction, "human_to_agent");
        assert_eq!(message.kind, "memo");
        assert_eq!(message.body, "Ping the agent.");

        let inbox = db_list_agent_inbox(&state, "agent-1", false, false, 10).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].id, message.id);
    }

    #[test]
    fn ratings_validate_targets_and_reject_repeat_scores() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            users.insert(new_user_record("alice@example.com", 1, "Alice"));
            users.insert(new_user_record("bob@example.com", 1, "Bob"));
            users.save(&state.config.users_file).unwrap();
        }

        let initial = db_reputation_summary_for(&state, "bob@example.com").unwrap();
        assert_eq!(initial.ratings_count, 0);
        assert_eq!(initial.reputation, 5.0);

        let invalid = rate_human_from_actor(
            &state,
            "alice@example.com",
            RateHumanRequest {
                rated_email: "bob@example.com".to_string(),
                score: 11.0,
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(invalid.status, StatusCode::BAD_REQUEST);

        let self_rating = rate_human_from_actor(
            &state,
            "alice@example.com",
            RateHumanRequest {
                rated_email: "alice@example.com".to_string(),
                score: 10.0,
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(self_rating.message, "cannot rate or report yourself");

        let missing = rate_human_from_actor(
            &state,
            "alice@example.com",
            RateHumanRequest {
                rated_email: "missing@example.com".to_string(),
                score: 7.0,
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(missing.message, "target human not found");

        let first = rate_human_from_actor(
            &state,
            "alice@example.com",
            RateHumanRequest {
                rated_email: "bob@example.com".to_string(),
                score: 7.0,
                note: Some("solid".to_string()),
            },
        )
        .unwrap();
        assert_eq!(first.ratings_count, 1);
        assert_eq!(first.reputation, 7.0);

        let repeated = rate_human_from_actor(
            &state,
            "alice@example.com",
            RateHumanRequest {
                rated_email: "bob@example.com".to_string(),
                score: 9.0,
                note: Some("better".to_string()),
            },
        )
        .unwrap_err();
        assert_eq!(repeated.status, StatusCode::CONFLICT);
        assert_eq!(repeated.message, "you have already rated this human");
        let unchanged = db_reputation_summary_for(&state, "bob@example.com").unwrap();
        assert_eq!(unchanged.ratings_count, 1);
        assert_eq!(unchanged.reputation, 7.0);
    }

    #[test]
    fn ratings_resolve_github_profiles_to_stable_identity() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            users.insert(new_user_record("rater@example.com", 1, "Rater"));
            users.save(&state.config.users_file).unwrap();
        }
        let account_key =
            upsert_github_user(&state, "42", Some("octo-human"), "public@example.com").unwrap();
        assert_eq!(account_key, "github:42");

        let profiles = user_profiles(&state, None, None).unwrap();
        let github_profile = profiles
            .iter()
            .find(|profile| profile.login.as_deref() == Some("octo-human"))
            .unwrap();
        assert_eq!(github_profile.email, "github:42");

        let rated = rate_human_from_actor(
            &state,
            "rater@example.com",
            RateHumanRequest {
                rated_email: "public@example.com".to_string(),
                score: 8.0,
                note: None,
            },
        )
        .unwrap();
        assert_eq!(rated.ratings_count, 1);
        assert_eq!(
            db_reputation_summary_for(&state, "github:42")
                .unwrap()
                .reputation,
            8.0
        );
        assert_eq!(
            db_reputation_summary_for(&state, "public@example.com")
                .unwrap()
                .ratings_count,
            0
        );
    }

    #[test]
    fn github_reputation_seed_sets_initial_score_and_blends_with_ratings() {
        let state = test_state();
        let seed = ReputationSeed {
            source: "github".to_string(),
            score: 8.0,
            weight: 2.0,
            details: json!({ "login": "alice" }),
        };

        let initial = db_upsert_reputation_seed(&state, "alice@example.com", seed).unwrap();
        assert_eq!(initial.ratings_count, 0);
        assert_eq!(initial.reputation, 8.0);
        assert_eq!(
            initial.reputation_breakdown.seed_source.as_deref(),
            Some("github")
        );
        assert_eq!(initial.reputation_breakdown.seed_weight, 2.0);
        assert_eq!(initial.reputation_breakdown.feedback_weight, 0.0);

        let blended = db_store_human_rating(
            &state,
            "alice@example.com",
            "bob@example.com",
            4.0,
            None,
        )
        .unwrap();
        assert_eq!(blended.ratings_count, 1);
        assert!((blended.reputation - 6.666_666_666).abs() < 0.000_001);
        assert_eq!(blended.reputation_breakdown.seed_weight, 2.0);
        assert_eq!(blended.reputation_breakdown.feedback_weight, 1.0);
        assert_eq!(blended.reputation_breakdown.total_weight, 3.0);
        assert!(blended.reputation_breakdown.confidence > 0.0);
    }

    #[test]
    fn ratings_are_weighted_by_rater_reputation() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            users.insert(new_user_record("trusted@example.com", 1, "Trusted"));
            users.insert(new_user_record("newbie@example.com", 1, "Newbie"));
            users.insert(new_user_record("bob@example.com", 1, "Bob"));
        }

        db_upsert_reputation_seed(
            &state,
            "trusted@example.com",
            ReputationSeed {
                source: "github".to_string(),
                score: 10.0,
                weight: 2.0,
                details: json!({ "login": "trusted" }),
            },
        )
        .unwrap();
        db_upsert_reputation_seed(
            &state,
            "newbie@example.com",
            ReputationSeed {
                source: "github".to_string(),
                score: 1.0,
                weight: 2.0,
                details: json!({ "login": "newbie" }),
            },
        )
        .unwrap();

        rate_human_from_actor(
            &state,
            "trusted@example.com",
            RateHumanRequest {
                rated_email: "bob@example.com".to_string(),
                score: 10.0,
                note: None,
            },
        )
        .unwrap();
        rate_human_from_actor(
            &state,
            "newbie@example.com",
            RateHumanRequest {
                rated_email: "bob@example.com".to_string(),
                score: 0.0,
                note: None,
            },
        )
        .unwrap();

        let summary = db_reputation_summary_for(&state, "bob@example.com").unwrap();
        assert_eq!(summary.ratings_count, 2);
        assert!((summary.reputation - 8.0).abs() < 0.000_001);
    }

    #[test]
    fn visible_profiles_sort_by_reputation_after_presence() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            users.insert(new_user_record("alice@example.com", 1, "Alice"));
            let mut bob = new_user_record("bob@example.com", 1, "Bob");
            bob.is_public = true;
            let mut carol = new_user_record("carol@example.com", 1, "Carol");
            carol.is_public = true;
            users.insert(bob);
            users.insert(carol);
        }

        db_upsert_reputation_seed(
            &state,
            "bob@example.com",
            ReputationSeed {
                source: "github".to_string(),
                score: 9.0,
                weight: 2.0,
                details: json!({ "login": "bob" }),
            },
        )
        .unwrap();
        db_upsert_reputation_seed(
            &state,
            "carol@example.com",
            ReputationSeed {
                source: "github".to_string(),
                score: 2.0,
                weight: 2.0,
                details: json!({ "login": "carol" }),
            },
        )
        .unwrap();

        let emails: Vec<_> = visible_user_profiles_for_session(
            &state,
            "alice@example.com",
            None,
            None,
        )
        .unwrap()
        .into_iter()
        .map(|profile| normalize_email(&profile.email))
        .collect();

        assert_eq!(
            emails,
            vec![
                "bob@example.com".to_string(),
                "alice@example.com".to_string(),
                "carol@example.com".to_string()
            ]
        );
    }

    #[test]
    fn leaderboard_includes_visible_seeded_profiles_without_handled_requests() {
        let state = test_state();
        {
            let mut users = state.users.lock().unwrap();
            let mut alice = new_user_record("alice@example.com", 1, "Alice");
            alice.is_public = true;
            users.insert(alice);
        }
        db_upsert_reputation_seed(
            &state,
            "alice@example.com",
            ReputationSeed {
                source: "github".to_string(),
                score: 8.5,
                weight: 2.0,
                details: json!({ "login": "alice" }),
            },
        )
        .unwrap();

        let profiles = user_profiles(&state, None, None).unwrap();
        let entries = leaderboard_entries_for_profiles(&state, profiles).unwrap();
        let alice = entries
            .iter()
            .find(|entry| normalize_email(&entry.email) == "alice@example.com")
            .unwrap();

        assert_eq!(alice.requests_handled, 0);
        assert_eq!(alice.sent_tokens, 0);
        assert_eq!(alice.reputation, 8.5);
        assert_eq!(
            alice.reputation_breakdown.seed_source.as_deref(),
            Some("github")
        );
    }

    #[test]
    fn github_account_cache_reuses_fresh_snapshot_and_expires_old_entries() {
        let state = test_state();
        let snapshot = GithubAccountSnapshot {
            login: "Alice".to_string(),
            account_created_at: Some("2016-01-01T00:00:00Z".to_string()),
            public_repos: 12,
            public_gists: 1,
            followers: 20,
            following: 5,
            total_stars_sampled: 30,
            source_repos_sampled: 10,
            fork_repos_sampled: 2,
            recent_events_sampled: 3,
            recent_activity_year: Some(2026),
            fetched_at: 100,
        };

        db_store_github_account_snapshot(&state, &snapshot, 50).unwrap();
        assert_eq!(
            db_get_fresh_github_account_snapshot(&state, "alice", 149)
                .unwrap()
                .unwrap()
                .public_repos,
            12
        );
        assert!(db_get_fresh_github_account_snapshot(&state, "alice", 151)
            .unwrap()
            .is_none());
    }

    #[test]
    fn admin_github_api_token_is_write_only_and_preserved_when_missing() {
        let state = test_state();
        {
            let mut settings = state.admin_settings.lock().unwrap();
            settings.github_api_token = Some("old-token".to_string());
        }

        let stored = state.admin_settings.lock().unwrap().clone();
        let response = admin_settings_response(&state, stored);
        assert!(response.github_api_token_configured);
        assert!(response.github_api_token.is_none());

        let mut payload = AdminSettings::default();
        payload.github_api_token = None;
        merge_admin_secret_settings(&state, &mut payload).unwrap();
        let sanitized = sanitize_admin_settings(payload);
        assert_eq!(sanitized.github_api_token.as_deref(), Some("old-token"));

        let mut clear_payload = AdminSettings::default();
        clear_payload.github_api_token = Some(" ".to_string());
        merge_admin_secret_settings(&state, &mut clear_payload).unwrap();
        let sanitized = sanitize_admin_settings(clear_payload);
        assert!(sanitized.github_api_token.is_none());
    }

    #[test]
    fn github_reputation_algorithm_rewards_established_public_accounts() {
        let user = json!({
            "login": "alice",
            "created_at": "2015-01-01T00:00:00Z",
            "public_repos": 40,
            "public_gists": 2,
            "followers": 150,
            "following": 20
        });
        let repos = json!([
            { "stargazers_count": 50, "fork": false, "pushed_at": "2026-03-01T00:00:00Z" },
            { "stargazers_count": 20, "fork": false, "pushed_at": "2025-09-01T00:00:00Z" },
            { "stargazers_count": 0, "fork": true, "pushed_at": "2024-01-01T00:00:00Z" }
        ]);
        let events = json!([{ "type": "PushEvent" }]);
        let snapshot = github_account_snapshot_from_api("alice", &user, &repos, &events, 1_783_468_800);
        let seed = github_reputation_seed_from_snapshot(snapshot);

        assert!(seed.score > 7.0);
        assert_eq!(seed.weight, 2.0);
    }

    fn tag_count(tags: &[Value], tag: &str) -> Option<u64> {
        tags.iter()
            .find(|item| item["tag"].as_str() == Some(tag))
            .and_then(|item| item["count"].as_u64())
    }

    fn test_human_request() -> HumanRequest {
        HumanRequest {
            id: Uuid::new_v4(),
            kind: TaskKind::Text,
            title: "Check release".to_string(),
            prompt: "Is the release ready?".to_string(),
            choices: Vec::new(),
            image_url: None,
            image_base64: None,
            image_mime_type: None,
            steps: Vec::new(),
            created_at: 100,
            timeout_seconds: 60,
            expires_at: 160,
            tags: Vec::new(),
            assigned_to: None,
            created_by: None,
            created_by_agent_id: None,
        }
    }

    fn test_webhook(help_prompt: String) -> WebhookConfig {
        WebhookConfig {
            id: Uuid::new_v4(),
            name: "Test webhook".to_string(),
            url: String::new(),
            enabled: true,
            assigned_to: None,
            secret: None,
            kind: "wechat".to_string(),
            help_prompt,
            weixin_qrcode: None,
            weixin_qrcode_url: None,
            weixin_status: None,
            weixin_status_message: None,
            weixin_bot_token: None,
            weixin_account_id: None,
            weixin_base_url: None,
            weixin_user_id: None,
            weixin_context_token: None,
            weixin_ready: false,
            weixin_last_request_id: None,
            weixin_get_updates_buf: None,
            weixin_last_error: None,
            weixin_last_seen_at: None,
            weixin_long_poll_timeout_ms: None,
            weixin_api_timeout_ms: None,
        }
    }
}
