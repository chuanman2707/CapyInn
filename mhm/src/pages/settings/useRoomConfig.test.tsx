import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  formatAppError,
  type AppError,
} from "@/lib/appError";

const invokeCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand,
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastError,
    success: toastSuccess,
  },
}));

import useRoomConfig from "./useRoomConfig";

const duplicateRoomError: AppError = {
  code: "ROOM_ALREADY_EXISTS",
  message: "Phòng đã tồn tại",
  kind: "user",
  support_id: null,
};

const duplicateRoomTypeError: AppError = {
  code: "ROOM_TYPE_ALREADY_EXISTS",
  message: "Loại phòng đã tồn tại",
  kind: "user",
  support_id: null,
};

const roomDeleteActiveBookingError: AppError = {
  code: "ROOM_DELETE_ACTIVE_BOOKING",
  message: "Không thể xóa phòng có booking đang hoạt động",
  kind: "user",
  support_id: null,
};

const roomNotFoundError: AppError = {
  code: "ROOM_NOT_FOUND",
  message: "Phòng không tồn tại",
  kind: "user",
  support_id: null,
};

const roomTypeInUseError: AppError = {
  code: "ROOM_TYPE_IN_USE",
  message: "Không thể xóa loại phòng đang được sử dụng",
  kind: "user",
  support_id: null,
};

describe("useRoomConfig", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeCommand.mockImplementation(async (command: string) => {
      switch (command) {
        case "get_rooms":
          return [];
        case "get_room_types":
          return [
            { id: "standard", name: "Standard", created_at: "2026-04-22T00:00:00Z" },
          ];
        case "create_room_type":
          throw duplicateRoomTypeError;
        case "delete_room_type":
          throw roomTypeInUseError;
        case "create_room":
          throw duplicateRoomError;
        case "update_room":
          throw roomNotFoundError;
        case "delete_room":
          throw roomDeleteActiveBookingError;
        default:
          throw new Error(`Unexpected command: ${command}`);
      }
    });
  });

  it("uses invokeCommand and shared formatting when adding a room type fails", async () => {
    const { result } = renderHook(() => useRoomConfig());

    await waitFor(() => expect(result.current.roomTypes).toHaveLength(1));

    await act(async () => {
      result.current.setNewTypeName("Deluxe");
    });
    await waitFor(() => expect(result.current.newTypeName).toBe("Deluxe"));

    await act(async () => {
      await result.current.handleAddType();
    });

    expect(invokeCommand).toHaveBeenCalledWith("create_room_type", {
      req: { name: "Deluxe" },
    });
    expect(toastError).toHaveBeenCalledWith(formatAppError(duplicateRoomTypeError));
  });

  it("trims the room type name in the success toast", async () => {
    invokeCommand.mockImplementation(async (command: string) => {
      switch (command) {
        case "get_rooms":
          return [];
        case "get_room_types":
          return [
            { id: "standard", name: "Standard", created_at: "2026-04-22T00:00:00Z" },
          ];
        case "create_room_type":
          return { id: "deluxe", name: "Deluxe", created_at: "2026-04-22T00:00:00Z" };
        default:
          throw new Error(`Unexpected command: ${command}`);
      }
    });

    const { result } = renderHook(() => useRoomConfig());

    await waitFor(() => expect(result.current.roomTypes).toHaveLength(1));

    await act(async () => {
      result.current.setNewTypeName("  Deluxe  ");
    });
    await waitFor(() => expect(result.current.newTypeName).toBe("  Deluxe  "));

    await act(async () => {
      await result.current.handleAddType();
    });

    expect(invokeCommand).toHaveBeenCalledWith("create_room_type", {
      req: { name: "Deluxe" },
    });
    expect(toastSuccess).toHaveBeenCalledWith('Đã tạo loại phòng "Deluxe"');
  });

  it("uses invokeCommand and shared formatting when deleting a room type fails", async () => {
    const { result } = renderHook(() => useRoomConfig());

    await waitFor(() => expect(result.current.roomTypes).toHaveLength(1));

    await act(async () => {
      await result.current.handleDeleteType("standard");
    });

    expect(invokeCommand).toHaveBeenCalledWith("delete_room_type", {
      roomTypeId: "standard",
    });
    expect(toastError).toHaveBeenCalledWith(formatAppError(roomTypeInUseError));
  });

  it("keeps the room form open and formats the error when saving a room fails", async () => {
    const { result } = renderHook(() => useRoomConfig());

    await waitFor(() => expect(result.current.roomTypes).toHaveLength(1));

    await act(async () => {
      result.current.openAdd();
      result.current.setForm({
        id: "R501",
        name: "Room 501",
        room_type: "Standard",
        floor: 5,
        has_balcony: false,
        base_price: 500000,
        max_guests: 2,
        extra_person_fee: 100000,
      });
    });

    expect(result.current.showRoomForm).toBe(true);
    await waitFor(() => expect(result.current.form.id).toBe("R501"));

    await act(async () => {
      await result.current.handleSaveRoom();
    });

    expect(invokeCommand).toHaveBeenCalledWith("create_room", {
      req: {
        id: "R501",
        name: "Room 501",
        room_type: "Standard",
        floor: 5,
        has_balcony: false,
        base_price: 500000,
        max_guests: 2,
        extra_person_fee: 100000,
      },
    });
    expect(result.current.showRoomForm).toBe(true);
    expect(toastError).toHaveBeenCalledWith(formatAppError(duplicateRoomError));
  });

  it("keeps the room form open and formats the error when updating a room fails", async () => {
    const { result } = renderHook(() => useRoomConfig());

    await waitFor(() => expect(result.current.roomTypes).toHaveLength(1));

    await act(async () => {
      result.current.openEdit({
        id: "R701",
        name: "Room 701",
        type: "Standard",
        floor: 7,
        has_balcony: true,
        base_price: 700000,
        max_guests: 3,
        extra_person_fee: 150000,
        status: "vacant",
      });
      result.current.setForm({
        id: "R701",
        name: "Room 701 Updated",
        room_type: "Standard",
        floor: 7,
        has_balcony: true,
        base_price: 750000,
        max_guests: 3,
        extra_person_fee: 150000,
      });
    });

    expect(result.current.showRoomForm).toBe(true);
    await waitFor(() => expect(result.current.form.name).toBe("Room 701 Updated"));

    await act(async () => {
      await result.current.handleSaveRoom();
    });

    expect(invokeCommand).toHaveBeenCalledWith("update_room", {
      req: {
        room_id: "R701",
        name: "Room 701 Updated",
        room_type: "Standard",
        floor: 7,
        has_balcony: true,
        base_price: 750000,
        max_guests: 3,
        extra_person_fee: 150000,
      },
    });
    expect(result.current.showRoomForm).toBe(true);
    expect(toastError).toHaveBeenCalledWith(formatAppError(roomNotFoundError));
  });

  it("uses invokeCommand and shared formatting when deleting a room fails", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    const { result } = renderHook(() => useRoomConfig());

    await waitFor(() => expect(result.current.roomTypes).toHaveLength(1));

    await act(async () => {
      await result.current.handleDeleteRoom("R601");
    });

    expect(invokeCommand).toHaveBeenCalledWith("delete_room", {
      roomId: "R601",
    });
    expect(toastError).toHaveBeenCalledWith(formatAppError(roomDeleteActiveBookingError));

    confirmSpy.mockRestore();
  });
});
