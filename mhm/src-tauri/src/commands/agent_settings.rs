use super::{get_user, require_admin_user, AppState};
use crate::{
    agent::settings::{
        get_ceo_cloud_data_opt_in as read_ceo_cloud_data_opt_in,
        set_ceo_cloud_data_opt_in_idempotent, SET_CEO_CLOUD_DATA_OPT_IN_COMMAND,
    },
    agent::{
        config::{
            get_ceo_telegram_config as read_ceo_telegram_config,
            set_ceo_telegram_config_idempotent,
            update_ceo_telegram_secret_presence_with_guard_idempotent, CeoTelegramConfig,
            CeoTelegramGateStatus, SET_CEO_TELEGRAM_CONFIG_COMMAND,
            SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
        },
        secrets::{
            agent_secret_fingerprint, AgentSecretKind, AgentSecretStore, KeychainSecretStore,
        },
    },
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::WriteCommandContext,
    models::User,
};
use sqlx::{Pool, Sqlite};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::State;

pub(crate) fn require_ceo_cloud_opt_in_admin(user: Option<User>) -> CommandResult<User> {
    require_admin_user(user)
}

#[tauri::command]
pub async fn get_ceo_cloud_data_opt_in(state: State<'_, AppState>) -> CommandResult<bool> {
    let _user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    read_ceo_cloud_data_opt_in(&state.db).await
}

#[tauri::command]
pub async fn set_ceo_cloud_data_opt_in(
    state: State<'_, AppState>,
    enabled: bool,
    idempotency_key: String,
) -> CommandResult<()> {
    let user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let mut ctx = WriteCommandContext::for_scoped_command(
        uuid::Uuid::new_v4().to_string(),
        idempotency_key,
        SET_CEO_CLOUD_DATA_OPT_IN_COMMAND,
    )?;
    ctx.actor_id = Some(user.id.clone());

    set_ceo_cloud_data_opt_in_idempotent(&state.db, &ctx, enabled)
        .await
        .map(|_| ())
}

#[tauri::command]
#[allow(dead_code)]
pub async fn get_ceo_telegram_config(
    state: State<'_, AppState>,
) -> CommandResult<CeoTelegramConfig> {
    let _user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    read_ceo_telegram_config(&state.db).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn set_ceo_telegram_config(
    state: State<'_, AppState>,
    runtime_enabled: bool,
    telegram_user_id: Option<String>,
    openai_model: String,
    idempotency_key: String,
) -> CommandResult<CeoTelegramConfig> {
    let user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let telegram_user_id = validate_optional_telegram_user_id(telegram_user_id)?;
    let ctx = scoped_admin_ctx(&user, idempotency_key, SET_CEO_TELEGRAM_CONFIG_COMMAND)?;

    set_ceo_telegram_config_idempotent(
        &state.db,
        &ctx,
        runtime_enabled,
        telegram_user_id,
        openai_model,
    )
    .await?;

    read_ceo_telegram_config(&state.db).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn set_ceo_telegram_bot_token(
    state: State<'_, AppState>,
    token: String,
    idempotency_key: String,
) -> CommandResult<()> {
    let user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let ctx = scoped_admin_ctx(
        &user,
        idempotency_key,
        SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
    )?;
    let store = KeychainSecretStore;
    set_ceo_telegram_bot_token_with_store(&state.db, &store, &ctx, &token).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn clear_ceo_telegram_bot_token(
    state: State<'_, AppState>,
    idempotency_key: String,
) -> CommandResult<()> {
    let user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let ctx = scoped_admin_ctx(
        &user,
        idempotency_key,
        SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
    )?;
    let store = KeychainSecretStore;
    clear_ceo_telegram_bot_token_with_store(&state.db, &store, &ctx).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn set_ceo_openai_api_key(
    state: State<'_, AppState>,
    api_key: String,
    idempotency_key: String,
) -> CommandResult<()> {
    let user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let ctx = scoped_admin_ctx(
        &user,
        idempotency_key,
        SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
    )?;
    let store = KeychainSecretStore;
    set_ceo_openai_api_key_with_store(&state.db, &store, &ctx, &api_key).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn clear_ceo_openai_api_key(
    state: State<'_, AppState>,
    idempotency_key: String,
) -> CommandResult<()> {
    let user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let ctx = scoped_admin_ctx(
        &user,
        idempotency_key,
        SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
    )?;
    let store = KeychainSecretStore;
    clear_ceo_openai_api_key_with_store(&state.db, &store, &ctx).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn get_ceo_telegram_gate_status(
    state: State<'_, AppState>,
) -> CommandResult<CeoTelegramGateStatus> {
    let _user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let config = read_ceo_telegram_config(&state.db).await?;
    let cloud_opt_in = read_ceo_cloud_data_opt_in(&state.db).await?;
    Ok(config.evaluate_gate(cloud_opt_in))
}

#[allow(dead_code)]
pub(crate) fn validate_telegram_user_id(value: &str) -> CommandResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|character| character.is_ascii_digit()) {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            "Telegram user ID must contain digits only",
        ));
    }
    Ok(trimmed.to_string())
}

#[allow(dead_code)]
fn validate_optional_telegram_user_id(value: Option<String>) -> CommandResult<Option<String>> {
    value
        .map(|value| validate_telegram_user_id(&value))
        .transpose()
}

#[allow(dead_code)]
fn scoped_admin_ctx(
    user: &User,
    idempotency_key: String,
    command_name: &'static str,
) -> CommandResult<WriteCommandContext> {
    let mut ctx = WriteCommandContext::for_scoped_command(
        uuid::Uuid::new_v4().to_string(),
        idempotency_key,
        command_name,
    )?;
    ctx.actor_id = Some(user.id.clone());
    Ok(ctx)
}

#[allow(dead_code)]
pub(crate) async fn set_ceo_telegram_bot_token_with_store(
    pool: &Pool<Sqlite>,
    store: &dyn AgentSecretStore,
    ctx: &WriteCommandContext,
    token: &str,
) -> CommandResult<()> {
    set_agent_secret_with_store(pool, store, ctx, AgentSecretKind::TelegramBotToken, token).await
}

#[allow(dead_code)]
pub(crate) async fn clear_ceo_telegram_bot_token_with_store(
    pool: &Pool<Sqlite>,
    store: &dyn AgentSecretStore,
    ctx: &WriteCommandContext,
) -> CommandResult<()> {
    clear_agent_secret_with_store(pool, store, ctx, AgentSecretKind::TelegramBotToken).await
}

#[allow(dead_code)]
pub(crate) async fn set_ceo_openai_api_key_with_store(
    pool: &Pool<Sqlite>,
    store: &dyn AgentSecretStore,
    ctx: &WriteCommandContext,
    api_key: &str,
) -> CommandResult<()> {
    set_agent_secret_with_store(pool, store, ctx, AgentSecretKind::OpenAiApiKey, api_key).await
}

#[allow(dead_code)]
pub(crate) async fn clear_ceo_openai_api_key_with_store(
    pool: &Pool<Sqlite>,
    store: &dyn AgentSecretStore,
    ctx: &WriteCommandContext,
) -> CommandResult<()> {
    clear_agent_secret_with_store(pool, store, ctx, AgentSecretKind::OpenAiApiKey).await
}

#[allow(dead_code)]
async fn set_agent_secret_with_store(
    pool: &Pool<Sqlite>,
    store: &dyn AgentSecretStore,
    ctx: &WriteCommandContext,
    kind: AgentSecretKind,
    value: &str,
) -> CommandResult<()> {
    validate_non_blank_secret(value)?;
    let keychain_mutated = Arc::new(AtomicBool::new(false));
    let keychain_mutated_for_guard = Arc::clone(&keychain_mutated);
    let payload = set_secret_idempotency_payload(kind, value);

    if let Err(error) =
        persist_secret_presence_with_guard(pool, ctx, kind, true, payload, || async move {
            store.set_secret(kind, value)?;
            keychain_mutated_for_guard.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await
    {
        if keychain_mutated.load(Ordering::SeqCst) {
            store.clear_secret(kind)?;
        }
        return Err(error);
    }

    Ok(())
}

#[allow(dead_code)]
async fn clear_agent_secret_with_store(
    pool: &Pool<Sqlite>,
    store: &dyn AgentSecretStore,
    ctx: &WriteCommandContext,
    kind: AgentSecretKind,
) -> CommandResult<()> {
    let payload = clear_secret_idempotency_payload(kind);
    persist_secret_presence_with_guard(pool, ctx, kind, false, payload, || async move {
        store.clear_secret(kind)
    })
    .await
}

#[allow(dead_code)]
async fn persist_secret_presence_with_guard<Before, Fut>(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    kind: AgentSecretKind,
    present: bool,
    idempotency_payload: serde_json::Value,
    before_transaction: Before,
) -> CommandResult<()>
where
    Before: FnOnce() -> Fut,
    Fut: std::future::Future<Output = CommandResult<()>> + Send,
{
    let (telegram_present, openai_present) = match kind {
        AgentSecretKind::TelegramBotToken => (Some(present), None),
        AgentSecretKind::OpenAiApiKey => (None, Some(present)),
    };

    update_ceo_telegram_secret_presence_with_guard_idempotent(
        pool,
        ctx,
        telegram_present,
        openai_present,
        idempotency_payload,
        before_transaction,
    )
    .await
    .map(|_| ())
}

fn set_secret_idempotency_payload(kind: AgentSecretKind, value: &str) -> serde_json::Value {
    serde_json::json!({
        "credential": kind.idempotency_label(),
        "operation": "set",
        "fingerprint": agent_secret_fingerprint(kind, value),
    })
}

fn clear_secret_idempotency_payload(kind: AgentSecretKind) -> serde_json::Value {
    serde_json::json!({
        "credential": kind.idempotency_label(),
        "operation": "clear",
    })
}

#[allow(dead_code)]
fn validate_non_blank_secret(value: &str) -> CommandResult<()> {
    if value.trim().is_empty() {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            "Secret value must not be blank",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_error::{codes, AppErrorKind};
    use crate::{
        agent::secrets::{AgentSecretKind, AgentSecretStore},
        command_idempotency::{ActorType, WriteCommandContext},
    };
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    struct CountingSecretStore {
        values: Arc<Mutex<HashMap<AgentSecretKind, String>>>,
        set_count: Arc<Mutex<usize>>,
        clear_count: Arc<Mutex<usize>>,
    }

    impl CountingSecretStore {
        fn set_count(&self) -> usize {
            *self.set_count.lock().expect("set count lock")
        }

        fn clear_count(&self) -> usize {
            *self.clear_count.lock().expect("clear count lock")
        }
    }

    impl AgentSecretStore for CountingSecretStore {
        fn get_secret(&self, kind: AgentSecretKind) -> CommandResult<Option<String>> {
            Ok(self.values.lock().expect("values lock").get(&kind).cloned())
        }

        fn set_secret(&self, kind: AgentSecretKind, value: &str) -> CommandResult<()> {
            *self.set_count.lock().expect("set count lock") += 1;
            self.values
                .lock()
                .expect("values lock")
                .insert(kind, value.to_string());
            Ok(())
        }

        fn clear_secret(&self, kind: AgentSecretKind) -> CommandResult<()> {
            *self.clear_count.lock().expect("clear count lock") += 1;
            self.values.lock().expect("values lock").remove(&kind);
            Ok(())
        }
    }

    fn mock_user(role: &str) -> User {
        User {
            id: "u1".to_string(),
            name: "Test".to_string(),
            role: role.to_string(),
            active: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn admin_user_allowed_by_auth_helper() {
        let user =
            require_ceo_cloud_opt_in_admin(Some(mock_user("admin"))).expect("admin must pass");
        assert_eq!(user.role, "admin");
    }

    #[test]
    fn receptionist_user_rejected() {
        let error = require_ceo_cloud_opt_in_admin(Some(mock_user("receptionist")))
            .expect_err("non-admin must fail");
        assert_eq!(error.code, codes::AUTH_FORBIDDEN);
        assert_eq!(error.kind, AppErrorKind::User);
    }

    #[test]
    fn unauthenticated_user_rejected() {
        let error =
            require_ceo_cloud_opt_in_admin(None).expect_err("missing user must be rejected");
        assert_eq!(error.code, codes::AUTH_NOT_AUTHENTICATED);
        assert_eq!(error.kind, AppErrorKind::User);
    }

    #[test]
    fn numeric_telegram_user_id_validation_accepts_digits() {
        assert!(validate_telegram_user_id("123456789").is_ok());
    }

    #[test]
    fn numeric_telegram_user_id_validation_rejects_username() {
        let error = validate_telegram_user_id("@owner").expect_err("username is not a numeric id");
        assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
    }

    async fn test_pool() -> Pool<Sqlite> {
        let database_url = format!(
            "sqlite://file:{}?mode=memory&cache=shared",
            uuid::Uuid::new_v4()
        );

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("failed to open sqlite test pool");

        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("failed to enable foreign keys");

        crate::db::run_migrations(&pool)
            .await
            .expect("failed to run migrations");

        pool
    }

    fn write_ctx() -> WriteCommandContext {
        write_ctx_with_key("idem-secret-command-1")
    }

    fn write_ctx_with_key(idempotency_key: &str) -> WriteCommandContext {
        let mut ctx = WriteCommandContext::for_internal_test(
            "req-secret-command-1",
            idempotency_key,
            crate::agent::config::SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
        );
        ctx.actor_type = ActorType::Human;
        ctx.actor_id = Some("admin-1".to_string());
        ctx
    }

    #[tokio::test]
    async fn telegram_secret_write_clears_store_when_metadata_persistence_fails() {
        let pool = test_pool().await;
        sqlx::query("DROP TABLE settings")
            .execute(&pool)
            .await
            .expect("drop settings table");
        let store = CountingSecretStore::default();
        let ctx = write_ctx();

        let error = set_ceo_telegram_bot_token_with_store(&pool, &store, &ctx, "telegram-token")
            .await
            .expect_err("closed pool must fail metadata persistence");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(store.set_count(), 1, "secret must be written after claim");
        assert_eq!(
            store.clear_count(),
            1,
            "failed metadata write must roll back secret"
        );
        assert_eq!(
            store
                .get_secret(AgentSecretKind::TelegramBotToken)
                .expect("store remains readable"),
            None
        );
    }

    async fn audit_row_count(pool: &Pool<Sqlite>) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM agent_audit_events")
            .fetch_one(pool)
            .await
            .expect("count audit rows")
    }

    #[tokio::test]
    async fn same_idempotency_key_same_secret_replays_without_rewriting_or_duplicate_audit() {
        let pool = test_pool().await;
        let store = CountingSecretStore::default();
        let ctx = write_ctx_with_key("idem-same-secret");

        set_ceo_telegram_bot_token_with_store(&pool, &store, &ctx, "telegram-token")
            .await
            .expect("first write succeeds");
        set_ceo_telegram_bot_token_with_store(&pool, &store, &ctx, "telegram-token")
            .await
            .expect("same secret replays");

        assert_eq!(store.set_count(), 1, "replay must not rewrite keychain");
        assert_eq!(audit_row_count(&pool).await, 1);
    }

    #[tokio::test]
    async fn same_idempotency_key_different_secret_rejects_before_keychain_mutation() {
        let pool = test_pool().await;
        let store = CountingSecretStore::default();
        let ctx = write_ctx_with_key("idem-different-secret");

        set_ceo_telegram_bot_token_with_store(&pool, &store, &ctx, "telegram-token-a")
            .await
            .expect("first write succeeds");

        let error = set_ceo_telegram_bot_token_with_store(&pool, &store, &ctx, "telegram-token-b")
            .await
            .expect_err("different secret must conflict");

        assert_eq!(error.code, codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH);
        assert_eq!(store.set_count(), 1, "conflict must not rewrite keychain");
        assert_eq!(
            store
                .get_secret(AgentSecretKind::TelegramBotToken)
                .expect("secret remains readable")
                .as_deref(),
            Some("telegram-token-a")
        );
    }

    #[tokio::test]
    async fn same_idempotency_key_set_then_clear_rejects_before_keychain_mutation() {
        let pool = test_pool().await;
        let store = CountingSecretStore::default();
        let ctx = write_ctx_with_key("idem-set-clear");

        set_ceo_telegram_bot_token_with_store(&pool, &store, &ctx, "telegram-token")
            .await
            .expect("first write succeeds");

        let error = clear_ceo_telegram_bot_token_with_store(&pool, &store, &ctx)
            .await
            .expect_err("clear with set key must conflict");

        assert_eq!(error.code, codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH);
        assert_eq!(store.clear_count(), 0, "conflict must not clear keychain");
        assert_eq!(
            store
                .get_secret(AgentSecretKind::TelegramBotToken)
                .expect("secret remains readable")
                .as_deref(),
            Some("telegram-token")
        );
    }
}
