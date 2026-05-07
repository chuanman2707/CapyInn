use crate::{
    agent::{
        channel::telegram::TelegramTransport,
        digest::{
            config::{get_ceo_digest_config, CeoDigestConfig},
            runtime::CeoDigestRuntime,
            store::{
                claim_due_digest_run_for_target, create_digest_run_if_absent,
                mark_digest_delivered, mark_digest_retry_or_failed, NewDigestRun,
                CEO_DIGEST_MAX_ATTEMPTS, DIGEST_STATUS_FAILED, DIGEST_STATUS_IN_PROGRESS,
                DIGEST_STATUS_RETRY_WAITING,
            },
        },
        provider::openai::AiProvider,
        settings::get_ceo_cloud_data_opt_in,
    },
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::system_error,
};
use chrono::{DateTime, Duration, Timelike, Utc};
use log::{error, info};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite};
use std::time::Duration as StdDuration;
use tokio::sync::oneshot;

pub const CEO_DIGEST_STARTUP_CATCHUP_AFTER_MINUTES: i64 = 60;
pub const CEO_DIGEST_SCHEDULER_POLL_DELAY: StdDuration = StdDuration::from_secs(30);
pub const CEO_DIGEST_RETRY_DELAY: StdDuration = StdDuration::from_secs(60);
pub const CEO_DIGEST_IN_PROGRESS_RECOVERY_AFTER_SECONDS: i64 = 60;

enum DigestSchedulerStep {
    Continue,
    Shutdown,
    ShutdownCleanupFailed(CommandError),
}

pub async fn ensure_startup_digest_due_run(
    pool: &Pool<Sqlite>,
    config: &CeoDigestConfig,
    now: DateTime<Utc>,
) -> CommandResult<bool> {
    if last_delivery_is_recent(pool, config, now).await? {
        return Ok(false);
    }
    let due_at = truncate_to_utc_second(now)?;

    create_due_run_if_absent_for_hour(
        pool,
        NewDigestRun {
            id: digest_run_id(
                due_at,
                &config.telegram_user_id,
                config.telegram_delivery_chat_id,
            ),
            channel_actor_id: config.telegram_user_id.clone(),
            delivery_chat_id: config.telegram_delivery_chat_id,
            due_at: due_at.to_rfc3339(),
            max_attempts: CEO_DIGEST_MAX_ATTEMPTS,
        },
        now,
    )
    .await
}

pub async fn run_ceo_digest_scheduler<P, T>(
    pool: Pool<Sqlite>,
    runtime: CeoDigestRuntime<P, T>,
    mut shutdown_rx: oneshot::Receiver<()>,
) where
    P: AiProvider + 'static,
    T: TelegramTransport + 'static,
{
    info!("CEO digest scheduler started");
    let mut startup_due_checked = false;

    loop {
        let config = match get_ceo_digest_config(&pool).await {
            Ok(config) => config,
            Err(error) => {
                error!(
                    "Failed to read CEO digest config: {} {}",
                    error.code, error.message
                );
                if wait_for_scheduler_delay_or_shutdown(&mut shutdown_rx, CEO_DIGEST_RETRY_DELAY)
                    .await
                {
                    break;
                }
                continue;
            }
        };
        let cloud_opt_in = match get_ceo_cloud_data_opt_in(&pool).await {
            Ok(value) => value,
            Err(error) => {
                error!(
                    "Failed to read CEO cloud data opt-in for digest scheduler: {} {}",
                    error.code, error.message
                );
                if wait_for_scheduler_delay_or_shutdown(&mut shutdown_rx, CEO_DIGEST_RETRY_DELAY)
                    .await
                {
                    break;
                }
                continue;
            }
        };

        if !config.evaluate_gate(cloud_opt_in).ready {
            info!("CEO digest scheduler stopping because gate is no longer ready");
            break;
        }

        let now = Utc::now();
        if let Err(error) = recover_stale_in_progress_digest_runs(&pool, now).await {
            error!(
                "Failed to recover stale CEO digest claims: {} {}",
                error.code, error.message
            );
            if wait_for_scheduler_delay_or_shutdown(&mut shutdown_rx, CEO_DIGEST_RETRY_DELAY).await
            {
                break;
            }
            continue;
        }

        if !startup_due_checked {
            match ensure_startup_digest_due_run(&pool, &config, now).await {
                Ok(_) => startup_due_checked = true,
                Err(error) => {
                    error!(
                        "Failed to ensure CEO startup digest run: {} {}",
                        error.code, error.message
                    );
                    if wait_for_scheduler_delay_or_shutdown(
                        &mut shutdown_rx,
                        CEO_DIGEST_RETRY_DELAY,
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                }
            }
        }

        if let Err(error) = ensure_hourly_digest_due_run(&pool, &config, now).await {
            error!(
                "Failed to ensure CEO hourly digest run: {} {}",
                error.code, error.message
            );
            if wait_for_scheduler_delay_or_shutdown(&mut shutdown_rx, CEO_DIGEST_RETRY_DELAY).await
            {
                break;
            }
            continue;
        }

        match deliver_next_due_digest(&pool, &runtime, &config, &mut shutdown_rx).await {
            Ok(DigestSchedulerStep::Continue) => {}
            Ok(DigestSchedulerStep::Shutdown) => break,
            Ok(DigestSchedulerStep::ShutdownCleanupFailed(error)) => {
                error!(
                    "CEO digest shutdown cleanup failed: {} {}",
                    error.code, error.message
                );
                break;
            }
            Err(error) => {
                error!(
                    "CEO digest delivery failed: {} {}",
                    error.code, error.message
                );
                if wait_for_scheduler_delay_or_shutdown(&mut shutdown_rx, CEO_DIGEST_RETRY_DELAY)
                    .await
                {
                    break;
                }
                continue;
            }
        }

        if wait_for_scheduler_delay_or_shutdown(&mut shutdown_rx, CEO_DIGEST_SCHEDULER_POLL_DELAY)
            .await
        {
            break;
        }
    }

    info!("CEO digest scheduler stopped");
}

async fn deliver_next_due_digest<P, T>(
    pool: &Pool<Sqlite>,
    runtime: &CeoDigestRuntime<P, T>,
    config: &CeoDigestConfig,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> CommandResult<DigestSchedulerStep>
where
    P: AiProvider,
    T: TelegramTransport,
{
    let claim_token = uuid::Uuid::new_v4().to_string();
    let Some(run) = claim_due_digest_run_for_target(
        pool,
        &Utc::now().to_rfc3339(),
        &claim_token,
        config.telegram_user_id.as_deref(),
        config.telegram_delivery_chat_id,
    )
    .await?
    else {
        return Ok(DigestSchedulerStep::Continue);
    };

    let delivery_result = tokio::select! {
        _ = shutdown_rx => {
            if let Err(mark_error) = mark_digest_retry_or_failed(
                pool,
                &run.id,
                &run.claim_token,
                codes::AGENT_RUNTIME_DISABLED,
                safe_shutdown_error_summary(),
                Some(next_retry_at()),
            )
            .await
            {
                error!(
                    "Failed to mark CEO digest shutdown retry state: {} {}",
                    mark_error.code, mark_error.message
                );
                return Ok(DigestSchedulerStep::ShutdownCleanupFailed(mark_error));
            }
            return Ok(DigestSchedulerStep::Shutdown);
        }
        result = runtime.deliver_digest(&run, config.openai_model.clone()) => result,
    };

    match delivery_result {
        Ok(result) => {
            mark_digest_delivered(
                pool,
                &run.id,
                &run.claim_token,
                json!({
                    "reply_char_count": result.reply_chars,
                    "unavailable_tools": result.unavailable_tools,
                }),
            )
            .await?;
            Ok(DigestSchedulerStep::Continue)
        }
        Err(error) => {
            let mark_result = mark_digest_retry_or_failed(
                pool,
                &run.id,
                &run.claim_token,
                &error.code,
                safe_delivery_error_summary(&error),
                Some(next_retry_at()),
            )
            .await;
            if let Err(mark_error) = mark_result {
                error!(
                    "Failed to mark CEO digest retry state: {} {}",
                    mark_error.code, mark_error.message
                );
                return Err(mark_error);
            }
            Err(error)
        }
    }
}

async fn ensure_hourly_digest_due_run(
    pool: &Pool<Sqlite>,
    config: &CeoDigestConfig,
    now: DateTime<Utc>,
) -> CommandResult<bool> {
    if last_delivery_is_recent(pool, config, now).await? {
        return Ok(false);
    }

    let hour = truncate_to_utc_hour(now)?;
    create_due_run_if_absent_for_hour(
        pool,
        NewDigestRun {
            id: hourly_digest_run_id(
                hour,
                &config.telegram_user_id,
                config.telegram_delivery_chat_id,
            ),
            channel_actor_id: config.telegram_user_id.clone(),
            delivery_chat_id: config.telegram_delivery_chat_id,
            due_at: hour.to_rfc3339(),
            max_attempts: CEO_DIGEST_MAX_ATTEMPTS,
        },
        now,
    )
    .await
}

async fn recover_stale_in_progress_digest_runs(
    pool: &Pool<Sqlite>,
    now: DateTime<Utc>,
) -> CommandResult<u64> {
    let stale_before =
        (now - Duration::seconds(CEO_DIGEST_IN_PROGRESS_RECOVERY_AFTER_SECONDS)).to_rfc3339();
    let now = now.to_rfc3339();
    let summary = serde_json::to_string(&safe_recovery_error_summary()).map_err(system_error)?;
    let send_started_summary =
        serde_json::to_string(&safe_recovery_after_send_started_error_summary())
            .map_err(system_error)?;
    let result = sqlx::query(
        "UPDATE agent_digest_runs
         SET status = CASE
                 WHEN json_extract(delivery_summary_json, '$.telegram_send_started') = 1 THEN ?
                 WHEN attempt_count >= max_attempts THEN ?
                 ELSE ?
             END,
             next_retry_at = CASE
                 WHEN json_extract(delivery_summary_json, '$.telegram_send_started') = 1 THEN NULL
                 WHEN attempt_count >= max_attempts THEN NULL
                 ELSE ?
             END,
             claimed_at = NULL,
             claim_token = NULL,
             last_error_code = ?,
             last_error_summary_json = CASE
                 WHEN json_extract(delivery_summary_json, '$.telegram_send_started') = 1 THEN ?
                 ELSE ?
             END,
             updated_at = ?
         WHERE status = ?
           AND claimed_at IS NOT NULL
           AND claimed_at <= ?",
    )
    .bind(DIGEST_STATUS_FAILED)
    .bind(DIGEST_STATUS_FAILED)
    .bind(DIGEST_STATUS_RETRY_WAITING)
    .bind(&now)
    .bind(codes::AGENT_RUNTIME_DISABLED)
    .bind(send_started_summary)
    .bind(summary)
    .bind(&now)
    .bind(DIGEST_STATUS_IN_PROGRESS)
    .bind(stale_before)
    .execute(pool)
    .await
    .map_err(system_error)?;

    Ok(result.rows_affected())
}

async fn create_due_run_if_absent_for_hour(
    pool: &Pool<Sqlite>,
    input: NewDigestRun,
    now: DateTime<Utc>,
) -> CommandResult<bool> {
    if let Some(matches) = existing_digest_run_matches(pool, &input).await? {
        if matches {
            return Ok(false);
        }
        create_digest_run_if_absent(pool, input).await?;
        return Ok(true);
    }

    if digest_run_exists_in_hour(pool, now, &input).await? {
        return Ok(false);
    }

    create_digest_run_if_absent(pool, input).await?;
    Ok(true)
}

async fn existing_digest_run_matches(
    pool: &Pool<Sqlite>,
    input: &NewDigestRun,
) -> CommandResult<Option<bool>> {
    let row = sqlx::query(
        "SELECT channel_actor_id, delivery_chat_id, due_at, max_attempts
         FROM agent_digest_runs
         WHERE id = ?",
    )
    .bind(&input.id)
    .fetch_optional(pool)
    .await
    .map_err(system_error)?;

    let Some(row) = row else {
        return Ok(None);
    };

    let delivery_chat_id = parse_optional_delivery_chat_id(row.get("delivery_chat_id"))?;
    Ok(Some(
        row.get::<Option<String>, _>("channel_actor_id") == input.channel_actor_id
            && delivery_chat_id == input.delivery_chat_id
            && row.get::<String, _>("due_at") == input.due_at
            && row.get::<i64, _>("max_attempts") == input.max_attempts,
    ))
}

async fn last_delivery_is_recent(
    pool: &Pool<Sqlite>,
    config: &CeoDigestConfig,
    now: DateTime<Utc>,
) -> CommandResult<bool> {
    let delivery_chat_id = config
        .telegram_delivery_chat_id
        .map(|value| value.to_string());
    let row = sqlx::query(
        "SELECT delivered_at FROM agent_digest_runs
         WHERE status = 'delivered'
           AND ((? IS NULL AND channel_actor_id IS NULL) OR channel_actor_id = ?)
           AND ((? IS NULL AND delivery_chat_id IS NULL) OR delivery_chat_id = ?)
         ORDER BY delivered_at DESC LIMIT 1",
    )
    .bind(&config.telegram_user_id)
    .bind(&config.telegram_user_id)
    .bind(&delivery_chat_id)
    .bind(&delivery_chat_id)
    .fetch_optional(pool)
    .await
    .map_err(system_error)?;

    let Some(row) = row else {
        return Ok(false);
    };
    let Some(delivered_at) = row.get::<Option<String>, _>("delivered_at") else {
        return Ok(false);
    };
    let delivered_at = parse_utc_timestamp(&delivered_at)?;

    Ok(now.signed_duration_since(delivered_at)
        < Duration::minutes(CEO_DIGEST_STARTUP_CATCHUP_AFTER_MINUTES))
}

async fn digest_run_exists_in_hour(
    pool: &Pool<Sqlite>,
    now: DateTime<Utc>,
    input: &NewDigestRun,
) -> CommandResult<bool> {
    let hour = truncate_to_utc_hour(now)?;
    let like_pattern = format!("ceo-digest-{}%", hour.format("%Y%m%dT%H"));
    let delivery_chat_id = input.delivery_chat_id.map(|value| value.to_string());
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_digest_runs
         WHERE id LIKE ?
           AND ((? IS NULL AND channel_actor_id IS NULL) OR channel_actor_id = ?)
           AND ((? IS NULL AND delivery_chat_id IS NULL) OR delivery_chat_id = ?)",
    )
    .bind(like_pattern)
    .bind(&input.channel_actor_id)
    .bind(&input.channel_actor_id)
    .bind(&delivery_chat_id)
    .bind(&delivery_chat_id)
    .fetch_one(pool)
    .await
    .map_err(system_error)?;
    Ok(count > 0)
}

fn digest_run_id(
    now: DateTime<Utc>,
    channel_actor_id: &Option<String>,
    delivery_chat_id: Option<i64>,
) -> String {
    format!(
        "ceo-digest-{}-{}",
        now.format("%Y%m%dT%H%M%SZ"),
        delivery_target_fingerprint(channel_actor_id, delivery_chat_id)
    )
}

fn hourly_digest_run_id(
    hour: DateTime<Utc>,
    channel_actor_id: &Option<String>,
    delivery_chat_id: Option<i64>,
) -> String {
    format!(
        "ceo-digest-{}-{}",
        hour.format("%Y%m%dT%H0000Z"),
        delivery_target_fingerprint(channel_actor_id, delivery_chat_id)
    )
}

fn delivery_target_fingerprint(
    channel_actor_id: &Option<String>,
    delivery_chat_id: Option<i64>,
) -> String {
    let actor = channel_actor_id.as_deref().unwrap_or("");
    let chat_id = delivery_chat_id
        .map(|value| value.to_string())
        .unwrap_or_default();
    let digest = Sha256::digest(format!("actor={actor}\nchat={chat_id}").as_bytes());
    format!("{digest:x}").chars().take(16).collect()
}

fn truncate_to_utc_hour(now: DateTime<Utc>) -> CommandResult<DateTime<Utc>> {
    now.with_minute(0)
        .and_then(|value| value.with_second(0))
        .and_then(|value| value.with_nanosecond(0))
        .ok_or_else(|| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                "Cannot calculate digest scheduler hour.",
            )
        })
}

fn truncate_to_utc_second(now: DateTime<Utc>) -> CommandResult<DateTime<Utc>> {
    now.with_nanosecond(0).ok_or_else(|| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            "Cannot calculate digest scheduler second.",
        )
    })
}

fn parse_utc_timestamp(value: &str) -> CommandResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(system_error)
}

fn parse_optional_delivery_chat_id(value: Option<String>) -> CommandResult<Option<i64>> {
    value
        .map(|value| value.parse::<i64>().map_err(system_error))
        .transpose()
}

fn safe_delivery_error_summary(error: &CommandError) -> serde_json::Value {
    json!({
        "code": error.code,
        "message": "CEO digest delivery failed. Retry may be attempted.",
    })
}

fn safe_shutdown_error_summary() -> serde_json::Value {
    json!({
        "code": codes::AGENT_RUNTIME_DISABLED,
        "message": "CEO digest scheduler shut down before delivery completed.",
    })
}

fn safe_recovery_error_summary() -> serde_json::Value {
    json!({
        "code": codes::AGENT_RUNTIME_DISABLED,
        "message": "CEO digest scheduler recovered an abandoned in-progress run.",
    })
}

fn safe_recovery_after_send_started_error_summary() -> serde_json::Value {
    json!({
        "code": codes::AGENT_RUNTIME_DISABLED,
        "message": "CEO digest scheduler recovered an abandoned run after Telegram send started; retry suppressed to avoid duplicate delivery.",
        "retry_suppressed": true,
    })
}

fn next_retry_at() -> String {
    (Utc::now() + Duration::seconds(CEO_DIGEST_RETRY_DELAY.as_secs() as i64)).to_rfc3339()
}

async fn wait_for_scheduler_delay_or_shutdown(
    shutdown_rx: &mut oneshot::Receiver<()>,
    delay: StdDuration,
) -> bool {
    tokio::select! {
        _ = shutdown_rx => true,
        _ = tokio::time::sleep(delay) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{
        channel::telegram::TelegramUpdate,
        config::{
            CeoTelegramConfig, CEO_OPENAI_KEY_PRESENT_SETTING, CEO_TELEGRAM_OPENAI_MODEL_SETTING,
            CEO_TELEGRAM_TOKEN_PRESENT_SETTING, CEO_TELEGRAM_USER_ID_SETTING,
        },
        digest::config::{
            CEO_HOURLY_DIGEST_ENABLED_SETTING, CEO_TELEGRAM_DELIVERY_CHAT_ID_SETTING,
        },
        digest::store::claim_due_digest_run,
        provider::openai::{ProviderRequest, ProviderTurn},
        settings::CEO_CLOUD_DATA_OPT_IN_SETTING,
    };
    use sqlx::{sqlite::SqlitePoolOptions, Row};
    use std::{
        future::{pending, Future},
        pin::Pin,
        sync::{Arc, Mutex},
    };
    use tokio::sync::oneshot;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        crate::db::run_migrations(&pool).await.expect("migrations");
        pool
    }

    fn digest_config() -> CeoDigestConfig {
        digest_config_for_target("123", 55)
    }

    fn digest_config_for_target(
        telegram_user_id: &str,
        telegram_delivery_chat_id: i64,
    ) -> CeoDigestConfig {
        CeoDigestConfig::from_telegram_config(
            CeoTelegramConfig {
                runtime_enabled: false,
                telegram_user_id: Some(telegram_user_id.to_string()),
                telegram_bot_token_present: true,
                openai_api_key_present: true,
                openai_model: "gpt-5".to_string(),
                last_update_id: None,
            },
            true,
            Some(telegram_delivery_chat_id),
        )
    }

    async fn save_test_setting(pool: &Pool<Sqlite>, key: &str, value: &str) {
        crate::services::settings_store::save_setting(pool, key, value)
            .await
            .expect("save setting");
    }

    async fn configure_ready_digest_gate(pool: &Pool<Sqlite>) {
        save_test_setting(pool, CEO_CLOUD_DATA_OPT_IN_SETTING, "true").await;
        save_test_setting(pool, CEO_HOURLY_DIGEST_ENABLED_SETTING, "true").await;
        save_test_setting(pool, CEO_TELEGRAM_USER_ID_SETTING, "123").await;
        save_test_setting(pool, CEO_TELEGRAM_DELIVERY_CHAT_ID_SETTING, "55").await;
        save_test_setting(pool, CEO_TELEGRAM_TOKEN_PRESENT_SETTING, "true").await;
        save_test_setting(pool, CEO_OPENAI_KEY_PRESENT_SETTING, "true").await;
        save_test_setting(pool, CEO_TELEGRAM_OPENAI_MODEL_SETTING, "gpt-test").await;
    }

    async fn seed_claimed_digest_run(pool: &Pool<Sqlite>, id: &str, max_attempts: i64) {
        create_digest_run_if_absent(
            pool,
            NewDigestRun {
                id: id.to_string(),
                channel_actor_id: Some("123".to_string()),
                delivery_chat_id: Some(55),
                due_at: "2026-05-07T01:00:00Z".to_string(),
                max_attempts,
            },
        )
        .await
        .expect("create digest run");
        claim_due_digest_run(pool, "2026-05-07T01:00:01Z", &format!("claim-{id}"))
            .await
            .expect("claim")
            .expect("claimed");
    }

    async fn seed_delivered_digest_run(
        pool: &Pool<Sqlite>,
        id: &str,
        telegram_user_id: &str,
        telegram_delivery_chat_id: i64,
        due_at: &str,
        delivered_at: &str,
    ) {
        create_digest_run_if_absent(
            pool,
            NewDigestRun {
                id: id.to_string(),
                channel_actor_id: Some(telegram_user_id.to_string()),
                delivery_chat_id: Some(telegram_delivery_chat_id),
                due_at: due_at.to_string(),
                max_attempts: CEO_DIGEST_MAX_ATTEMPTS,
            },
        )
        .await
        .expect("create delivered digest run");
        sqlx::query(
            "UPDATE agent_digest_runs
             SET status = 'delivered', delivered_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(delivered_at)
        .bind(delivered_at)
        .bind(id)
        .execute(pool)
        .await
        .expect("mark delivered digest run");
    }

    async fn make_claim_stale(pool: &Pool<Sqlite>, id: &str) {
        sqlx::query("UPDATE agent_digest_runs SET claimed_at = ? WHERE id = ?")
            .bind("2026-05-07T00:00:00Z")
            .bind(id)
            .execute(pool)
            .await
            .expect("make claim stale");
    }

    async fn digest_run_state(pool: &Pool<Sqlite>, id: &str) -> sqlx::sqlite::SqliteRow {
        sqlx::query(
            "SELECT status, next_retry_at, last_error_code, last_error_summary_json
             FROM agent_digest_runs
             WHERE id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("digest run row")
    }

    struct StaticProvider;

    impl AiProvider for StaticProvider {
        fn create_turn<'a>(
            &'a self,
            _request: ProviderRequest,
        ) -> Pin<Box<dyn Future<Output = CommandResult<ProviderTurn>> + Send + 'a>> {
            Box::pin(async { Ok(ProviderTurn::FinalText("Digest test".to_string())) })
        }
    }

    #[derive(Clone)]
    struct BlockingTelegram {
        send_started_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    }

    impl TelegramTransport for BlockingTelegram {
        fn get_updates<'a>(
            &'a self,
            _offset: Option<i64>,
        ) -> Pin<Box<dyn Future<Output = CommandResult<Vec<TelegramUpdate>>> + Send + 'a>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn send_message<'a>(
            &'a self,
            _chat_id: i64,
            _text: String,
        ) -> Pin<Box<dyn Future<Output = CommandResult<()>> + Send + 'a>> {
            Box::pin(async move {
                let send_started_tx = self
                    .send_started_tx
                    .lock()
                    .expect("send started lock")
                    .take();
                if let Some(send_started_tx) = send_started_tx {
                    let _ = send_started_tx.send(());
                }
                pending::<CommandResult<()>>().await
            })
        }
    }

    #[tokio::test]
    async fn startup_creates_one_digest_when_last_delivery_is_old() {
        let pool = test_pool().await;
        let now = "2026-05-07T02:10:00Z"
            .parse::<DateTime<Utc>>()
            .expect("now");

        let created = ensure_startup_digest_due_run(&pool, &digest_config(), now)
            .await
            .expect("ensure due");
        assert!(created);

        let second = ensure_startup_digest_due_run(&pool, &digest_config(), now)
            .await
            .expect("ensure due again");
        assert!(!second);

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_digest_runs")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn startup_and_hourly_due_run_are_idempotent_at_top_of_hour_with_subseconds() {
        let pool = test_pool().await;
        let now = "2026-05-07T02:00:00.123Z"
            .parse::<DateTime<Utc>>()
            .expect("now");
        let config = digest_config();

        let startup_created = ensure_startup_digest_due_run(&pool, &config, now)
            .await
            .expect("startup due");
        assert!(startup_created);

        let hourly_created = ensure_hourly_digest_due_run(&pool, &config, now)
            .await
            .expect("hourly due");
        assert!(!hourly_created);

        let row = sqlx::query("SELECT id, due_at FROM agent_digest_runs")
            .fetch_one(&pool)
            .await
            .expect("digest row");
        let id = row.get::<String, _>("id");
        assert!(
            id.starts_with("ceo-digest-20260507T020000Z-"),
            "digest id keeps hour prefix"
        );
        assert!(!id.contains("55"), "digest id must not expose raw chat id");
        assert_eq!(row.get::<String, _>("due_at"), "2026-05-07T02:00:00+00:00");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_digest_runs")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn current_target_hourly_run_is_created_when_stale_target_exists_for_hour() {
        let pool = test_pool().await;
        let now = "2026-05-07T02:10:00Z"
            .parse::<DateTime<Utc>>()
            .expect("now");

        let old_created =
            ensure_hourly_digest_due_run(&pool, &digest_config_for_target("123", 55), now)
                .await
                .expect("old target due run");
        assert!(old_created);

        let current_created =
            ensure_hourly_digest_due_run(&pool, &digest_config_for_target("123", 66), now)
                .await
                .expect("current target due run");
        assert!(current_created);

        let rows = sqlx::query(
            "SELECT delivery_chat_id
             FROM agent_digest_runs
             ORDER BY delivery_chat_id ASC",
        )
        .fetch_all(&pool)
        .await
        .expect("digest rows");
        let chat_ids = rows
            .iter()
            .map(|row| row.get::<Option<String>, _>("delivery_chat_id"))
            .collect::<Vec<_>>();
        assert_eq!(
            chat_ids,
            vec![Some("55".to_string()), Some("66".to_string())]
        );
    }

    #[tokio::test]
    async fn startup_due_run_ignores_recent_delivery_for_stale_target() {
        let pool = test_pool().await;
        let now = "2026-05-07T02:10:00Z"
            .parse::<DateTime<Utc>>()
            .expect("now");
        seed_delivered_digest_run(
            &pool,
            "old-delivered-target",
            "123",
            55,
            "2026-05-07T01:00:00Z",
            "2026-05-07T01:55:00Z",
        )
        .await;

        let created =
            ensure_startup_digest_due_run(&pool, &digest_config_for_target("123", 66), now)
                .await
                .expect("startup due run");
        assert!(created);
    }

    #[tokio::test]
    async fn hourly_due_run_ignores_recent_delivery_for_stale_target() {
        let pool = test_pool().await;
        let now = "2026-05-07T02:10:00Z"
            .parse::<DateTime<Utc>>()
            .expect("now");
        seed_delivered_digest_run(
            &pool,
            "old-delivered-target",
            "123",
            55,
            "2026-05-07T01:00:00Z",
            "2026-05-07T01:55:00Z",
        )
        .await;

        let created =
            ensure_hourly_digest_due_run(&pool, &digest_config_for_target("123", 66), now)
                .await
                .expect("hourly due run");
        assert!(created);
    }

    #[tokio::test]
    async fn scheduler_shutdown_exits_while_delivery_is_blocked() {
        let pool = test_pool().await;
        configure_ready_digest_gate(&pool).await;
        let assertion_pool = pool.clone();
        let (send_started_tx, send_started_rx) = oneshot::channel();
        let telegram = BlockingTelegram {
            send_started_tx: Arc::new(Mutex::new(Some(send_started_tx))),
        };
        let runtime = CeoDigestRuntime::new(pool.clone(), StaticProvider, telegram);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let scheduler = tokio::spawn(run_ceo_digest_scheduler(pool, runtime, shutdown_rx));

        tokio::time::timeout(StdDuration::from_secs(5), send_started_rx)
            .await
            .expect("delivery starts")
            .expect("delivery signal is sent");
        shutdown_tx.send(()).expect("send shutdown");

        tokio::time::timeout(StdDuration::from_millis(200), scheduler)
            .await
            .expect("scheduler exits promptly")
            .expect("scheduler task succeeds");

        let row = sqlx::query(
            "SELECT status, next_retry_at, last_error_code, last_error_summary_json
             FROM agent_digest_runs
             LIMIT 1",
        )
        .fetch_one(&assertion_pool)
        .await
        .expect("digest run row");
        assert_eq!(row.get::<String, _>("status"), "failed");
        assert_eq!(row.get::<Option<String>, _>("next_retry_at"), None);
        assert_eq!(
            row.get::<Option<String>, _>("last_error_code"),
            Some(codes::AGENT_RUNTIME_DISABLED.to_string())
        );

        let summary: serde_json::Value =
            serde_json::from_str(&row.get::<String, _>("last_error_summary_json"))
                .expect("safe summary json");
        assert_eq!(summary["code"], codes::AGENT_RUNTIME_DISABLED);
        assert_eq!(summary["retry_suppressed"], true);
    }

    #[tokio::test]
    async fn stale_in_progress_digest_run_recovers_to_retry_waiting_and_can_be_claimed() {
        let pool = test_pool().await;
        let now = "2026-05-07T02:00:00Z"
            .parse::<DateTime<Utc>>()
            .expect("now");
        seed_claimed_digest_run(&pool, "digest-stale", CEO_DIGEST_MAX_ATTEMPTS).await;
        make_claim_stale(&pool, "digest-stale").await;

        let recovered = recover_stale_in_progress_digest_runs(&pool, now)
            .await
            .expect("recover stale claims");

        assert_eq!(recovered, 1);
        let row = digest_run_state(&pool, "digest-stale").await;
        assert_eq!(row.get::<String, _>("status"), "retry_waiting");
        assert!(
            row.get::<Option<String>, _>("next_retry_at").is_some(),
            "recovered retry must be claimable"
        );
        assert_eq!(
            row.get::<Option<String>, _>("last_error_code"),
            Some(codes::AGENT_RUNTIME_DISABLED.to_string())
        );
        let summary: serde_json::Value =
            serde_json::from_str(&row.get::<String, _>("last_error_summary_json"))
                .expect("safe summary");
        assert_eq!(summary["code"], codes::AGENT_RUNTIME_DISABLED);
        assert_eq!(
            summary["message"],
            "CEO digest scheduler recovered an abandoned in-progress run."
        );

        let claim = claim_due_digest_run(&pool, &now.to_rfc3339(), "claim-recovered")
            .await
            .expect("claim recovered");
        assert!(claim.is_some(), "recovered retry should be claimable");
    }

    #[tokio::test]
    async fn stale_in_progress_after_telegram_send_started_is_not_retried() {
        let pool = test_pool().await;
        let now = "2026-05-07T02:00:00Z"
            .parse::<DateTime<Utc>>()
            .expect("now");
        seed_claimed_digest_run(&pool, "digest-send-started", CEO_DIGEST_MAX_ATTEMPTS).await;
        sqlx::query(
            "UPDATE agent_digest_runs
             SET claimed_at = ?,
                 delivery_summary_json = ?
             WHERE id = ?",
        )
        .bind("2026-05-07T00:00:00Z")
        .bind(
            serde_json::json!({
                "telegram_send_started": true,
                "reply_char_count": 12,
                "unavailable_tools": [],
            })
            .to_string(),
        )
        .bind("digest-send-started")
        .execute(&pool)
        .await
        .expect("mark send started");

        let recovered = recover_stale_in_progress_digest_runs(&pool, now)
            .await
            .expect("recover stale send-started claim");

        assert_eq!(recovered, 1);
        let row = digest_run_state(&pool, "digest-send-started").await;
        assert_eq!(row.get::<String, _>("status"), "failed");
        assert_eq!(row.get::<Option<String>, _>("next_retry_at"), None);
        assert_eq!(
            row.get::<Option<String>, _>("last_error_code"),
            Some(codes::AGENT_RUNTIME_DISABLED.to_string())
        );
        let summary: serde_json::Value =
            serde_json::from_str(&row.get::<String, _>("last_error_summary_json"))
                .expect("safe summary");
        assert_eq!(summary["code"], codes::AGENT_RUNTIME_DISABLED);
        assert_eq!(summary["retry_suppressed"], true);
    }

    #[tokio::test]
    async fn stale_in_progress_digest_run_recovers_to_failed_when_attempts_are_exhausted() {
        let pool = test_pool().await;
        let now = "2026-05-07T02:00:00Z"
            .parse::<DateTime<Utc>>()
            .expect("now");
        seed_claimed_digest_run(&pool, "digest-exhausted", 1).await;
        make_claim_stale(&pool, "digest-exhausted").await;

        let recovered = recover_stale_in_progress_digest_runs(&pool, now)
            .await
            .expect("recover stale exhausted claim");

        assert_eq!(recovered, 1);
        let row = digest_run_state(&pool, "digest-exhausted").await;
        assert_eq!(row.get::<String, _>("status"), "failed");
        assert_eq!(row.get::<Option<String>, _>("next_retry_at"), None);
        assert_eq!(
            row.get::<Option<String>, _>("last_error_code"),
            Some(codes::AGENT_RUNTIME_DISABLED.to_string())
        );
        let summary: serde_json::Value =
            serde_json::from_str(&row.get::<String, _>("last_error_summary_json"))
                .expect("safe summary");
        assert_eq!(summary["code"], codes::AGENT_RUNTIME_DISABLED);
        assert_eq!(
            summary["message"],
            "CEO digest scheduler recovered an abandoned in-progress run."
        );
    }
}
