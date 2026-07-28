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

  it("dòng diễn giải nói badge đếm cái gì", async () => {
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_undeclared_breakdown")
        return Promise.resolve({ total: 6, not_scanned: 0, not_exported: 3, held: 1, awaiting: 2 });
      if (cmd === "kbtt_pending_rows") return Promise.resolve([]);
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_list_batches") return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<Declaration />);
    await waitFor(() =>
      expect(
        screen.getByText(/6 khách chưa khai xong: 3 chưa xuất file · 2 chờ đối chiếu · 1 gác lại/),
      ).toBeTruthy(),
    );
  });

  // FINDING C2: trang khai báo giờ sống qua tab switch (KeepMounted), nên
  // không có gì unmount/remount để tự tải lại dữ liệu khi quay lại tab.
  // `reactivateSignal` là tín hiệu cha (MainShell/KeepMounted) bơm vào mỗi
  // lần quay lại tab — component phải tự bump reloadKey khi tín hiệu đó đổi,
  // để badge/GuestList/ReconcilePanel tải lại đúng lúc quay lại, không phải
  // mỗi lần render.
  describe("reactivateSignal", () => {
    it("mount lần đầu chỉ gọi kbtt_pending_rows đúng một lần, không tải thêm vì reactivateSignal ban đầu", async () => {
      render(<Declaration reactivateSignal={0} />);
      await waitFor(() =>
        expect(screen.getByText(/chưa khai báo \(0\)/i)).toBeTruthy(),
      );
      const callsAfterMount = invokeCommand.mock.calls.filter(
        ([cmd]) => cmd === "kbtt_pending_rows",
      ).length;
      expect(callsAfterMount).toBe(1);
    });

    it("reactivateSignal đổi (quay lại tab) tải lại danh sách khách", async () => {
      const { rerender } = render(<Declaration reactivateSignal={0} />);
      await waitFor(() =>
        expect(screen.getByText(/chưa khai báo \(0\)/i)).toBeTruthy(),
      );
      const callsBefore = invokeCommand.mock.calls.filter(
        ([cmd]) => cmd === "kbtt_pending_rows",
      ).length;
      expect(callsBefore).toBe(1);

      rerender(<Declaration reactivateSignal={1} />);

      await waitFor(() => {
        const callsAfter = invokeCommand.mock.calls.filter(
          ([cmd]) => cmd === "kbtt_pending_rows",
        ).length;
        expect(callsAfter).toBe(2);
      });
    });

    it("re-render với cùng reactivateSignal không tải lại thêm lần nào (không dội backend vô hình)", async () => {
      const { rerender } = render(<Declaration reactivateSignal={0} />);
      await waitFor(() =>
        expect(screen.getByText(/chưa khai báo \(0\)/i)).toBeTruthy(),
      );

      rerender(<Declaration reactivateSignal={0} />);
      rerender(<Declaration reactivateSignal={0} />);

      // Đợi một tick cho mọi effect có cơ hội chạy rồi mới đếm.
      await waitFor(() => expect(screen.getByText(/chưa khai báo \(0\)/i)).toBeTruthy());
      const calls = invokeCommand.mock.calls.filter(
        ([cmd]) => cmd === "kbtt_pending_rows",
      ).length;
      expect(calls).toBe(1);
    });
  });

  it("dòng diễn giải thêm caveat khi có chồng lấn", async () => {
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_undeclared_breakdown")
        return Promise.resolve({ total: 4, not_scanned: 2, not_exported: 1, held: 0, awaiting: 1 });
      if (cmd === "kbtt_pending_rows") return Promise.resolve([]);
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_list_batches") return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<Declaration />);
    await waitFor(() =>
      expect(
        screen.getByText(/4 khách chưa khai xong: 2 lưu trú chưa xác nhận · 1 chưa xuất file · 1 chờ đối chiếu, một số khách ghi nhận ở nhiều mục/),
      ).toBeTruthy(),
    );
  });
});
