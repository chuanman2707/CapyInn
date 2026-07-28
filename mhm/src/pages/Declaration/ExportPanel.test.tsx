import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "sonner";

import type { DeclarationRow } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import ExportPanel from "./ExportPanel";

function row(over: Partial<DeclarationRow>): DeclarationRow {
  return {
    link_id: "l1", identity_id: "i1", full_name: "Nguyễn Văn A", dob: "1980-05-02",
    gender: "M", nationality_iso3: "VNM", doc_type_code: "1", doc_type_name: null,
    doc_no: "058195006173", phone: null, residence_status: null, address_detail: null,
    passport_no: null, passport_expiry: null, visa_valid_until: null, room_no: "5A",
    stay_id: null,
    check_in_date: "2026-07-27", expected_check_out: "2026-07-28", stay_reason: "1",
    stay_reason_note: null, name_confirmed_by_human: true, single_token_name_ok: false,
    held: false,
    ...over,
  };
}

describe("ExportPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("một cú bấm chia hai file theo quốc tịch", async () => {
    invokeCommand.mockImplementation((cmd: string, args: { kind?: string }) => {
      if (cmd === "kbtt_export") {
        return Promise.resolve({
          batch_id: args.kind === "VN" ? "b-vn" : "b-nnn",
          file_path: args.kind === "VN" ? "/x/TBLT.xlsx" : "/x/KBTT.xml",
          row_count: 1,
          kind: args.kind,
        });
      }
      return Promise.resolve(null);
    });
    const onExported = vi.fn();
    const eligible = [
      row({}),
      row({ link_id: "l2", identity_id: "i2", full_name: "JOHN SMITH", nationality_iso3: "USA" }),
    ];
    render(<ExportPanel eligible={eligible} blockedCount={0} onExported={onExported} />);

    fireEvent.click(screen.getByRole("button", { name: /xuất file cho 2 khách/i }));

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_export",
        expect.objectContaining({ kind: "VN", linkIds: ["l1"] }),
      ),
    );
    expect(invokeCommand).toHaveBeenCalledWith(
      "kbtt_export",
      expect.objectContaining({ kind: "NNN", linkIds: ["l2"] }),
    );
    await waitFor(() => expect(screen.getByText(/TBLT\.xlsx/)).toBeTruthy());
    expect(screen.getByText(/KBTT\.xml/)).toBeTruthy();
    expect(screen.getByText(/không mở\/sửa file này bằng excel/i)).toBeTruthy();
    expect(onExported).toHaveBeenCalled();
  });

  it("chỉ một loại khách thì chỉ gọi một lần", async () => {
    invokeCommand.mockResolvedValue({
      batch_id: "b-vn", file_path: "/x/TBLT.xlsx", row_count: 1, kind: "VN",
    });
    render(<ExportPanel eligible={[row({})]} blockedCount={0} onExported={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /xuất file cho 1 khách/i }));
    await waitFor(() => expect(invokeCommand).toHaveBeenCalledTimes(1));
  });

  it("nút nói thật khi có khách bị lỗi chặn ở lại", () => {
    render(<ExportPanel eligible={[row({})]} blockedCount={2} onExported={() => {}} />);
    expect(screen.getByRole("button", { name: /xuất file cho 1 khách/i })).toBeTruthy();
    expect(screen.getByText(/2 khách còn lỗi sẽ ở lại danh sách/)).toBeTruthy();
  });

  it("không còn ai đủ điều kiện thì nút mờ", () => {
    render(<ExportPanel eligible={[]} blockedCount={1} onExported={() => {}} />);
    const button = screen.getByRole("button", { name: /xuất file/i });
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });

  // FINDING I5 — kiểm tra lỗi thì không được cho xuất, kể cả khi `eligible`
  // (tính từ dữ liệu cũ, không còn đáng tin) trông vẫn đầy đủ.
  it("checkFailed: nút mờ dù eligible không rỗng, và báo người dùng kiểm tra lỗi", () => {
    render(
      <ExportPanel
        eligible={[row({})]}
        blockedCount={0}
        checkFailed
        onExported={() => {}}
      />,
    );
    const button = screen.getByRole("button", { name: /xuất file/i });
    expect((button as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/kiểm tra lỗi/i)).toBeTruthy();
  });

  // FINDING A: kbtt_export trả Err(String) thẳng, không qua registry AppError
  // dùng chung. formatAppError() sẽ nuốt mất câu tiếng Việt thật và trả về
  // "Có lỗi hệ thống, vui lòng thử lại" — đúng lúc người vận hành cần đọc rõ
  // tại sao xuất file thất bại nhất. Toast phải giữ nguyên câu gốc.
  it("lỗi Err(String) từ kbtt_export lên toast nguyên văn, không phải câu chung chung", async () => {
    const raw = "Khách này đã có một khai báo cho lượt lưu trú đó rồi.";
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_export") return Promise.reject(raw);
      return Promise.resolve(null);
    });
    render(<ExportPanel eligible={[row({})]} blockedCount={0} onExported={() => {}} />);

    fireEvent.click(screen.getByRole("button", { name: /xuất file cho 1 khách/i }));

    await waitFor(() => expect(toast.error).toHaveBeenCalledWith(raw));
  });
});
