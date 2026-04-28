use crate::app_error::{codes, CommandError, CommandResult};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};
use tokio::sync::{Mutex, OwnedMutexGuard};

#[derive(Clone, Default)]
pub struct AggregateLockManager {
    inner: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

pub struct AggregateLockGuard {
    keys: Vec<String>,
    _guards: Vec<OwnedMutexGuard<()>>,
}

impl AggregateLockGuard {
    pub fn keys(&self) -> &[String] {
        &self.keys
    }
}

pub fn room_key(room_id: &str) -> CommandResult<String> {
    aggregate_key("room", room_id)
}

pub fn booking_key(booking_id: &str) -> CommandResult<String> {
    aggregate_key("booking", booking_id)
}

pub fn folio_key(booking_id: &str) -> CommandResult<String> {
    aggregate_key("folio", booking_id)
}

pub fn group_key(group_id: &str) -> CommandResult<String> {
    aggregate_key("group", group_id)
}

fn aggregate_key(prefix: &str, id: &str) -> CommandResult<String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(CommandError::user(
            codes::CONFLICT_INVALID_STATE_TRANSITION,
            "Missing aggregate lock key",
        ));
    }
    Ok(format!("{prefix}:{trimmed}"))
}

pub fn canonicalize_lock_keys<I, S>(keys: I) -> CommandResult<Vec<String>>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut keys = keys
        .into_iter()
        .map(Into::into)
        .map(|key| key.trim().to_string())
        .collect::<Vec<_>>();

    if keys.is_empty() || keys.iter().any(|key| key.is_empty()) {
        return Err(CommandError::user(
            codes::CONFLICT_INVALID_STATE_TRANSITION,
            "Missing aggregate lock key",
        ));
    }

    keys.sort();
    keys.dedup();
    Ok(keys)
}

impl AggregateLockManager {
    pub async fn acquire<I, S>(&self, keys: I) -> CommandResult<AggregateLockGuard>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let keys = canonicalize_lock_keys(keys)?;
        let locks = {
            let mut map = self.inner.lock().await;
            keys.iter()
                .map(|key| {
                    map.entry(key.clone())
                        .or_insert_with(|| Arc::new(Mutex::new(())))
                        .clone()
                })
                .collect::<Vec<_>>()
        };

        let mut guards = Vec::with_capacity(locks.len());
        for lock in locks {
            guards.push(lock.lock_owned().await);
        }

        Ok(AggregateLockGuard {
            keys,
            _guards: guards,
        })
    }
}

static GLOBAL_MANAGER: OnceLock<AggregateLockManager> = OnceLock::new();

pub fn global_manager() -> &'static AggregateLockManager {
    GLOBAL_MANAGER.get_or_init(AggregateLockManager::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn canonicalize_lock_keys_sorts_and_deduplicates() {
        let keys = canonicalize_lock_keys(vec![
            "room:R2".to_string(),
            "booking:B1".to_string(),
            "room:R2".to_string(),
            "room:R1".to_string(),
        ])
        .expect("keys canonicalize");

        assert_eq!(keys, vec!["booking:B1", "room:R1", "room:R2"]);
    }

    #[tokio::test]
    async fn canonicalize_lock_keys_rejects_empty_input() {
        let err = canonicalize_lock_keys(Vec::<String>::new()).expect_err("empty keys reject");

        assert_eq!(err.code, codes::CONFLICT_INVALID_STATE_TRANSITION);
        assert_eq!(err.message, "Missing aggregate lock key");
    }

    #[tokio::test]
    async fn same_key_waits_for_first_guard_to_drop() {
        let manager = AggregateLockManager::default();
        let first = manager.acquire(["room:R1"]).await.expect("first lock");

        let second_manager = manager.clone();
        let second = tokio::spawn(async move {
            second_manager.acquire(["room:R1"]).await.expect("second lock")
        });

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert!(!second.is_finished());

        drop(first);
        let _second_guard = second.await.expect("second task joins");
    }

    #[tokio::test]
    async fn unrelated_keys_do_not_wait_for_each_other() {
        let manager = AggregateLockManager::default();
        let first = manager.acquire(["room:R1"]).await.expect("first lock");
        let (acquired_tx, acquired_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();

        let second_manager = manager.clone();
        let second = tokio::spawn(async move {
            let second_guard = second_manager.acquire(["room:R2"]).await.expect("second lock");
            acquired_tx
                .send(second_guard.keys().to_vec())
                .expect("main task waits for acquired signal");
            release_rx.await.expect("release signal received");
            drop(second_guard);
        });

        assert_eq!(
            acquired_rx.await.expect("second key acquired"),
            vec!["room:R2".to_string()]
        );

        drop(first);
        release_tx.send(()).expect("second task waits for release");
        second.await.expect("second task joins");
    }

    #[tokio::test]
    async fn reversed_multi_key_inputs_are_canonicalized() {
        let first = canonicalize_lock_keys(vec!["room:R2".to_string(), "room:R1".to_string()])
            .expect("first order");
        let second = canonicalize_lock_keys(vec!["room:R1".to_string(), "room:R2".to_string()])
            .expect("second order");

        assert_eq!(first, second);
    }
}
