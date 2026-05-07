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
        digest::config::{
            get_ceo_digest_config as read_ceo_digest_config, set_ceo_digest_config_idempotent,
            CeoDigestConfig, CeoDigestGateStatus, SET_CEO_DIGEST_CONFIG_COMMAND,
        },
        secrets::{
            agent_secret_fingerprint, AgentSecretKind, AgentSecretStore, KeychainSecretStore,
        },
        supervisor::{reconcile_managed_supervisor, AgentSupervisor},
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
    supervisor: State<'_, AgentSupervisor>,
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

    set_ceo_cloud_data_opt_in_idempotent(&state.db, &ctx, enabled).await?;
    reconcile_managed_supervisor(&state.db, Some(&*supervisor)).await
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
    supervisor: State<'_, AgentSupervisor>,
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

    reconcile_managed_supervisor(&state.db, Some(&*supervisor)).await?;

    read_ceo_telegram_config(&state.db).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn set_ceo_telegram_bot_token(
    state: State<'_, AppState>,
    supervisor: State<'_, AgentSupervisor>,
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
    set_ceo_telegram_bot_token_with_store(&state.db, &store, &ctx, &token).await?;
    reconcile_managed_supervisor(&state.db, Some(&*supervisor)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn clear_ceo_telegram_bot_token(
    state: State<'_, AppState>,
    supervisor: State<'_, AgentSupervisor>,
    idempotency_key: String,
) -> CommandResult<()> {
    let user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let ctx = scoped_admin_ctx(
        &user,
        idempotency_key,
        SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
    )?;
    let store = KeychainSecretStore;
    clear_ceo_telegram_bot_token_with_store(&state.db, &store, &ctx).await?;
    reconcile_managed_supervisor(&state.db, Some(&*supervisor)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn set_ceo_openai_api_key(
    state: State<'_, AppState>,
    supervisor: State<'_, AgentSupervisor>,
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
    set_ceo_openai_api_key_with_store(&state.db, &store, &ctx, &api_key).await?;
    reconcile_managed_supervisor(&state.db, Some(&*supervisor)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn clear_ceo_openai_api_key(
    state: State<'_, AppState>,
    supervisor: State<'_, AgentSupervisor>,
    idempotency_key: String,
) -> CommandResult<()> {
    let user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let ctx = scoped_admin_ctx(
        &user,
        idempotency_key,
        SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
    )?;
    let store = KeychainSecretStore;
    clear_ceo_openai_api_key_with_store(&state.db, &store, &ctx).await?;
    reconcile_managed_supervisor(&state.db, Some(&*supervisor)).await
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

#[tauri::command]
#[allow(dead_code)]
pub async fn get_ceo_digest_config(state: State<'_, AppState>) -> CommandResult<CeoDigestConfig> {
    let _user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    read_ceo_digest_config(&state.db).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn get_ceo_digest_gate_status(
    state: State<'_, AppState>,
) -> CommandResult<CeoDigestGateStatus> {
    let _user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let config = read_ceo_digest_config(&state.db).await?;
    let cloud_opt_in = read_ceo_cloud_data_opt_in(&state.db).await?;
    Ok(config.evaluate_gate(cloud_opt_in))
}

#[tauri::command]
#[allow(dead_code)]
pub async fn set_ceo_digest_config(
    state: State<'_, AppState>,
    supervisor: State<'_, AgentSupervisor>,
    digest_enabled: bool,
    telegram_delivery_chat_id: Option<i64>,
    idempotency_key: String,
) -> CommandResult<CeoDigestConfig> {
    let user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let ctx = scoped_admin_ctx(&user, idempotency_key, SET_CEO_DIGEST_CONFIG_COMMAND)?;

    set_ceo_digest_config_idempotent(&state.db, &ctx, digest_enabled, telegram_delivery_chat_id)
        .await?;

    reconcile_managed_supervisor(&state.db, Some(&*supervisor)).await?;

    read_ceo_digest_config(&state.db).await
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
    let _secret_lock = acquire_agent_secret_lock(kind).await?;
    let prior_secret = store.get_secret(kind)?;
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
            restore_agent_secret(store, kind, prior_secret)?;
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
    let _secret_lock = acquire_agent_secret_lock(kind).await?;
    let prior_secret = store.get_secret(kind)?;
    let keychain_mutated = Arc::new(AtomicBool::new(false));
    let keychain_mutated_for_guard = Arc::clone(&keychain_mutated);
    let payload = clear_secret_idempotency_payload(kind);
    if let Err(error) =
        persist_secret_presence_with_guard(pool, ctx, kind, false, payload, || async move {
            store.clear_secret(kind)?;
            keychain_mutated_for_guard.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await
    {
        if keychain_mutated.load(Ordering::SeqCst) {
            restore_agent_secret(store, kind, prior_secret)?;
        }
        return Err(error);
    }

    Ok(())
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

async fn acquire_agent_secret_lock(
    kind: AgentSecretKind,
) -> CommandResult<crate::aggregate_locks::AggregateLockGuard> {
    crate::aggregate_locks::global_manager()
        .acquire([format!("agent_secret:{}", kind.idempotency_label())])
        .await
}

fn restore_agent_secret(
    store: &dyn AgentSecretStore,
    kind: AgentSecretKind,
    prior_secret: Option<String>,
) -> CommandResult<()> {
    match prior_secret {
        Some(value) => store.set_secret(kind, &value),
        None => store.clear_secret(kind),
    }
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
        sync::mpsc,
        sync::{Arc, Mutex},
        time::Duration,
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

    struct InterleavingSecretStore {
        values: Arc<Mutex<HashMap<AgentSecretKind, String>>>,
        a_mutated_tx: Mutex<Option<mpsc::Sender<()>>>,
        a_release_rx: Mutex<Option<mpsc::Receiver<()>>>,
        b_mutated_tx: Mutex<Option<mpsc::Sender<()>>>,
    }

    impl InterleavingSecretStore {
        fn new(
            a_mutated_tx: mpsc::Sender<()>,
            a_release_rx: mpsc::Receiver<()>,
            b_mutated_tx: mpsc::Sender<()>,
        ) -> Self {
            Self {
                values: Arc::new(Mutex::new(HashMap::new())),
                a_mutated_tx: Mutex::new(Some(a_mutated_tx)),
                a_release_rx: Mutex::new(Some(a_release_rx)),
                b_mutated_tx: Mutex::new(Some(b_mutated_tx)),
            }
        }
    }

    impl AgentSecretStore for InterleavingSecretStore {
        fn get_secret(&self, kind: AgentSecretKind) -> CommandResult<Option<String>> {
            Ok(self.values.lock().expect("values lock").get(&kind).cloned())
        }

        fn set_secret(&self, kind: AgentSecretKind, value: &str) -> CommandResult<()> {
            self.values
                .lock()
                .expect("values lock")
                .insert(kind, value.to_string());

            if value == "new-a" {
                if let Some(tx) = self.a_mutated_tx.lock().expect("a signal lock").take() {
                    let _ = tx.send(());
                }
                if let Some(rx) = self.a_release_rx.lock().expect("a release lock").take() {
                    rx.recv_timeout(Duration::from_secs(2))
                        .expect("test releases command A");
                }
            }

            if value == "new-b" {
                if let Some(tx) = self.b_mutated_tx.lock().expect("b signal lock").take() {
                    let _ = tx.send(());
                }
            }

            Ok(())
        }

        fn clear_secret(&self, kind: AgentSecretKind) -> CommandResult<()> {
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
        write_ctx_with_key_and_actor(idempotency_key, "admin-1")
    }

    fn write_ctx_with_key_and_actor(idempotency_key: &str, actor_id: &str) -> WriteCommandContext {
        let mut ctx = WriteCommandContext::for_internal_test(
            "req-secret-command-1",
            idempotency_key,
            crate::agent::config::SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
        );
        ctx.actor_type = ActorType::Human;
        ctx.actor_id = Some(actor_id.to_string());
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

    #[tokio::test]
    async fn replacing_existing_secret_restores_old_secret_when_metadata_persistence_fails() {
        let pool = test_pool().await;
        sqlx::query("DROP TABLE settings")
            .execute(&pool)
            .await
            .expect("drop settings table");
        let store = CountingSecretStore::default();
        store
            .set_secret(AgentSecretKind::TelegramBotToken, "old-token")
            .expect("seed old secret");
        let ctx = write_ctx_with_key("idem-replace-fails");

        let error = set_ceo_telegram_bot_token_with_store(&pool, &store, &ctx, "new-token")
            .await
            .expect_err("metadata write must fail");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(
            store
                .get_secret(AgentSecretKind::TelegramBotToken)
                .expect("secret remains readable")
                .as_deref(),
            Some("old-token")
        );
    }

    #[tokio::test]
    async fn clearing_existing_secret_restores_old_secret_when_metadata_persistence_fails() {
        let pool = test_pool().await;
        sqlx::query("DROP TABLE settings")
            .execute(&pool)
            .await
            .expect("drop settings table");
        let store = CountingSecretStore::default();
        store
            .set_secret(AgentSecretKind::TelegramBotToken, "old-token")
            .expect("seed old secret");
        let ctx = write_ctx_with_key("idem-clear-fails");

        let error = clear_ceo_telegram_bot_token_with_store(&pool, &store, &ctx)
            .await
            .expect_err("metadata write must fail");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(
            store
                .get_secret(AgentSecretKind::TelegramBotToken)
                .expect("secret remains readable")
                .as_deref(),
            Some("old-token")
        );
    }

    #[tokio::test]
    async fn setting_new_secret_leaves_no_secret_when_metadata_persistence_fails() {
        let pool = test_pool().await;
        sqlx::query("DROP TABLE settings")
            .execute(&pool)
            .await
            .expect("drop settings table");
        let store = CountingSecretStore::default();
        let ctx = write_ctx_with_key("idem-new-fails");

        let error = set_ceo_telegram_bot_token_with_store(&pool, &store, &ctx, "new-token")
            .await
            .expect_err("metadata write must fail");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(
            store
                .get_secret(AgentSecretKind::TelegramBotToken)
                .expect("store remains readable"),
            None
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_secret_replacement_does_not_clobber_later_successful_secret_write() {
        let pool = test_pool().await;
        sqlx::query(
            "CREATE TRIGGER fail_agent_audit_for_actor_a
             BEFORE INSERT ON agent_audit_events
             WHEN NEW.actor_id = 'admin-a'
             BEGIN
                 SELECT RAISE(ABORT, 'fail actor a audit');
             END",
        )
        .execute(&pool)
        .await
        .expect("create failure trigger");

        let (a_mutated_tx, a_mutated_rx) = mpsc::channel();
        let (a_release_tx, a_release_rx) = mpsc::channel();
        let (b_mutated_tx, b_mutated_rx) = mpsc::channel();
        let store = Arc::new(InterleavingSecretStore::new(
            a_mutated_tx,
            a_release_rx,
            b_mutated_tx,
        ));
        store
            .set_secret(AgentSecretKind::TelegramBotToken, "old-token")
            .expect("seed old secret");

        let pool_a = pool.clone();
        let store_a = Arc::clone(&store);
        let command_a = tokio::spawn(async move {
            let ctx = write_ctx_with_key_and_actor("idem-a", "admin-a");
            set_ceo_telegram_bot_token_with_store(&pool_a, store_a.as_ref(), &ctx, "new-a").await
        });

        a_mutated_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("command A mutates keychain");

        let pool_b = pool.clone();
        let store_b = Arc::clone(&store);
        let command_b = tokio::spawn(async move {
            let ctx = write_ctx_with_key_and_actor("idem-b", "admin-b");
            set_ceo_telegram_bot_token_with_store(&pool_b, store_b.as_ref(), &ctx, "new-b").await
        });

        let _ = b_mutated_rx.recv_timeout(Duration::from_millis(100));
        a_release_tx.send(()).expect("release command A");

        let error_a = command_a
            .await
            .expect("command A task joins")
            .expect_err("command A metadata write fails");
        assert_eq!(error_a.code, codes::SYSTEM_INTERNAL_ERROR);
        command_b
            .await
            .expect("command B task joins")
            .expect("command B succeeds");

        assert_eq!(
            store
                .get_secret(AgentSecretKind::TelegramBotToken)
                .expect("secret remains readable")
                .as_deref(),
            Some("new-b")
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
