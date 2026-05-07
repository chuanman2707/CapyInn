use crate::{
    agent::{
        channel::telegram::{
            poll_once, HttpTelegramTransport, TelegramMessageRuntime, TelegramRuntimeMessage,
            TelegramTransport,
        },
        config::{
            get_ceo_telegram_config, set_ceo_telegram_last_update_id_idempotent,
            CeoTelegramGateStatus,
        },
        digest::config::SET_CEO_TELEGRAM_DELIVERY_CHAT_ID_COMMAND,
        provider::openai::{AiProvider, OpenAiProvider},
        runtime::ceo_chat::{CeoChatMessage, CeoChatRuntime},
        secrets::KeychainSecretStore,
        settings::get_ceo_cloud_data_opt_in,
    },
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{ActorType, WriteCommandContext},
};
use log::{error, info};
use sqlx::{Pool, Sqlite};
use std::{future::Future, pin::Pin, sync::Mutex, time::Duration};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

const SUPERVISOR_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const SUPERVISOR_RETRY_DELAY: Duration = Duration::from_secs(5);
const CEO_TELEGRAM_OFFSET_COMMAND: &str = "agent.set_ceo_telegram_last_update_id";
const CEO_TELEGRAM_RUNTIME_ACTOR: &str = "ceo_telegram_runtime";
const CEO_TELEGRAM_DELIVERY_CHAT_IDEMPOTENCY_KEY: &str = "ceo-telegram-delivery-chat";
const CEO_TELEGRAM_DELIVERY_CHAT_ACTOR: &str = "ceo_telegram_runtime";

pub struct AgentSupervisor {
    running: Mutex<Option<JoinHandle<()>>>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    mode: SupervisorMode,
}

enum SupervisorMode {
    Polling {
        pool: Pool<Sqlite>,
    },
    #[cfg(test)]
    TestIdle,
}

impl AgentSupervisor {
    pub fn new(pool: Pool<Sqlite>) -> Self {
        Self {
            running: Mutex::new(None),
            shutdown_tx: Mutex::new(None),
            mode: SupervisorMode::Polling { pool },
        }
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self {
            running: Mutex::new(None),
            shutdown_tx: Mutex::new(None),
            mode: SupervisorMode::TestIdle,
        }
    }

    pub async fn reconcile(&self, gate: CeoTelegramGateStatus) -> CommandResult<()> {
        if gate.ready {
            self.start_if_needed()
        } else {
            self.shutdown().await;
            Ok(())
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|handle| !handle.is_finished()))
            .unwrap_or(false)
    }

    pub async fn shutdown(&self) {
        let task = self.running.lock().ok().and_then(|mut guard| guard.take());
        let shutdown_tx = self
            .shutdown_tx
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());

        if let Some(shutdown_tx) = shutdown_tx {
            let _ = shutdown_tx.send(());
        }

        if let Some(mut task) = task {
            if tokio::time::timeout(SUPERVISOR_SHUTDOWN_TIMEOUT, &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = tokio::time::timeout(SUPERVISOR_SHUTDOWN_TIMEOUT, task).await;
            }
        }
    }

    fn start_if_needed(&self) -> CommandResult<()> {
        let mut running = self.running.lock().map_err(|_| supervisor_lock_error())?;
        if running
            .as_ref()
            .map(|handle| !handle.is_finished())
            .unwrap_or(false)
        {
            return Ok(());
        }
        let mut shutdown_tx_guard = self
            .shutdown_tx
            .lock()
            .map_err(|_| supervisor_lock_error())?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = self.spawn_runtime_task(shutdown_rx)?;
        *running = Some(task);
        *shutdown_tx_guard = Some(shutdown_tx);
        Ok(())
    }

    fn spawn_runtime_task(
        &self,
        shutdown_rx: oneshot::Receiver<()>,
    ) -> CommandResult<JoinHandle<()>> {
        match &self.mode {
            SupervisorMode::Polling { pool } => {
                let transport = HttpTelegramTransport::new(KeychainSecretStore)?;
                let provider = OpenAiProvider::new(KeychainSecretStore)?;
                let runtime = SupervisorTelegramRuntime {
                    pool: pool.clone(),
                    chat_runtime: CeoChatRuntime::new(pool.clone(), provider),
                };
                Ok(tokio::spawn(run_ceo_telegram_runtime(
                    pool.clone(),
                    transport,
                    runtime,
                    shutdown_rx,
                )))
            }
            #[cfg(test)]
            SupervisorMode::TestIdle => Ok(tokio::spawn(async move {
                let _ = shutdown_rx.await;
            })),
        }
    }

    #[cfg(test)]
    fn running_task_debug_id(&self) -> Option<String> {
        self.running
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|handle| format!("{:?}", handle.id())))
    }
}

#[cfg(test)]
impl Default for AgentSupervisor {
    fn default() -> Self {
        Self::new_for_test()
    }
}

pub async fn reconcile_managed_supervisor(
    pool: &Pool<Sqlite>,
    supervisor: Option<&AgentSupervisor>,
) -> CommandResult<()> {
    let Some(supervisor) = supervisor else {
        return Ok(());
    };

    if crate::runtime_config::env_flag("CAPYINN_DISABLE_CEO_TELEGRAM") {
        supervisor.shutdown().await;
        return Ok(());
    }

    let config = get_ceo_telegram_config(pool).await?;
    let cloud_opt_in = get_ceo_cloud_data_opt_in(pool).await?;
    supervisor
        .reconcile(config.evaluate_gate(cloud_opt_in))
        .await
}

struct SupervisorTelegramRuntime<P> {
    pool: Pool<Sqlite>,
    chat_runtime: CeoChatRuntime<P>,
}

impl<P> TelegramMessageRuntime for SupervisorTelegramRuntime<P>
where
    P: AiProvider,
{
    fn handle_message<'a>(
        &'a self,
        message: TelegramRuntimeMessage,
    ) -> Pin<Box<dyn Future<Output = CommandResult<String>> + Send + 'a>> {
        Box::pin(async move {
            let config = get_ceo_telegram_config(&self.pool).await?;
            let cloud_opt_in = get_ceo_cloud_data_opt_in(&self.pool).await?;
            if let Err(error) = persist_delivery_chat_id(&self.pool, message.chat_id).await {
                error!(
                    "Failed to persist CEO Telegram delivery chat id: {} {}",
                    error.code, error.message
                );
            }
            let reply = self
                .chat_runtime
                .handle_message(CeoChatMessage {
                    actor: message.actor,
                    chat_id: message.chat_id,
                    text: message.text,
                    config,
                    ceo_cloud_data_opt_in: cloud_opt_in,
                })
                .await?;
            Ok(reply.text)
        })
    }
}

async fn run_ceo_telegram_runtime<T, R>(
    pool: Pool<Sqlite>,
    transport: T,
    runtime: R,
    mut shutdown_rx: oneshot::Receiver<()>,
) where
    T: TelegramTransport,
    R: TelegramMessageRuntime,
{
    info!("CEO Telegram runtime started");

    loop {
        let config = match get_ceo_telegram_config(&pool).await {
            Ok(config) => config,
            Err(error) => {
                error!(
                    "Failed to read CEO Telegram config: {} {}",
                    error.code, error.message
                );
                if wait_for_retry_or_shutdown(&mut shutdown_rx).await {
                    break;
                }
                continue;
            }
        };
        let cloud_opt_in = match get_ceo_cloud_data_opt_in(&pool).await {
            Ok(value) => value,
            Err(error) => {
                error!(
                    "Failed to read CEO cloud data opt-in: {} {}",
                    error.code, error.message
                );
                if wait_for_retry_or_shutdown(&mut shutdown_rx).await {
                    break;
                }
                continue;
            }
        };

        if !config.evaluate_gate(cloud_opt_in).ready {
            info!("CEO Telegram runtime stopping because gate is no longer ready");
            break;
        }

        let poll_result = tokio::select! {
            _ = &mut shutdown_rx => break,
            result = poll_once(&transport, &runtime, &config) => result,
        };

        match poll_result {
            Ok(Some(last_update_id)) => {
                if let Err(error) = persist_last_update_id(&pool, last_update_id).await {
                    error!(
                        "Failed to persist CEO Telegram update offset: {} {}",
                        error.code, error.message
                    );
                    if wait_for_retry_or_shutdown(&mut shutdown_rx).await {
                        break;
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                error!(
                    "CEO Telegram polling failed: {} {}",
                    error.code, error.message
                );
                if wait_for_retry_or_shutdown(&mut shutdown_rx).await {
                    break;
                }
            }
        }
    }

    info!("CEO Telegram runtime stopped");
}

async fn wait_for_retry_or_shutdown(shutdown_rx: &mut oneshot::Receiver<()>) -> bool {
    tokio::select! {
        _ = shutdown_rx => true,
        _ = tokio::time::sleep(SUPERVISOR_RETRY_DELAY) => false,
    }
}

async fn persist_last_update_id(pool: &Pool<Sqlite>, last_update_id: i64) -> CommandResult<()> {
    let mut ctx = WriteCommandContext::for_scoped_command(
        uuid::Uuid::new_v4().to_string(),
        format!("ceo-telegram-offset-{last_update_id}"),
        CEO_TELEGRAM_OFFSET_COMMAND,
    )?;
    ctx.actor_type = ActorType::System;
    ctx.actor_id = Some(CEO_TELEGRAM_RUNTIME_ACTOR.to_string());

    set_ceo_telegram_last_update_id_idempotent(pool, &ctx, Some(last_update_id))
        .await
        .map(|_| ())
}

async fn persist_delivery_chat_id(pool: &Pool<Sqlite>, chat_id: i64) -> CommandResult<()> {
    let mut ctx = WriteCommandContext::for_scoped_command(
        uuid::Uuid::new_v4().to_string(),
        CEO_TELEGRAM_DELIVERY_CHAT_IDEMPOTENCY_KEY,
        SET_CEO_TELEGRAM_DELIVERY_CHAT_ID_COMMAND,
    )?;
    ctx.actor_type = ActorType::System;
    ctx.actor_id = Some(CEO_TELEGRAM_DELIVERY_CHAT_ACTOR.to_string());

    crate::agent::digest::config::set_ceo_telegram_delivery_chat_id_idempotent(pool, &ctx, chat_id)
        .await
        .map(|_| ())
}

#[cfg(test)]
async fn persist_delivery_chat_id_for_test(pool: &Pool<Sqlite>, chat_id: i64) -> CommandResult<()> {
    persist_delivery_chat_id(pool, chat_id).await
}

fn supervisor_lock_error() -> CommandError {
    CommandError::system(
        codes::SYSTEM_INTERNAL_ERROR,
        "Agent supervisor lock is unavailable",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::CeoTelegramGateStatus;

    fn ready_gate() -> CeoTelegramGateStatus {
        CeoTelegramGateStatus {
            ready: true,
            missing: Vec::new(),
        }
    }

    fn revoked_gate() -> CeoTelegramGateStatus {
        CeoTelegramGateStatus {
            ready: false,
            missing: Vec::new(),
        }
    }

    #[tokio::test]
    async fn supervisor_starts_only_when_gate_ready() {
        let supervisor = AgentSupervisor::new_for_test();

        supervisor
            .reconcile(revoked_gate())
            .await
            .expect("not-ready gate reconciles");
        assert!(!supervisor.is_running());

        supervisor
            .reconcile(ready_gate())
            .await
            .expect("ready gate reconciles");
        assert!(supervisor.is_running());

        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn supervisor_stops_when_gate_is_revoked() {
        let supervisor = AgentSupervisor::new_for_test();

        supervisor
            .reconcile(ready_gate())
            .await
            .expect("ready gate reconciles");
        assert!(supervisor.is_running());

        supervisor
            .reconcile(revoked_gate())
            .await
            .expect("revoked gate reconciles");
        assert!(!supervisor.is_running());
    }

    #[tokio::test]
    async fn supervisor_reconcile_ready_is_idempotent() {
        let supervisor = AgentSupervisor::new_for_test();

        supervisor
            .reconcile(ready_gate())
            .await
            .expect("initial ready gate reconciles");
        let first_task_id = supervisor.running_task_debug_id();

        supervisor
            .reconcile(ready_gate())
            .await
            .expect("second ready gate reconciles");

        assert!(supervisor.is_running());
        assert_eq!(supervisor.running_task_debug_id(), first_task_id);

        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn paired_chat_persists_delivery_chat_id_for_digest() {
        use crate::agent::digest::config::{
            get_ceo_digest_config, CEO_TELEGRAM_DELIVERY_CHAT_ID_SETTING,
        };
        use sqlx::sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        crate::db::run_migrations(&pool).await.expect("migrations");

        persist_delivery_chat_id_for_test(&pool, 55)
            .await
            .expect("persist chat id");

        let config = get_ceo_digest_config(&pool)
            .await
            .expect("read digest config");
        assert_eq!(config.telegram_delivery_chat_id, Some(55));

        let raw: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(CEO_TELEGRAM_DELIVERY_CHAT_ID_SETTING)
            .fetch_one(&pool)
            .await
            .expect("read raw setting");
        assert_eq!(raw, "55");
    }

    #[tokio::test]
    async fn paired_chat_delivery_command_metadata_excludes_raw_chat_id() {
        use crate::agent::digest::config::SET_CEO_TELEGRAM_DELIVERY_CHAT_ID_COMMAND;
        use sqlx::sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        crate::db::run_migrations(&pool).await.expect("migrations");

        persist_delivery_chat_id_for_test(&pool, 55)
            .await
            .expect("persist chat id");

        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        >(
            "SELECT idempotency_key, intent_json, response_json,
                    result_summary_json, error_summary_json
             FROM command_idempotency
             WHERE command_name = ?
             ORDER BY id ASC",
        )
        .bind(SET_CEO_TELEGRAM_DELIVERY_CHAT_ID_COMMAND)
        .fetch_all(&pool)
        .await
        .expect("read command metadata");

        assert_eq!(rows.len(), 1);
        for (index, row) in rows.iter().enumerate() {
            let fields = [
                ("idempotency_key", Some(row.0.as_str())),
                ("intent_json", Some(row.1.as_str())),
                ("response_json", row.2.as_deref()),
                ("result_summary_json", row.3.as_deref()),
                ("error_summary_json", row.4.as_deref()),
            ];

            for (field_name, value) in fields {
                if let Some(value) = value {
                    assert!(
                        !value.contains("55"),
                        "{field_name} leaked raw chat id in command metadata row {index}: {value}"
                    );
                }
            }
        }
    }
}
