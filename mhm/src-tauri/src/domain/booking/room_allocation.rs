//! Choosing which vacant rooms a group gets.
//!
//! Pure: it takes the vacant rooms and the count, and returns the picks. No
//! database, so the policy is testable on its own.
//!
//! The policy is *keep the group together*: fill from the floor with the most
//! vacancies first, so a party of four lands on one floor rather than scattered
//! across three.

use std::collections::BTreeMap;

use crate::models::{Room, RoomAssignment};

/// Not enough vacant rooms to satisfy the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotEnoughRooms {
    pub available: usize,
    pub requested: usize,
}

/// Picks `requested` rooms, preferring floors that can host the most of the
/// group.
///
/// Ties are broken by floor number ascending. That tie-break is load-bearing:
/// this used to group into a `std::collections::HashMap` and sort by size
/// alone, so two floors with equally many vacancies came out in whatever order
/// the hash seed produced — a different answer on each app launch for the same
/// database.
pub fn assign_rooms_by_floor(
    vacant_rooms: &[Room],
    requested: usize,
) -> Result<Vec<RoomAssignment>, NotEnoughRooms> {
    if vacant_rooms.len() < requested {
        return Err(NotEnoughRooms {
            available: vacant_rooms.len(),
            requested,
        });
    }

    let mut by_floor: BTreeMap<i32, Vec<&Room>> = BTreeMap::new();
    for room in vacant_rooms {
        by_floor.entry(room.floor).or_default().push(room);
    }

    // BTreeMap yields floors ascending; `sort_by_key` is stable, so equal-sized
    // floors keep that ascending order.
    let mut floors: Vec<(i32, Vec<&Room>)> = by_floor.into_iter().collect();
    floors.sort_by_key(|(_, rooms)| std::cmp::Reverse(rooms.len()));

    let mut assignments = Vec::with_capacity(requested);
    for (floor, rooms) in &floors {
        for room in rooms {
            if assignments.len() >= requested {
                return Ok(assignments);
            }
            assignments.push(RoomAssignment {
                room: (*room).clone(),
                floor: *floor,
            });
        }
    }

    Ok(assignments)
}

#[cfg(test)]
mod tests {
    use super::assign_rooms_by_floor;
    use crate::models::Room;

    fn room(id: &str, floor: i32) -> Room {
        Room {
            id: id.to_string(),
            name: format!("Room {id}"),
            room_type: "standard".to_string(),
            floor,
            has_balcony: false,
            base_price: 300_000,
            max_guests: 2,
            extra_person_fee: 0,
            status: "vacant".to_string(),
        }
    }

    fn ids(rooms: &[Room], requested: usize) -> Vec<String> {
        assign_rooms_by_floor(rooms, requested)
            .unwrap_or_else(|_| panic!("expected {requested} rooms to be assignable"))
            .into_iter()
            .map(|assignment| assignment.room.id)
            .collect()
    }

    #[test]
    fn the_floor_that_can_host_the_whole_group_wins_over_a_lower_floor() {
        let rooms = vec![
            room("101", 1),
            room("201", 2),
            room("202", 2),
            room("203", 2),
        ];

        assert_eq!(
            ids(&rooms, 3),
            vec!["201", "202", "203"],
            "floor 2 keeps the group together; floor 1 cannot"
        );
    }

    #[test]
    fn equally_sized_floors_break_the_tie_by_floor_number() {
        let rooms = vec![
            room("301", 3),
            room("302", 3),
            room("101", 1),
            room("102", 1),
        ];

        // Input order puts floor 3 first; the answer must not depend on that.
        assert_eq!(ids(&rooms, 2), vec!["101", "102"]);
    }

    #[test]
    fn the_same_input_always_gives_the_same_answer() {
        // Enough distinct equal-sized floors that a hash-ordered grouping would
        // reorder them across runs.
        let rooms: Vec<_> = (1..=8).flat_map(|f| [room(&format!("{f}01"), f)]).collect();

        let first = ids(&rooms, 3);
        for _ in 0..25 {
            assert_eq!(ids(&rooms, 3), first);
        }
        assert_eq!(first, vec!["101", "201", "301"]);
    }

    #[test]
    fn a_group_larger_than_one_floor_spills_onto_the_next_biggest() {
        let rooms = vec![
            room("101", 1),
            room("201", 2),
            room("202", 2),
            room("301", 3),
        ];

        assert_eq!(
            ids(&rooms, 3),
            vec!["201", "202", "101"],
            "floor 2 first, then the tie between floors 1 and 3 goes to floor 1"
        );
    }

    #[test]
    fn asking_for_more_than_is_vacant_reports_both_numbers() {
        let rooms = vec![room("101", 1), room("102", 1)];

        let error = assign_rooms_by_floor(&rooms, 5).expect_err("should fail");
        assert_eq!(error.available, 2);
        assert_eq!(error.requested, 5);
    }

    #[test]
    fn asking_for_none_assigns_none_even_when_rooms_are_free() {
        assert!(assign_rooms_by_floor(&[room("101", 1)], 0)
            .expect("zero is satisfiable")
            .is_empty());
    }

    #[test]
    fn every_assignment_reports_the_floor_of_its_own_room() {
        let rooms = vec![room("101", 1), room("201", 2), room("202", 2)];

        for assignment in assign_rooms_by_floor(&rooms, 3).expect("assignable") {
            assert_eq!(assignment.floor, assignment.room.floor);
        }
    }
}
