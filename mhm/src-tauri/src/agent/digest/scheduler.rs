use crate::{
    agent::{
        channel::telegram::TelegramTransport,
        digest::{
            config::{get_ceo_digest_config, CeoDigestConfig},
            runtime::CeoDigestRuntime,
            store::{
                claim_due_digest_run, create_digest_run_if_absent, mark_digest_delivered,
                mark_digest_retry_or_failed, NewDigestRun, CEO_DIGEST_MAX_ATTEMPTS,
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
use sqlx::{Pool, Row, Sqlite};
use std::time::Duration as StdDuration;
use tokio::sync::oneshot;

pub const CEO_DIGEST_STARTUP_CATCHUP_AFTER_MINUTES: i64 = 60;
pub const CEO_DIGEST_SCHEDULER_POLL_DELAY: StdDuration = StdDuration::from_secs(30);
pub const CEO_DIGEST_RETRY_DELAY: StdDuration = StdDuration::from_secs(60);

pub async fn ensure_startup_digest_due_run(
    pool: &Pool<Sqlite>,
    config: &CeoDigestConfig,
    now: DateTime<Utc>,
) -> CommandResult<bool> {
    if last_delivery_is_recent(pool, now).await? {
        return Ok(false);
    }

    create_due_run_if_absent_for_hour(
        pool,
        NewDigestRun {
            id: digest_run_id(now),
            channel_actor_id: config.telegram_user_id.clone(),
            delivery_chat_id: config.telegram_delivery_chat_id,
            due_at: now.to_rfc3339(),
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

        match deliver_next_due_digest(&pool, &runtime, &config).await {
            Ok(_) => {}
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
) -> CommandResult<()>
where
    P: AiProvider,
    T: TelegramTransport,
{
    let claim_token = uuid::Uuid::new_v4().to_string();
    let Some(run) = claim_due_digest_run(pool, &Utc::now().to_rfc3339(), &claim_token).await?
    else {
        return Ok(());
    };

    match runtime
        .deliver_digest(&run, config.openai_model.clone())
        .await
    {
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
            .await
        }
        Err(error) => {
            let retry_at = (Utc::now()
                + Duration::seconds(CEO_DIGEST_RETRY_DELAY.as_secs() as i64))
            .to_rfc3339();
            let mark_result = mark_digest_retry_or_failed(
                pool,
                &run.id,
                &run.claim_token,
                &error.code,
                safe_delivery_error_summary(&error),
                Some(retry_at),
            )
            .await;
            if let Err(mark_error) = mark_result {
                error!(
                    "Failed to mark CEO digest retry state: {} {}",
                    mark_error.code, mark_error.message
                );
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
    if last_delivery_is_recent(pool, now).await? {
        return Ok(false);
    }

    let hour = truncate_to_utc_hour(now)?;
    create_due_run_if_absent_for_hour(
        pool,
        NewDigestRun {
            id: hourly_digest_run_id(hour),
            channel_actor_id: config.telegram_user_id.clone(),
            delivery_chat_id: config.telegram_delivery_chat_id,
            due_at: hour.to_rfc3339(),
            max_attempts: CEO_DIGEST_MAX_ATTEMPTS,
        },
        now,
    )
    .await
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

    if digest_run_exists_in_hour(pool, now).await? {
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

async fn last_delivery_is_recent(pool: &Pool<Sqlite>, now: DateTime<Utc>) -> CommandResult<bool> {
    let row = sqlx::query(
        "SELECT delivered_at FROM agent_digest_runs
         WHERE status = 'delivered'
         ORDER BY delivered_at DESC LIMIT 1",
    )
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

async fn digest_run_exists_in_hour(pool: &Pool<Sqlite>, now: DateTime<Utc>) -> CommandResult<bool> {
    let hour = truncate_to_utc_hour(now)?;
    let like_pattern = format!("ceo-digest-{}%", hour.format("%Y%m%dT%H"));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_digest_runs WHERE id LIKE ?")
        .bind(like_pattern)
        .fetch_one(pool)
        .await
        .map_err(system_error)?;
    Ok(count > 0)
}

fn digest_run_id(now: DateTime<Utc>) -> String {
    format!("ceo-digest-{}", now.format("%Y%m%dT%H%M%SZ"))
}

fn hourly_digest_run_id(hour: DateTime<Utc>) -> String {
    format!("ceo-digest-{}", hour.format("%Y%m%dT%H0000Z"))
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
    use crate::agent::config::CeoTelegramConfig;
    use sqlx::sqlite::SqlitePoolOptions;

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
        CeoDigestConfig::from_telegram_config(
            CeoTelegramConfig {
                runtime_enabled: false,
                telegram_user_id: Some("123".to_string()),
                telegram_bot_token_present: true,
                openai_api_key_present: true,
                openai_model: "gpt-5".to_string(),
                last_update_id: None,
            },
            true,
            Some(55),
        )
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
}
