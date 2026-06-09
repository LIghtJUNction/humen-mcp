#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> AppState {
        AppState::new(Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            public_base_url: "http://127.0.0.1:8787".to_string(),
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
            trash_retention_seconds: 60,
            cleanup_interval_seconds: 60,
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

        upsert_github_user(&state, "admin-local").unwrap();
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
        assert!(agent.can_view_directory);
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
    fn agent_created_tasks_are_scoped_to_secret_owner() {
        let state = test_state();
        let agent = AgentContext {
            email: "alice@example.com".to_string(),
            can_view_directory: false,
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
