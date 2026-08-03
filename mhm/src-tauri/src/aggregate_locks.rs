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

#[derive(Debug)]
pub struct AggregateLockGuard {
    keys: Vec<String>,
    _guards: Vec<OwnedMutexGuard<()>>,
}

impl AggregateLockGuard {
    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    fn max_rank(&self) -> u8 {
        self.keys
            .iter()
            .map(|key| scope_rank(key))
            .max()
            .unwrap_or(0)
    }

    /// Lấy pha kế tiếp trong khi vẫn đang cầm pha này.
    ///
    /// Ăn `self` và trả về một guard đã gộp, nên người gọi không thể cầm hai
    /// pha thành hai biến rời rồi thả sai thứ tự, và `keys()` luôn trả về hợp
    /// của mọi pha — đúng cái `refresh_claim_lock_keys` cần ghi lại.
    ///
    /// Pha sau phải có hạng **cao hơn hẳn** mọi khoá đang cầm. Đó là bất biến
    /// chống kẹt: mọi chuỗi lấy khoá trong tiến trình đi theo `(hạng, key)`
    /// tăng nghiêm ngặt.
    pub async fn acquire_next<I, S>(
        self,
        manager: &AggregateLockManager,
        keys: I,
    ) -> CommandResult<AggregateLockGuard>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let next_keys = canonicalize_lock_keys(keys)?;
        let held_rank = self.max_rank();
        let next_rank = next_keys
            .iter()
            .map(|key| scope_rank(key))
            .min()
            .unwrap_or(0);

        if next_rank <= held_rank {
            return Err(CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!(
                    "Lock phase out of order: holding {:?}, requested {:?}",
                    self.keys, next_keys
                ),
            ));
        }

        let next = manager.acquire(next_keys).await?;
        let AggregateLockGuard {
            keys: next_keys,
            _guards: next_guards,
        } = next;

        let mut keys = self.keys;
        keys.extend(next_keys);
        let mut guards = self._guards;
        guards.extend(next_guards);

        Ok(AggregateLockGuard {
            keys,
            _guards: guards,
        })
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

/// Thứ tự lấy khoá đi theo HẠNG, không theo alphabet.
///
/// Một lệnh nhiều pha lấy khoá theo hạng tăng nghiêm ngặt, còn một lệnh
/// một-phát lấy cả bộ đã canonicalize — hai đường đó chỉ không kẹt nhau khi
/// cùng đi theo một thứ tự toàn cục. Alphabet không làm được: `booking:` đứng
/// trước `group:`, nên `remove_group_service` (lấy một phát group + booking +
/// folio) và một `group_checkout` hai pha (group trước, booking sau) tạo thành
/// một chu trình chờ nhau có thật.
fn scope_rank(key: &str) -> u8 {
    match key.split(':').next().unwrap_or_default() {
        "group" => 0,
        "booking" => 1,
        "folio" => 2,
        "room" => 3,
        _ => 9,
    }
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

    keys.sort_by(|left, right| {
        scope_rank(left)
            .cmp(&scope_rank(right))
            .then_with(|| left.cmp(right))
    });
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
    async fn mixed_group_booking_room_and_folio_keys_are_canonicalized() {
        let left = canonicalize_lock_keys(vec![
            group_key("G1").unwrap(),
            booking_key("B2").unwrap(),
            room_key("R2").unwrap(),
            folio_key("B2").unwrap(),
            booking_key("B1").unwrap(),
            room_key("R1").unwrap(),
            folio_key("B1").unwrap(),
            group_key("G1").unwrap(),
        ])
        .unwrap();
        let right = canonicalize_lock_keys(vec![
            folio_key("B1").unwrap(),
            room_key("R1").unwrap(),
            booking_key("B1").unwrap(),
            folio_key("B2").unwrap(),
            room_key("R2").unwrap(),
            booking_key("B2").unwrap(),
            group_key("G1").unwrap(),
        ])
        .unwrap();

        assert_eq!(left, right);
        assert_eq!(
            left,
            vec![
                "group:G1".to_string(),
                "booking:B1".to_string(),
                "booking:B2".to_string(),
                "folio:B1".to_string(),
                "folio:B2".to_string(),
                "room:R1".to_string(),
                "room:R2".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn canonicalize_lock_keys_orders_unknown_prefixes_last() {
        let keys = canonicalize_lock_keys(vec![
            "settings:ceo_cloud_data_opt_in".to_string(),
            "room:R1".to_string(),
            "group:G1".to_string(),
            "agent_secret:telegram".to_string(),
        ])
        .expect("keys canonicalize");

        assert_eq!(
            keys,
            vec![
                "group:G1".to_string(),
                "room:R1".to_string(),
                "agent_secret:telegram".to_string(),
                "settings:ceo_cloud_data_opt_in".to_string(),
            ]
        );
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
            second_manager
                .acquire(["room:R1"])
                .await
                .expect("second lock")
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
            let second_guard = second_manager
                .acquire(["room:R2"])
                .await
                .expect("second lock");
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

    #[tokio::test]
    async fn acquire_next_merges_keys_of_every_phase() {
        let manager = AggregateLockManager::default();
        let guard = manager
            .acquire([booking_key("B1").unwrap(), folio_key("B1").unwrap()])
            .await
            .expect("phase one");
        let guard = guard
            .acquire_next(&manager, [room_key("R1").unwrap()])
            .await
            .expect("phase two");

        assert_eq!(
            guard.keys(),
            vec![
                "booking:B1".to_string(),
                "folio:B1".to_string(),
                "room:R1".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn acquire_next_rejects_a_phase_that_does_not_rank_higher() {
        let manager = AggregateLockManager::default();
        let guard = manager
            .acquire([room_key("R1").unwrap()])
            .await
            .expect("phase one");

        let error = guard
            .acquire_next(&manager, [booking_key("B1").unwrap()])
            .await
            .expect_err("pha sau phải có hạng cao hơn hẳn");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn acquire_next_rejects_a_phase_of_the_same_rank() {
        let manager = AggregateLockManager::default();
        let guard = manager
            .acquire([booking_key("B1").unwrap()])
            .await
            .expect("phase one");

        let error = guard
            .acquire_next(&manager, [booking_key("B2").unwrap()])
            .await
            .expect_err("cùng hạng cũng phải bị chặn");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
    }

    #[tokio::test]
    async fn a_phased_holder_blocks_a_one_shot_acquirer() {
        let manager = AggregateLockManager::default();
        let guard = manager
            .acquire([booking_key("B1").unwrap()])
            .await
            .expect("phase one");
        let guard = guard
            .acquire_next(&manager, [room_key("R1").unwrap()])
            .await
            .expect("phase two");

        let contender_manager = manager.clone();
        let contender = tokio::spawn(async move {
            contender_manager
                .acquire([booking_key("B1").unwrap(), room_key("R1").unwrap()])
                .await
                .expect("contender lock")
        });

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert!(!contender.is_finished());

        drop(guard);
        let _contender_guard = contender.await.expect("contender joins");
    }
}
