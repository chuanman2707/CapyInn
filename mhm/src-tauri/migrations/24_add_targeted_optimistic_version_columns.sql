-- Issue #88: targeted optimistic version columns.
--
-- Only operator-mutable aggregates with overwrite-prone UPDATE paths get a
-- version column: bookings (reservation/stay lifecycle) and rooms (room
-- status/availability). Financial append-only tables are intentionally left
-- unchanged.
--
-- The columns are additive and backfill with version 1, so existing rows
-- remain readable and the first guarded UPDATE from any current client
-- succeeds against the version it read.

ALTER TABLE bookings ADD COLUMN optimistic_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE rooms ADD COLUMN optimistic_version INTEGER NOT NULL DEFAULT 1;
