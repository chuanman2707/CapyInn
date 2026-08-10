#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockDeriverId {
    RoomFromRequest,
    BookingAndRoomFromBooking,
    GroupCheckinRooms,
    GroupCheckoutBookingsAndRooms,
    ReservationBookingAndRoom,
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
        command_name: "shorten_stay",
        lock_deriver: LockDeriverId::BookingAndRoomFromBooking,
        enforced_in_foundation: true,
    },
    WriteCommandMeta {
        // Đổi giá không đụng phòng: chỉ khoá `booking:` + `folio:`, không lấy
        // khoá phòng. Xem `set_booking_rate_idempotent`.
        command_name: "set_booking_rate",
        lock_deriver: LockDeriverId::FolioBooking,
        enforced_in_foundation: true,
    },
    WriteCommandMeta {
        command_name: "change_room",
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
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn src_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    /// Tên lệnh đã đăng ký trong `tauri::generate_handler![…]` của `lib.rs` —
    /// bề mặt IPC thật. Lấy đoạn cuối của mỗi đường dẫn
    /// (`commands::rooms::check_in` → `check_in`): tên lệnh IPC là tên hàm,
    /// `rename_all` chỉ đổi cách viết **tham số** chứ không đổi tên lệnh.
    fn registered_command_names() -> BTreeSet<String> {
        let source = fs::read_to_string(src_dir().join("lib.rs")).expect("read lib.rs");
        let mut names = BTreeSet::new();
        let mut inside = false;

        for line in source.lines() {
            if !inside {
                inside = line.contains("generate_handler![");
                continue;
            }
            let trimmed = line.trim();
            if trimmed.starts_with("])") {
                break;
            }
            let code = trimmed.split("//").next().unwrap_or_default();
            for entry in code.split(',') {
                let path = entry.trim();
                if path.is_empty() {
                    continue;
                }
                names.insert(path.rsplit("::").next().unwrap_or(path).to_string());
            }
        }

        names
    }

    /// Mọi hàm mang `#[tauri::command…]` trong cây `src/`.
    ///
    /// `starts_with` chứ **không** so bằng `==`: `#[tauri::command(rename_all =
    /// "snake_case")]` cũng là một lệnh thật y hệt. Vì so trên dòng đã `trim`,
    /// một chuỗi hay dòng chú thích *chứa* đoạn text đó (có trong
    /// `commands/assistant_conversations.rs`) không lọt vào.
    fn declared_command_names() -> BTreeSet<String> {
        let mut files = Vec::new();
        collect_rust_files(&src_dir(), &mut files);

        let mut names = BTreeSet::new();
        for file in files {
            let source = fs::read_to_string(&file).expect("read source file");
            let mut lines = source.lines();

            while let Some(line) = lines.next() {
                if !line.trim().starts_with("#[tauri::command") {
                    continue;
                }
                // Quét tới chữ ký: giữa attribute và `fn` có thể còn attribute khác.
                for line in lines.by_ref() {
                    let Some((_, rest)) = line.split_once("fn ") else {
                        continue;
                    };
                    let name = rest
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                        .unwrap_or_default();
                    if !name.is_empty() {
                        names.insert(name.to_string());
                    }
                    break;
                }
            }
        }

        names
    }

    /// Meta-test cho hai bộ đọc trên. Cả hai đọc mã nguồn theo hình dạng
    /// rustfmt giữ; nếu một bộ đọc hỏng và trả về rỗng thì test hai chiều bên
    /// dưới vẫn đỏ, nhưng đỏ vì lý do sai. Chốt trước ở đây để thông điệp lỗi
    /// chỉ đúng chỗ.
    #[test]
    fn command_surface_readers_still_parse_the_source() {
        let registered = registered_command_names();
        let declared = declared_command_names();

        // Ngưỡng để rời xa 0, không phải để đếm chính xác: bề mặt lệnh là 132
        // lúc viết test này và được phép co giãn.
        assert!(
            registered.len() > 50,
            "chỉ đọc được {} lệnh trong `generate_handler![…]` — bộ đọc hỏng, không phải manifest",
            registered.len()
        );
        assert!(
            declared.len() > 50,
            "chỉ đọc được {} hàm `#[tauri::command]` — bộ đọc hỏng, không phải manifest",
            declared.len()
        );
        for anchor in ["check_in", "get_rooms"] {
            assert!(
                registered.contains(anchor),
                "{anchor} phải đọc được từ lib.rs"
            );
            assert!(
                declared.contains(anchor),
                "{anchor} phải đọc được từ commands/"
            );
        }
    }

    /// Chiều thuận: mọi lệnh bắt buộc phải có dòng trong manifest.
    ///
    /// Đi cùng `command_manifest_has_no_entry_for_a_removed_command` bên dưới —
    /// một mình nó chỉ là kiểm tra chứa một chiều, không bắt được dòng mồ côi.
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
            "shorten_stay",
            "set_booking_rate",
            "change_room",
            "group_checkin",
            "group_checkout",
            "confirm_reservation",
            "cancel_reservation",
            "modify_reservation",
            "create_reservation",
            "add_folio_line",
            "record_payment",
        ] {
            assert!(command_names.contains(expected), "missing {expected}");
        }
    }

    /// Chiều ngược: mọi dòng trong manifest phải trỏ vào một lệnh còn sống.
    ///
    /// Thiếu chiều này thì một `WriteCommandMeta` mồ côi — lệnh đã gỡ nhưng
    /// dòng manifest ở lại — không bao giờ làm CI đỏ. Đúng cảnh đã xảy ra ở
    /// nhánh gỡ housekeeping: nếu dòng `update_housekeeping` bị bỏ quên thì
    /// không test Rust nào bắt. Bản TS khớp allowlist hai chiều
    /// (`mhm/tests/frontend-invoke-wrapper-guardrails.test.ts`) và chính chiều
    /// ngược đó đã buộc phải gỡ dòng housekeeping bên ấy; đây là nửa còn thiếu
    /// ở phía Rust.
    #[test]
    fn command_manifest_has_no_entry_for_a_removed_command() {
        let registered = registered_command_names();
        let declared = declared_command_names();

        for meta in WRITE_COMMAND_MANIFEST {
            let name = meta.command_name;
            assert!(
                declared.contains(name),
                "{name}: manifest còn khai dòng này nhưng không còn hàm `#[tauri::command]` nào tên vậy — gỡ dòng manifest"
            );
            assert!(
                registered.contains(name),
                "{name}: có hàm nhưng không nằm trong `generate_handler![…]` của lib.rs — lệnh đã rời bề mặt IPC, manifest phải theo"
            );
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
