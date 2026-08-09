/**
 * Mock data factories for tests.
 * Creates realistic test data matching Rust backend interfaces.
 */
import type { Booking, DashboardStats, Guest, Room, RoomWithBooking } from "@/types";

let idCounter = 0;
const nextId = () => `test-${++idCounter}`;

export function resetIdCounter() {
    idCounter = 0;
}

// --- Room ---
export function createRoom(overrides: Partial<Room> = {}): Room {
    return {
        id: overrides.id ?? nextId(),
        name: overrides.name ?? "1A",
        type: overrides.type ?? "deluxe",
        floor: overrides.floor ?? 1,
        has_balcony: overrides.has_balcony ?? true,
        base_price: overrides.base_price ?? 350000,
        max_guests: overrides.max_guests ?? 2,
        extra_person_fee: overrides.extra_person_fee ?? 0,
        status: overrides.status ?? "vacant",
        ...overrides,
    };
}

export function createAllRooms(): Room[] {
    return [
        createRoom({ id: "1A", name: "1A", type: "deluxe", floor: 1, has_balcony: true, base_price: 400000, status: "vacant" }),
        createRoom({ id: "1B", name: "1B", type: "standard", floor: 1, has_balcony: false, base_price: 300000, status: "vacant" }),
        createRoom({ id: "2A", name: "2A", type: "deluxe", floor: 2, has_balcony: true, base_price: 400000, status: "occupied" }),
        createRoom({ id: "2B", name: "2B", type: "standard", floor: 2, has_balcony: false, base_price: 300000, status: "vacant" }),
        createRoom({ id: "3A", name: "3A", type: "deluxe", floor: 3, has_balcony: true, base_price: 400000, status: "booked" }),
        createRoom({ id: "3B", name: "3B", type: "standard", floor: 3, has_balcony: false, base_price: 300000, status: "occupied" }),
        createRoom({ id: "4A", name: "4A", type: "deluxe", floor: 4, has_balcony: true, base_price: 400000, status: "vacant" }),
        createRoom({ id: "4B", name: "4B", type: "standard", floor: 4, has_balcony: false, base_price: 300000, status: "vacant" }),
        createRoom({ id: "5A", name: "5A", type: "deluxe", floor: 5, has_balcony: true, base_price: 400000, status: "occupied" }),
        createRoom({ id: "5B", name: "5B", type: "standard", floor: 5, has_balcony: false, base_price: 300000, status: "vacant" }),
    ];
}

// --- Guest ---
export function createGuest(overrides: Partial<Guest> = {}): Guest {
    return {
        id: nextId(),
        guest_type: "domestic",
        full_name: "Nguyễn Văn A",
        doc_number: "012345678901",
        dob: "1990-01-15",
        gender: "Nam",
        nationality: "Việt Nam",
        address: "123 Nguyễn Huệ, Q.1, TP.HCM",
        created_at: new Date().toISOString(),
        ...overrides,
    };
}

// --- Booking ---
export function createBooking(overrides: Partial<Booking> = {}): Booking {
    const now = new Date();
    const tomorrow = new Date(now);
    tomorrow.setDate(tomorrow.getDate() + 1);

    return {
        id: nextId(),
        room_id: "2A",
        primary_guest_id: "guest-1",
        check_in_at: now.toISOString(),
        expected_checkout: tomorrow.toISOString(),
        nights: 1,
        total_price: 400000,
        paid_amount: 400000,
        status: "active",
        source: "walk-in",
        created_at: now.toISOString(),
        ...overrides,
    };
}

// --- Dashboard Stats ---
export function createStats(overrides: Partial<DashboardStats> = {}): DashboardStats {
    return {
        total_rooms: 10,
        occupied: 3,
        vacant: 6,
        revenue_today: 1200000,
        ...overrides,
    };
}

// --- Room With Booking ---
export function createRoomWithBooking(overrides: { room?: Partial<Room>; booking?: Partial<Booking> | null; guests?: Guest[] } = {}): RoomWithBooking {
    const room = createRoom({ id: "2A", name: "2A", status: "occupied", ...overrides.room });
    const booking = overrides.booking === null ? null : createBooking({ room_id: room.id, ...overrides.booking });
    const guests = overrides.guests ?? [createGuest()];

    return { room, booking, guests };
}

// --- User (Auth) ---
export function createUser(overrides: Partial<{ id: string; name: string; role: "admin" | "receptionist"; active: boolean; created_at: string }> = {}) {
    return {
        id: nextId(),
        name: "Admin",
        role: "admin" as const,
        active: true,
        created_at: new Date().toISOString(),
        ...overrides,
    };
}

// --- Booking With Guest (for listing) ---
export function createBookingWithGuest(overrides: Partial<{ id: string; room_id: string; room_name: string; guest_name: string; check_in_at: string; expected_checkout: string; actual_checkout: string | null; total_price: number; paid_amount: number; status: string; nights: number; source: string | null; booking_type: string | null; deposit_amount: number | null; scheduled_checkin: string | null; scheduled_checkout: string | null; guest_phone: string | null }> = {}) {
    const now = new Date();
    return {
        id: nextId(),
        room_id: "2A",
        room_name: "2A",
        guest_name: "Nguyễn Văn A",
        check_in_at: now.toISOString(),
        expected_checkout: new Date(now.getTime() + 86400000).toISOString(),
        actual_checkout: null,
        total_price: 400000,
        paid_amount: 400000,
        status: "active",
        nights: 1,
        source: "walk-in",
        booking_type: "walk-in",
        deposit_amount: null,
        scheduled_checkin: null,
        scheduled_checkout: null,
        guest_phone: null,
        ...overrides,
    };
}

// --- Guest Summary (for listing) ---
export function createGuestSummary(overrides: Partial<{ id: string; full_name: string; doc_number: string; nationality: string | null; total_stays: number; total_spent: number; last_visit: string | null }> = {}) {
    return {
        id: nextId(),
        full_name: "Nguyễn Văn A",
        doc_number: "012345678901",
        nationality: "Việt Nam",
        total_stays: 3,
        total_spent: 1200000,
        last_visit: new Date().toISOString(),
        ...overrides,
    };
}

// --- Analytics Data ---
export function createAnalyticsData(overrides: Partial<Record<string, unknown>> = {}) {
    return {
        period: "today",
        total_revenue: 2400000,
        total_expenses: 500000,
        net_profit: 1900000,
        occupancy_rate: 70.0,
        rooms_sold: 7,
        avg_rate: 342857,
        revenue_by_day: [
            { date: "2026-03-14", revenue: 1200000 },
            { date: "2026-03-15", revenue: 1200000 },
        ],
        top_rooms: [
            { room_id: "2A", room_name: "2A", revenue: 800000, nights: 2 },
        ],
        source_breakdown: [
            { source: "walk-in", count: 5, revenue: 2000000 },
            { source: "agoda", count: 2, revenue: 400000 },
        ],
        ...overrides,
    };
}
