//! Sequences stay pricing: load the inputs, then apply the rules.
//!
//! The split is deliberate — `queries::booking::pricing_queries` owns the
//! reads, `domain::booking::pricing` owns the rules, and this module is the
//! only place that knows both exist.

use sqlx::{Sqlite, Transaction};

use crate::domain::booking::pricing::calculate_from_loaded_inputs;
use crate::domain::booking::BookingResult;
use crate::queries::booking::pricing_queries::load_stay_pricing_inputs_tx;

/// Prices inside the caller's transaction so a lifecycle write can read the
/// rows it just inserted but has not committed.
pub async fn calculate_stay_price_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
) -> BookingResult<crate::pricing::PricingResult> {
    let inputs =
        load_stay_pricing_inputs_tx(tx, room_id, check_in, check_out, pricing_type).await?;
    calculate_from_loaded_inputs(&inputs)
}
