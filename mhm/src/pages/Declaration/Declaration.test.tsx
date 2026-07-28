import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DeclarationFinding, DeclarationRow } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({
  toast: { error: toastError, success: toastSuccess },
}));

import Declaration from "./index";

function row(over: Partial<DeclarationRow>): DeclarationRow {
  return {
    link_id: "l1", identity_id: "i1", full_name: "Nguyễn Văn A", dob: "1980-05-02",
    gender: "M", nationality_iso3: "VNM", doc_type_code: "1", doc_type_name: null,
    doc_no: "058195006173", phone: null, residence_status: null, address_detail: null,
    passport_no: null, passport_expiry: null, visa_valid_until: null, room_no: null,
    check_in_date: "2026-07-27", expected_check_out: "2026-07-28", stay_reason: "1",
    stay_reason_note: null, name_confirmed_by_human: true, single_token_name_ok: false,
    held: false, stay_id: null,
    ...over,
  };
}

function finding(over: Partial<DeclarationFinding>): DeclarationFinding {
  return {
    code: "ERR", severity: "warning", link_id: "l1", field: null, message: "Error",
    ...over,
  };
}

describe("Declaration page", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeCommand.mockImplementation((command: string) => {
      switch (command) {
        case "kbtt_pending_rows":
        case "kbtt_list_stays":
        case "kbtt_validate":
        case "kbtt_list_batches":
          return Promise.resolve([]);
        case "kbtt_undeclared_count":
          return Promise.resolve(0);
        default:
          return Promise.resolve([]);
      }
    });
  });

  it("trang rỗng: danh sách trống và nút xuất mờ", async () => {
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_pending_rows") return Promise.resolve([]);
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_validate") return Promise.resolve([]);
      if (cmd === "kbtt_list_batches") return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<Declaration />);

    await waitFor(() =>
      expect(screen.getByText(/chưa khai báo \(0\)/i)).toBeTruthy(),
    );
    const button = screen.getByRole("button", { name: /xuất file/i });
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });

  it("lỗi chặn không cho xuất, lỗi cảnh báo vẫn cho xuất", async () => {
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_pending_rows") {
        return Promise.resolve([
          row({ link_id: "l1", full_name: "Nguyễn Văn A" }),
          row({ link_id: "l2", identity_id: "i2", full_name: "Trần Thị B" }),
          row({ link_id: "l3", identity_id: "i3", full_name: "Hoàng Văn C" }),
        ]);
      }
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_validate") {
        return Promise.resolve([
          finding({ link_id: "l1", severity: "blocking" }),
          finding({ link_id: "l2", severity: "warning" }),
          finding({ link_id: "l3", severity: "blocking" }),
        ]);
      }
      if (cmd === "kbtt_list_batches") return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<Declaration />);

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /xuất file cho 1 khách/i })).toBeTruthy(),
    );
    expect(screen.getByText(/2 khách còn lỗi sẽ ở lại danh sách/)).toBeTruthy();
  });

  // FINDING I5 — GuestList từng nuốt lỗi kbtt_validate thành setFindings([]),
  // nên index.tsx tính blockedLinks rỗng và nút xuất mời xuất TOÀN BỘ khách
  // dù backend sẽ từ chối cả lô ("Còn lỗi chặn, không xuất được"). Giờ khi
  // kiểm tra lỗi, không khách nào được coi là đủ điều kiện và người vận hành
  // phải thấy rõ lý do.
  it("kbtt_validate lỗi: không mời xuất khách nào, báo người dùng là kiểm tra thất bại", async () => {
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_pending_rows") {
        return Promise.resolve([
          row({ link_id: "l1", full_name: "Nguyễn Văn A" }),
          row({ link_id: "l2", identity_id: "i2", full_name: "Trần Thị B" }),
        ]);
      }
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_validate") return Promise.reject(new Error("mất kết nối"));
      if (cmd === "kbtt_list_batches") return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<Declaration />);

    await waitFor(() => expect(screen.getByText("Nguyễn Văn A")).toBeTruthy());
    // Lỗi validate đến sau một tick nữa (rows nạp xong rồi mới gọi
    // kbtt_validate) — đợi đúng thông báo lỗi thay vì đọc DOM ngay lập tức.
    await waitFor(() => expect(screen.getByText(/kiểm tra lỗi/i)).toBeTruthy());

    const button = screen.getByRole("button", { name: /xuất file/i });
    expect((button as HTMLButtonElement).disabled).toBe(true);
    expect(button.textContent).not.toMatch(/xuất file cho \d/i);
  });
});
