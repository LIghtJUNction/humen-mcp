const PASSKEY_CHALLENGE_TTL_SECONDS: u64 = 10 * 60;

async fn list_passkeys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PasskeyInfo>>, ApiError> {
    let session = require_session(&state, &headers)?;
    let email = normalize_email(&session.user.email);
    let users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let mut passkeys = users
        .users
        .get(&email)
        .map(|record| {
            record
                .passkeys
                .iter()
                .map(passkey_info)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    passkeys.sort_by_key(|passkey| passkey.created_at);
    Ok(Json(passkeys))
}

async fn passkey_registration_start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<PasskeyRegistrationStartResponse>, ApiError> {
    let session = require_session(&state, &headers)?;
    cleanup_expired_passkey_challenges(&state);
    let webauthn = require_webauthn(&state)?;
    let (email, user_id, display_name, exclude_credentials) =
        ensure_passkey_registration_user(&state, &session.user.email)?;
    let (options, registration_state) = webauthn
        .start_passkey_registration(
            user_id,
            &email,
            &display_name,
            (!exclude_credentials.is_empty()).then_some(exclude_credentials),
        )
        .map_err(passkey_api_error)?;
    let registration_id = Uuid::new_v4();
    state.passkey_registrations.insert(
        registration_id,
        PendingPasskeyRegistration {
            email,
            state: registration_state,
            created_at: now_unix(),
        },
    );
    Ok(Json(PasskeyRegistrationStartResponse {
        registration_id,
        options,
    }))
}

async fn passkey_registration_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<PasskeyRegistrationFinishRequest>,
) -> Result<Json<Vec<PasskeyInfo>>, ApiError> {
    let session = require_session(&state, &headers)?;
    cleanup_expired_passkey_challenges(&state);
    let webauthn = require_webauthn(&state)?;
    let (_, pending) = state
        .passkey_registrations
        .remove(&payload.registration_id)
        .ok_or_else(|| ApiError::bad_request("passkey registration challenge expired"))?;
    let session_email = normalize_email(&session.user.email);
    if normalize_email(&pending.email) != session_email {
        return Err(ApiError::unauthorized(
            "passkey registration does not belong to this session",
        ));
    }
    if pending.created_at + PASSKEY_CHALLENGE_TTL_SECONDS < now_unix() {
        return Err(ApiError::bad_request(
            "passkey registration challenge expired",
        ));
    }
    let passkey = webauthn
        .finish_passkey_registration(&payload.credential, &pending.state)
        .map_err(passkey_api_error)?;
    store_passkey(&state, &session_email, payload.name.as_deref(), passkey)?;
    list_passkeys(State(state), headers).await
}

async fn delete_passkey(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PasskeyInfo>>, ApiError> {
    let session = require_session(&state, &headers)?;
    let email = normalize_email(&session.user.email);
    let mut passkeys = {
        let mut users = state
            .users
            .lock()
            .map_err(|_| ApiError::internal("user store lock poisoned"))?;
        if let Some(record) = users.users.get_mut(&email) {
            record.passkeys.retain(|passkey| passkey.id != id);
        }
        users
            .save(&state.config.users_file)
            .map_err(|err| ApiError::internal(format!("failed to save passkeys: {err}")))?;
        users
            .users
            .get(&email)
            .map(|record| {
                record
                    .passkeys
                    .iter()
                    .map(passkey_info)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    passkeys.sort_by_key(|passkey| passkey.created_at);
    Ok(Json(passkeys))
}

async fn passkey_authentication_start(
    State(state): State<AppState>,
    Json(payload): Json<PasskeyAuthenticationStartRequest>,
) -> Result<Json<PasskeyAuthenticationStartResponse>, ApiError> {
    cleanup_expired_passkey_challenges(&state);
    let webauthn = require_webauthn(&state)?;
    let email = normalize_email(&payload.email);
    validate_email_like_identifier(&email)?;
    ensure_user_allowed(&state, &email)?;
    let passkeys = {
        let users = state
            .users
            .lock()
            .map_err(|_| ApiError::internal("user store lock poisoned"))?;
        users
            .users
            .get(&email)
            .map(|record| {
                record
                    .passkeys
                    .iter()
                    .map(|stored| stored.credential.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    if passkeys.is_empty() {
        return Err(ApiError::unauthorized(
            "this account does not have passkeys enabled",
        ));
    }
    let (options, authentication_state) = webauthn
        .start_passkey_authentication(&passkeys)
        .map_err(passkey_api_error)?;
    let authentication_id = Uuid::new_v4();
    state.passkey_authentications.insert(
        authentication_id,
        PendingPasskeyAuthentication {
            email,
            state: authentication_state,
            created_at: now_unix(),
        },
    );
    Ok(Json(PasskeyAuthenticationStartResponse {
        authentication_id,
        options,
    }))
}

async fn passkey_authentication_finish(
    State(state): State<AppState>,
    Json(payload): Json<PasskeyAuthenticationFinishRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    cleanup_expired_passkey_challenges(&state);
    let webauthn = require_webauthn(&state)?;
    let (_, pending) = state
        .passkey_authentications
        .remove(&payload.authentication_id)
        .ok_or_else(|| ApiError::bad_request("passkey authentication challenge expired"))?;
    if pending.created_at + PASSKEY_CHALLENGE_TTL_SECONDS < now_unix() {
        return Err(ApiError::bad_request(
            "passkey authentication challenge expired",
        ));
    }
    let auth_result = webauthn
        .finish_passkey_authentication(&payload.credential, &pending.state)
        .map_err(passkey_api_error)?;
    update_passkey_after_authentication(&state, &pending.email, &auth_result)?;
    Ok(Json(
        state.create_session(pending.email, AuthProvider::Passkey),
    ))
}

fn require_webauthn(state: &AppState) -> Result<Arc<Webauthn>, ApiError> {
    state
        .webauthn
        .clone()
        .ok_or_else(|| ApiError::bad_request("passkey support is not available on this origin"))
}

fn cleanup_expired_passkey_challenges(state: &AppState) {
    let cutoff = now_unix().saturating_sub(PASSKEY_CHALLENGE_TTL_SECONDS);
    state
        .passkey_registrations
        .retain(|_, pending| pending.created_at >= cutoff);
    state
        .passkey_authentications
        .retain(|_, pending| pending.created_at >= cutoff);
}

fn ensure_passkey_registration_user(
    state: &AppState,
    email: &str,
) -> Result<(String, Uuid, String, Vec<CredentialID>), ApiError> {
    let email = normalize_email(email);
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let now = now_unix();
    let record = users
        .users
        .entry(email.clone())
        .or_insert_with(|| new_user_record(email.clone(), now, default_profile_template(&email)));
    prepare_user_record(record);
    let user_id = record.passkey_user_id.unwrap_or_else(Uuid::new_v4);
    record.passkey_user_id = Some(user_id);
    let display_name = passkey_display_name(record);
    let exclude_credentials = record
        .passkeys
        .iter()
        .map(|stored| stored.credential.cred_id().clone())
        .collect::<Vec<_>>();
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save passkey user id: {err}")))?;
    Ok((email, user_id, display_name, exclude_credentials))
}

fn store_passkey(
    state: &AppState,
    email: &str,
    name: Option<&str>,
    passkey: Passkey,
) -> Result<(), ApiError> {
    let email = normalize_email(email);
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    if passkey_credential_exists(&users, passkey.cred_id()) {
        return Err(ApiError::bad_request(
            "this passkey is already registered to an account",
        ));
    }
    let record = users
        .users
        .get_mut(&email)
        .ok_or_else(|| ApiError::unauthorized("user account no longer exists"))?;
    prepare_user_record(record);
    record.passkeys.push(StoredPasskey {
        id: Uuid::new_v4(),
        name: normalize_optional_value(name).unwrap_or_else(|| default_passkey_name(record)),
        created_at: now_unix(),
        last_used_at: None,
        credential: passkey,
    });
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to save passkey: {err}")))
}

fn update_passkey_after_authentication(
    state: &AppState,
    email: &str,
    auth_result: &webauthn_rs::prelude::AuthenticationResult,
) -> Result<(), ApiError> {
    let email = normalize_email(email);
    let mut users = state
        .users
        .lock()
        .map_err(|_| ApiError::internal("user store lock poisoned"))?;
    let record = users
        .users
        .get_mut(&email)
        .ok_or_else(|| ApiError::unauthorized("user account no longer exists"))?;
    let now = now_unix();
    let mut matched = false;
    for stored in &mut record.passkeys {
        if stored.credential.update_credential(auth_result).is_some() {
            stored.last_used_at = Some(now);
            matched = true;
            break;
        }
    }
    if !matched {
        return Err(ApiError::unauthorized("passkey is not registered to this account"));
    }
    record.last_login_at = now;
    users
        .save(&state.config.users_file)
        .map_err(|err| ApiError::internal(format!("failed to update passkey: {err}")))
}

fn passkey_credential_exists(users: &UserStore, credential_id: &CredentialID) -> bool {
    users.users.values().any(|record| {
        record
            .passkeys
            .iter()
            .any(|stored| stored.credential.cred_id().as_slice() == credential_id.as_slice())
    })
}

fn passkey_display_name(record: &UserRecord) -> String {
    record
        .profile
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&record.email)
        .to_string()
}

fn default_passkey_name(record: &UserRecord) -> String {
    let next = record.passkeys.len() + 1;
    format!("{} Passkey {next}", passkey_display_name(record))
}

fn passkey_info(passkey: &StoredPasskey) -> PasskeyInfo {
    PasskeyInfo {
        id: passkey.id,
        name: passkey.name.clone(),
        created_at: passkey.created_at,
        last_used_at: passkey.last_used_at,
    }
}

fn passkey_api_error(error: webauthn_rs::prelude::WebauthnError) -> ApiError {
    ApiError::bad_request(format!("passkey ceremony failed: {error:?}"))
}
