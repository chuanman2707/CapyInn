//! Contract tests for the `BookingError` -> `CommandError` mappers that the
//! stay lifecycle actually uses.
//!
//! These assertions were previously written against `commands::rooms::map_stay_error`,
//! a duplicate of this logic that was `#[allow(dead_code)]` and reachable from
//! nothing but its own tests. The mapping had moved into `stay_lifecycle`, so the
//! error contract was only ever verified against code that never ran. Deleting
//! the duplicate and pointing the tests at the live mappers is the point.
//!
//! One behaviour differs from the retired duplicate and is pinned below rather
//! than quietly changed: a zero-guest check-in reports `BOOKING_INVALID_STATE`,
//! not `BOOKING_GUEST_REQUIRED`. No live path emits `BOOKING_GUEST_REQUIRED`.

use super::prelude::*;

use crate::app_error::AppErrorKind;
use crate::services::booking::stay_lifecycle::{
    map_check_in_command_error, map_check_out_command_error, map_extend_stay_command_error,
};

#[test]
fn check_in_maps_missing_room_to_room_not_found() {
    let error = map_check_in_command_error(BookingError::not_found("Không tìm thấy phòng R101"));

    assert_eq!(error.code, "ROOM_NOT_FOUND");
    assert_eq!(error.message, "Không tìm thấy phòng R101");
    assert_eq!(error.kind, AppErrorKind::User);
    assert!(error.support_id.is_none());
}

#[test]
fn check_in_maps_other_missing_rows_to_booking_not_found() {
    let error = map_check_in_command_error(BookingError::not_found("Không tìm thấy booking B1"));

    assert_eq!(error.code, "BOOKING_NOT_FOUND");
    assert_eq!(error.kind, AppErrorKind::User);
}

/// Pins the divergence from the retired duplicate — see the module docs.
#[test]
fn check_in_maps_the_zero_guest_validation_to_invalid_state_not_guest_required() {
    let error = map_check_in_command_error(BookingError::validation("Phải có ít nhất 1 khách"));

    assert_eq!(error.code, "BOOKING_INVALID_STATE");
    assert_eq!(error.message, "Phải có ít nhất 1 khách");
    assert_eq!(error.kind, AppErrorKind::User);
}

#[test]
fn check_in_maps_a_calendar_conflict_to_the_stable_room_unavailable_code() {
    let error = map_check_in_command_error(BookingError::conflict(
        "Room R101 has a reservation starting 2026-04-20 (Guest). Max 2 nights.",
    ));

    assert_eq!(error.code, "CONFLICT_ROOM_UNAVAILABLE");
    assert!(error.message.contains("has a reservation starting"));
    assert_eq!(error.kind, AppErrorKind::User);
    assert!(error.support_id.is_none());
}

#[test]
fn check_in_maps_an_unrecognised_conflict_to_invalid_state() {
    let error = map_check_in_command_error(BookingError::conflict("something else entirely"));

    assert_eq!(error.code, "BOOKING_INVALID_STATE");
    assert_eq!(error.kind, AppErrorKind::User);
}

#[test]
fn check_in_maps_a_write_failure_to_a_system_error_with_a_support_id() {
    let error = map_check_in_command_error(BookingError::database_write("disk full"));

    assert_eq!(error.code, "SYSTEM_INTERNAL_ERROR");
    assert_eq!(error.kind, AppErrorKind::System);
    assert!(error.support_id.is_some());
    assert!(!error.retryable);
}

#[test]
fn check_in_maps_a_locked_database_to_a_retryable_code() {
    let error = map_check_in_command_error(BookingError::database("database is locked"));

    assert_eq!(error.code, "DB_LOCKED_RETRYABLE");
    assert_eq!(error.kind, AppErrorKind::System);
    assert!(error.retryable);
}

#[test]
fn check_out_maps_both_invalid_settlement_total_messages_to_one_code() {
    for message in [
        "Tổng quyết toán phải lớn hơn hoặc bằng 0",
        "final_total must be greater than or equal to 0",
    ] {
        let error = map_check_out_command_error(BookingError::validation(message));

        assert_eq!(error.code, "BOOKING_INVALID_SETTLEMENT_TOTAL", "{message}");
        assert_eq!(error.message, message);
        assert_eq!(error.kind, AppErrorKind::User);
    }
}

#[test]
fn check_out_maps_the_overpaid_guard_to_invalid_state() {
    let error = map_check_out_command_error(BookingError::validation(
        "Overpaid booking requires refund handling before checkout",
    ));

    assert_eq!(error.code, "BOOKING_INVALID_STATE");
    assert_eq!(
        error.message,
        "Overpaid booking requires refund handling before checkout"
    );
    assert_eq!(error.kind, AppErrorKind::User);
}

/// Unlike check-in, check-out has no room lookup, so *every* missing row is a
/// missing booking.
#[test]
fn check_out_maps_a_missing_room_message_to_booking_not_found() {
    let error = map_check_out_command_error(BookingError::not_found("Không tìm thấy phòng R101"));

    assert_eq!(error.code, "BOOKING_NOT_FOUND");
}

#[test]
fn check_out_maps_a_locked_database_read_to_a_retryable_code() {
    let error = map_check_out_command_error(BookingError::database("database is locked"));

    assert_eq!(error.code, "DB_LOCKED_RETRYABLE");
    assert_eq!(error.kind, AppErrorKind::System);
    assert!(error.retryable);
    assert!(error.support_id.is_some());
}

#[test]
fn extend_stay_maps_a_calendar_conflict_to_room_unavailable() {
    let error = map_extend_stay_command_error(BookingError::conflict(
        "Room R101 is booked on 2026-04-21 (Guest).",
    ));

    assert_eq!(error.code, "CONFLICT_ROOM_UNAVAILABLE");
    assert_eq!(error.kind, AppErrorKind::User);
}

#[test]
fn extend_stay_maps_missing_room_to_room_not_found() {
    let error = map_extend_stay_command_error(BookingError::not_found("Không tìm thấy phòng R101"));

    assert_eq!(error.code, "ROOM_NOT_FOUND");
    assert_eq!(error.kind, AppErrorKind::User);
}

#[test]
fn extend_stay_maps_a_datetime_parse_failure_to_a_system_error() {
    let error = map_extend_stay_command_error(BookingError::DateTimeParse("bad date".to_string()));

    assert_eq!(error.code, "SYSTEM_INTERNAL_ERROR");
    assert_eq!(error.kind, AppErrorKind::System);
    assert!(error.support_id.is_some());
}
