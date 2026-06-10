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
        assert!(tool_names.contains(&"list_humen_plugins"));
        assert!(tool_names.contains(&"create_humen_request_from_template"));
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
        assert!(public_users.contains(&"carol@example.com".to_string()));
        assert!(public_users.contains(&"dave@example.com".to_string()));
        assert!(!public_users.contains(&"bob@example.com".to_string()));
        assert!(!public_users.contains(&"erin@example.com".to_string()));

        let reputable = visible(AgentDirectoryVisibility::ReputationAtLeast);
        assert!(reputable.contains(&"alice@example.com".to_string()));
        assert!(reputable.contains(&"carol@example.com".to_string()));
        assert!(!reputable.contains(&"dave@example.com".to_string()));
        assert!(!reputable.contains(&"erin@example.com".to_string()));
    }

    #[test]
    fn agent_created_tasks_are_scoped_to_secret_owner() {
        let state = test_state();
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
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
    fn ratings_validate_targets_and_update_existing_score() {
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

        let updated = rate_human_from_actor(
            &state,
            "alice@example.com",
            RateHumanRequest {
                rated_email: "bob@example.com".to_string(),
                score: 9.0,
                note: Some("better".to_string()),
            },
        )
        .unwrap();
        assert_eq!(updated.ratings_count, 1);
        assert_eq!(updated.reputation, 9.0);
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
        }
    }

    fn test_webhook(help_prompt: String) -> WebhookConfig {
        WebhookConfig {
            id: Uuid::new_v4(),
            name: "Test webhook".to_string(),
            url: String::new(),
            enabled: true,
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
            weixin_last_request_id: None,
            weixin_get_updates_buf: None,
            weixin_last_error: None,
            weixin_last_seen_at: None,
            weixin_long_poll_timeout_ms: None,
            weixin_api_timeout_ms: None,
        }
    }
}
