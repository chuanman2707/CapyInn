//! Booking service tests, split by the production module under test.
//!
//! This was one 6,201-line `tests.rs`. The split is mechanical — no test body
//! was edited — and mirrors the production layout so a failure names the module
//! it belongs to. Local fixtures moved with the tests that use them.
//!
//! Shared imports live in `prelude` and are glob-imported by each submodule,
//! which keeps `-D warnings` happy: a glob never reports unused imports, so
//! submodules do not each need a hand-maintained import list.

mod support;

mod prelude {
    pub(crate) use super::support::*;

    pub(crate) use chrono::{Duration, Local, NaiveDate, TimeZone};
    pub(crate) use sqlx::Row;

    pub(crate) use crate::{
        commands::reservations,
        domain::booking::{BookingError, OriginSideEffect},
        models::{
            AddGroupServiceRequest, BackfillStayRequest, CheckOutRequest, CheckoutSettlementMode,
            CheckoutSettlementPreviewRequest, CreateGuestRequest, CreateReservationRequest,
            GroupCheckoutRequest, ModifyReservationRequest,
        },
        money::MAX_TRANSPORT_SAFE_MONEY_VND,
        queries::booking::{audit_queries, billing_queries, revenue_queries},
    };

    pub(crate) use crate::services::booking::{
        audit_service, backfill,
        billing_service::{
            add_folio_line, add_folio_line_idempotent, record_cancellation_fee_tx,
            record_deposit_tx, record_deposit_with_origin_tx, record_payment,
            record_payment_idempotent, record_payment_returning_id_tx, record_payment_tx,
            record_payment_with_origin_tx,
        },
        group_lifecycle, group_service_management, guest_service, pricing_service,
        pricing_service::calculate_stay_price_tx,
        reservation_lifecycle, stay_lifecycle,
    };
}

mod backfill;
mod checkout_settlement;
mod extend_stay;
mod folio;
mod group_idempotency;
mod group_services;
mod groups;
mod guests;
mod money_guards;
mod payments;
mod peak_season_e2e;
mod pricing;
mod reporting;
mod reservation_idempotency;
mod reservations;
mod stay_error_mapping;
mod stays;
