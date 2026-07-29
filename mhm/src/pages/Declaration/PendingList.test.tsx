import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DeclarationIdentity, DeclarationRow, StayInfo } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({
  toast: { error: toastError, success: toastSuccess },
}));

import PendingList from "./PendingList";
import { nameScore, normaliseName } from "./nameMatch";

const identity: DeclarationIdentity = {
  id: "i1",
  full_name: "ZOLOCHEVSKAIA VERONIKA",
  dob: "1990-03-08",
  gender: "F",
  nationality_iso3: "RUS",
  passport_no: "777785671",
  name_confirmed_by_human: true,
};

const stays: StayInfo[] = [
  {
    stay_id: "b1",
    room_no: "5A",
    check_in: "2026-07-25",
    expected_out: "2026-08-03",
    guest_name: "Andrei",
  },
  {
    stay_id: "b2",
    room_no: "5B",
    check_in: "2026-07-25",
    expected_out: "2026-07-26",
    guest_name: "Veronika Zolochevskaia",
  },
];

function row(over: Partial<DeclarationRow>): DeclarationRow {
  return {
    link_id: "l1",
    identity_id: "i1",
    full_name: "Nguyễn Văn A",
    dob: "1980-05-02",
    gender: "M",
    nationality_iso3: "VNM",
    doc_type_code: "1",
    doc_type_name: null,
    doc_no: "058195006173",
    phone: null,
    residence_status: null,
    address_detail: null,
    passport_no: null,
    passport_expiry: null,
    visa_valid_until: null,
    room_no: "101",
    check_in_date: "2026-07-25",
    expected_check_out: "2026-07-28",
    stay_reason: "1",
    stay_reason_note: null,
    name_confirmed_by_human: true,
    single_token_name_ok: false,
    ...over,
  };
}

function mockCommands(overrides: Record<string, unknown> = {}) {
  invokeCommand.mockImplementation((command: string) => {
    if (command in overrides) {
      return Promise.resolve(overrides[command]);
    }
    switch (command) {
      case "kbtt_pending_rows":
      case "kbtt_list_stays":
      case "kbtt_validate":
        return Promise.resolve([]);
      default:
        return Promise.resolve([]);
    }
  });
}

describe("nameMatch", () => {
  it("strips Vietnamese diacritics and lowercases", () => {
    expect(normaliseName("Nguyễn Văn Đức")).toEqual(["nguyen", "van", "duc"]);
  });

  it("scores token overlap regardless of order", () => {
    expect(nameScore("ZOLOCHEVSKAIA VERONIKA", "Veronika Zolochevskaia")).toBe(1);
    expect(nameScore("ZOLOCHEVSKAIA VERONIKA", "Andrei")).toBe(0);
    expect(nameScore("ZOLOCHEVSKAIA VERONIKA", "")).toBe(0);
  });
});

describe("PendingList", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCommands();
  });

  it("suggests bookings by name similarity but never auto-confirms", async () => {
    mockCommands({ kbtt_list_stays: stays, kbtt_unlinked_identities: [identity] });

    render(<PendingList identity={identity} />);

    const select = (await screen.findByLabelText(
      /Ghép với khách đang ở/i,
    )) as HTMLSelectElement;

    // app gợi ý thứ tự, nhưng không tự chọn giúp
    expect(select.value).toBe("");
  });

  it("orders the suggestions by name similarity", async () => {
    mockCommands({ kbtt_list_stays: stays, kbtt_unlinked_identities: [identity] });

    render(<PendingList identity={identity} />);

    const select = (await screen.findByLabelText(
      /Ghép với khách đang ở/i,
    )) as HTMLSelectElement;
    const values = Array.from(select.options).map((o) => o.value);

    // ô rỗng luôn đứng đầu, rồi tới ứng viên giống tên nhất
    expect(values[0]).toBe("");
    expect(values[1]).toBe("__chua_co_phong__");
    expect(values[2]).toBe("b2");
    expect(values[3]).toBe("b1");
  });

  it("links the identity to the stay the human picked", async () => {
    mockCommands({
      kbtt_list_stays: stays,
      kbtt_link: "link-1",
      kbtt_unlinked_identities: [identity],
    });
    const onLinked = vi.fn();

    render(<PendingList identity={identity} onLinked={onLinked} />);

    const select = (await screen.findByLabelText(
      /Ghép với khách đang ở/i,
    )) as HTMLSelectElement;
    fireEvent.change(select, { target: { value: "b1" } });
    fireEvent.click(screen.getByRole("button", { name: /^Ghép khách$/i }));

    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_link", {
        identityId: "i1",
        stayId: "b1",
        stayReason: "1",
        note: null,
      });
    });
    expect(onLinked).toHaveBeenCalled();
  });

  /**
   * Lỗi đã xảy ra: người vận hành thả ảnh CCCD, sang tab Dashboard rồi quay lại
   * thì tưởng mất dữ liệu. Danh tính vẫn nằm trong DB — chỉ là `useState` bị
   * hủy cùng component. Nguồn sự thật phải là DB.
   */
  it("keeps the waiting identity after the tab is switched away and back", async () => {
    mockCommands({ kbtt_list_stays: stays, kbtt_unlinked_identities: [identity] });

    const first = render(<PendingList identity={identity} />);
    await screen.findByText(/ZOLOCHEVSKAIA VERONIKA/);

    // Đổi tab = MainShell hủy component (render có điều kiện).
    first.unmount();

    // Quay lại tab, KHÔNG có prop identity nữa — chỉ còn DB.
    render(<PendingList />);

    expect(await screen.findByText(/ZOLOCHEVSKAIA VERONIKA/)).toBeInTheDocument();
    expect(
      await screen.findByLabelText(/Ghép với khách đang ở/i),
    ).toBeInTheDocument();
  });

  it("declares a guest who has no room yet", async () => {
    mockCommands({
      kbtt_list_stays: stays,
      kbtt_link: "link-1",
      kbtt_unlinked_identities: [identity],
    });

    render(<PendingList identity={identity} />);

    const select = (await screen.findByLabelText(
      /Ghép với khách đang ở/i,
    )) as HTMLSelectElement;
    fireEvent.change(select, { target: { value: "__chua_co_phong__" } });
    fireEvent.click(screen.getByRole("button", { name: /^Ghép khách$/i }));

    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_link", {
        identityId: "i1",
        stayId: null,
        stayReason: "1",
        note: null,
      });
    });
  });

  it("refuses to link until a stay is picked", async () => {
    mockCommands({ kbtt_list_stays: stays, kbtt_unlinked_identities: [identity] });

    render(<PendingList identity={identity} />);

    await screen.findByLabelText(/Ghép với khách đang ở/i);
    expect(screen.getByRole("button", { name: /^Ghép khách$/i })).toBeDisabled();
  });

  it("groups rows by NNN and VN because they export to different files", async () => {
    mockCommands({
      kbtt_pending_rows: [
        row({ link_id: "l1", nationality_iso3: "VNM" }),
        row({
          link_id: "l2",
          nationality_iso3: "RUS",
          full_name: "ZOLOCHEVSKAIA VERONIKA",
        }),
      ],
    });

    render(<PendingList />);

    // Await a *row*, not a section header. Both headers render immediately with
    // an empty "Không có khách nào" body, so awaiting one resolves on the first
    // paint and the synchronous row assertions below then race the pending
    // `kbtt_pending_rows` promise. That lost on CI while passing everywhere else.
    expect(await screen.findByText("ZOLOCHEVSKAIA VERONIKA")).toBeInTheDocument();
    expect(screen.getByText("Nguyễn Văn A")).toBeInTheDocument();
    expect(screen.getByText(/Khách nước ngoài \(XML\)/i)).toBeInTheDocument();
    expect(screen.getByText(/Khách Việt Nam \(XLSX\)/i)).toBeInTheDocument();
  });

  it("shows blocking error codes on the row", async () => {
    mockCommands({
      kbtt_pending_rows: [row({ link_id: "l1" })],
      kbtt_validate: [
        {
          code: "E04",
          severity: "blocking",
          link_id: "l1",
          field: "full_name",
          message: "Chưa ai xác nhận tên",
        },
      ],
    });

    render(<PendingList />);

    expect(await screen.findByText("E04")).toBeInTheDocument();
    expect(screen.getByTitle(/Chưa ai xác nhận tên/i)).toBeInTheDocument();
  });

  /// Ghép nhầm phòng hoặc ghép trùng thì phải gỡ được ngay trên dòng đó —
  /// không có nó, một dòng thừa chặn E14 và cả lô không xuất được.
  it("removes a wrong declaration straight from its row", async () => {
    mockCommands({
      kbtt_pending_rows: [row({ link_id: "l1", full_name: "Phạm Thị Minh Hiền" })],
      kbtt_unlink: null,
    });

    render(<PendingList />);

    const remove = await screen.findByLabelText(
      /Gỡ khai báo của Phạm Thị Minh Hiền/i,
    );
    fireEvent.click(remove);

    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_unlink", {
        linkId: "l1",
      });
    });
  });

  it("reports the selected link ids upward", async () => {
    mockCommands({
      kbtt_pending_rows: [row({ link_id: "l1", nationality_iso3: "VNM" })],
    });
    const onSelectionChange = vi.fn();

    render(<PendingList onSelectionChange={onSelectionChange} />);

    await screen.findByText("Nguyễn Văn A");
    await waitFor(() => {
      expect(onSelectionChange).toHaveBeenCalledWith(["l1"]);
    });

    fireEvent.click(screen.getByLabelText(/Chọn Nguyễn Văn A/i));
    await waitFor(() => {
      expect(onSelectionChange).toHaveBeenCalledWith([]);
    });
  });
});
