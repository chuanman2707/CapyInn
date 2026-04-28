use crate::domain::booking::{BookingError, BookingResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginSideEffect {
    idempotency_key: String,
    ordinal: i64,
}

impl OriginSideEffect {
    #[allow(dead_code)]
    pub fn new(idempotency_key: impl Into<String>, ordinal: i64) -> BookingResult<Self> {
        let idempotency_key = idempotency_key.into();
        if idempotency_key.trim().is_empty() {
            return Err(BookingError::validation(
                "Origin idempotency key is required",
            ));
        }
        if ordinal < 0 {
            return Err(BookingError::validation(
                "Origin ordinal must be greater than or equal to zero",
            ));
        }
        Ok(Self {
            idempotency_key,
            ordinal,
        })
    }

    pub fn key(&self) -> &str {
        &self.idempotency_key
    }

    pub fn ordinal(&self) -> i64 {
        self.ordinal
    }
}
