pub mod audit_service;
pub mod backfill;
pub mod billing_service;
pub mod group_lifecycle;
pub mod group_service_management;
pub mod guest_service;
pub mod invoice_generation;
pub mod pricing_service;
pub mod reservation_lifecycle;
pub mod room_change;
pub mod stay_lifecycle;
pub mod support;
pub mod void_lifecycle;

#[cfg(test)]
mod tests;
