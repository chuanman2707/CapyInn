import type { RoomStatus } from "@/types";

export const STATUS_LABELS: Record<RoomStatus, string> = {
  vacant: "Trống",
  occupied: "Có khách",
  cleaning: "Cần dọn",
  booked: "Đặt trước",
};

export const STATUS_COLORS: Record<RoomStatus, string> = {
  vacant: "bg-emerald-50 text-emerald-700 border-emerald-200",
  occupied: "bg-blue-50 text-blue-700 border-blue-200",
  cleaning: "bg-amber-50 text-amber-700 border-amber-200",
  booked: "bg-purple-50 text-purple-700 border-purple-200",
};

export const STATUS_DOT_COLORS: Record<RoomStatus, string> = {
  vacant: "bg-status-vacant-border",
  occupied: "bg-status-paid-border",
  cleaning: "bg-status-unpaid-border",
  booked: "bg-status-partPaid-border",
};

export const ROOM_STATUS_CARD_BG: Record<RoomStatus, string> = {
  vacant: "bg-status-vacant-bg",
  occupied: "bg-status-paid-bg",
  cleaning: "bg-status-unpaid-bg",
  booked: "bg-status-partPaid-bg",
};

export const ROOM_STATUS_TEXT: Record<RoomStatus, string> = {
  vacant: "text-status-vacant-text",
  occupied: "text-status-paid-text",
  cleaning: "text-status-unpaid-text",
  booked: "text-status-partPaid-text",
};

export const ROOM_TYPE_LABELS: Record<string, string> = {
  deluxe: "Deluxe",
  standard: "Standard",
};

export function getRoomTypeLabel(roomType: string): string {
  const override = ROOM_TYPE_LABELS[roomType];
  if (override) return override;

  return roomType
    .replace(/[_-]+/g, " ")
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}
