#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockDeriverId {
    RoomFromRequest,
    BookingAndRoomFromBooking,
    GroupCheckinRooms,
    GroupCheckoutBookingsAndRooms,
    ReservationBookingAndRoom,
    HousekeepingTaskRoom,
    FolioBooking,
    PaymentBooking,
}

impl LockDeriverId {
    pub const fn policy_name(self) -> &'static str {
        match self {
            Self::RoomFromRequest => "room_from_request",
            Self::BookingAndRoomFromBooking => "booking_and_room_from_booking",
            Self::GroupCheckinRooms => "group_checkin_rooms",
            Self::GroupCheckoutBookingsAndRooms => "group_checkout_bookings_and_rooms",
            Self::ReservationBookingAndRoom => "reservation_booking_and_room",
            Self::HousekeepingTaskRoom => "housekeeping_task_room",
            Self::FolioBooking => "folio_booking",
            Self::PaymentBooking => "payment_booking",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteCommandMeta {
    pub command_name: &'static str,
    pub lock_deriver: LockDeriverId,
    pub enforced_in_foundation: bool,
}

pub const WRITE_COMMAND_MANIFEST: &[WriteCommandMeta] = &[
    WriteCommandMeta {
        command_name: "check_in",
        lock_deriver: LockDeriverId::RoomFromRequest,
        enforced_in_foundation: true,
    },
    WriteCommandMeta {
        command_name: "check_out",
        lock_deriver: LockDeriverId::BookingAndRoomFromBooking,
        enforced_in_foundation: true,
    },
    WriteCommandMeta {
        command_name: "extend_stay",
        lock_deriver: LockDeriverId::BookingAndRoomFromBooking,
        enforced_in_foundation: true,
    },
    WriteCommandMeta {
        command_name: "group_checkin",
        lock_deriver: LockDeriverId::GroupCheckinRooms,
        enforced_in_foundation: true,
    },
    WriteCommandMeta {
        command_name: "group_checkout",
        lock_deriver: LockDeriverId::GroupCheckoutBookingsAndRooms,
        enforced_in_foundation: true,
    },
    WriteCommandMeta {
        command_name: "confirm_reservation",
        lock_deriver: LockDeriverId::ReservationBookingAndRoom,
        enforced_in_foundation: true,
    },
    WriteCommandMeta {
        command_name: "cancel_reservation",
        lock_deriver: LockDeriverId::ReservationBookingAndRoom,
        enforced_in_foundation: true,
    },
    WriteCommandMeta {
        command_name: "modify_reservation",
        lock_deriver: LockDeriverId::ReservationBookingAndRoom,
        enforced_in_foundation: true,
    },
    WriteCommandMeta {
        command_name: "update_housekeeping",
        lock_deriver: LockDeriverId::HousekeepingTaskRoom,
        enforced_in_foundation: true,
    },
    WriteCommandMeta {
        command_name: "create_reservation",
        lock_deriver: LockDeriverId::RoomFromRequest,
        enforced_in_foundation: false,
    },
    WriteCommandMeta {
        command_name: "add_folio_line",
        lock_deriver: LockDeriverId::FolioBooking,
        enforced_in_foundation: false,
    },
    WriteCommandMeta {
        command_name: "record_payment",
        lock_deriver: LockDeriverId::PaymentBooking,
        enforced_in_foundation: false,
    },
];

pub fn meta_for(command_name: &str) -> Option<&'static WriteCommandMeta> {
    WRITE_COMMAND_MANIFEST
        .iter()
        .find(|meta| meta.command_name == command_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_manifest_declares_required_lock_derivers() {
        let command_names = WRITE_COMMAND_MANIFEST
            .iter()
            .map(|meta| meta.command_name)
            .collect::<std::collections::BTreeSet<_>>();

        for expected in [
            "check_in",
            "check_out",
            "extend_stay",
            "group_checkin",
            "group_checkout",
            "confirm_reservation",
            "cancel_reservation",
            "modify_reservation",
            "update_housekeeping",
            "create_reservation",
            "add_folio_line",
            "record_payment",
        ] {
            assert!(command_names.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn metadata_only_commands_are_not_runtime_enforced() {
        let folio = meta_for("add_folio_line").expect("folio meta exists");
        let payment = meta_for("record_payment").expect("payment meta exists");

        assert!(!folio.enforced_in_foundation);
        assert!(!payment.enforced_in_foundation);
    }
}
