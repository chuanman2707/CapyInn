import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { ExtractedIdentity } from "@/types";

import IdentityCard from "./IdentityCard";

const mrzExtract: ExtractedIdentity = {
  source: "mrz_td3",
  confidence: "verified",
  identity: {
    id: "i1",
    full_name: "ZOLOCHEVSKAIA VERONIKA",
    dob: "1990-03-08",
    gender: "F",
    nationality_iso3: "RUS",
    passport_no: "777785671",
    passport_expiry: "2035-12-09",
    name_confirmed_by_human: false,
  },
  review_hints: ["full_name"],
  crop_data_url: "data:image/png;base64,AAAA",
};

const vnExtract: ExtractedIdentity = {
  source: "qr_cccd",
  confidence: "verified",
  identity: {
    id: "i2",
    full_name: "Nguyễn Văn A",
    dob: "1980-05-02",
    gender: "M",
    nationality_iso3: "VNM",
    doc_no: "058195006173",
    name_confirmed_by_human: false,
  },
  review_hints: [],
  crop_data_url: null,
};

describe("IdentityCard", () => {
  it("shows the MRZ crop right next to the name field", () => {
    render(<IdentityCard extracted={mrzExtract} onChange={vi.fn()} />);

    const crop = screen.getByAltText(/hai dòng MRZ/i);
    const nameInput = screen.getByLabelText(/Họ và tên/i);

    // Cùng một khối để mắt đối chiếu trong một giây — E04 mất ý nghĩa nếu
    // người dùng phải cuộn hay bấm mở mới thấy.
    const block = crop.closest("[data-mrz-review]");
    expect(block).not.toBeNull();
    expect(nameInput.closest("[data-mrz-review]")).toBe(block);
  });

  it("keeps the MRZ crop out of any collapsed or hidden container", () => {
    render(<IdentityCard extracted={mrzExtract} onChange={vi.fn()} />);

    const crop = screen.getByAltText(/hai dòng MRZ/i);
    expect(crop.closest("details")).toBeNull();
    expect(crop.closest("[hidden]")).toBeNull();
    expect(crop).toBeVisible();
  });

  it("requires an explicit click to confirm the name", () => {
    const onChange = vi.fn();
    render(<IdentityCard extracted={mrzExtract} onChange={onChange} />);

    expect(screen.getByText(/Chưa xác nhận tên/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Xác nhận tên/i }));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ name_confirmed_by_human: true }),
    );
  });

  it("never pre-confirms the name even at full checksum", () => {
    render(<IdentityCard extracted={mrzExtract} onChange={vi.fn()} />);

    // confidence là "verified" nhưng tên vẫn phải chờ người
    expect(screen.getByText(/Chưa xác nhận tên/i)).toBeInTheDocument();
  });

  it("offers the single-token override for mononyms", () => {
    const mono: ExtractedIdentity = {
      ...mrzExtract,
      identity: { ...mrzExtract.identity, full_name: "SUHARTO" },
    };
    const onChange = vi.fn();
    render(<IdentityCard extracted={mono} onChange={onChange} />);

    fireEvent.click(screen.getByLabelText(/chỉ có một chữ/i));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ single_token_name_ok: true }),
    );
  });

  it("asks for the residence deadline on foreign guests", () => {
    render(<IdentityCard extracted={mrzExtract} onChange={vi.fn()} />);

    expect(screen.getByLabelText(/Thời hạn tạm trú/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Không phải ngày hết hạn hộ chiếu/i),
    ).toBeInTheDocument();
  });

  it("hides the residence deadline for Vietnamese guests", () => {
    render(<IdentityCard extracted={vnExtract} onChange={vi.fn()} />);

    expect(screen.queryByLabelText(/Thời hạn tạm trú/i)).toBeNull();
  });

  it("edits the name through onChange", () => {
    const onChange = vi.fn();
    render(<IdentityCard extracted={mrzExtract} onChange={onChange} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "ZOLOCHEVSKAIA VERONIKA A" },
    });

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ full_name: "ZOLOCHEVSKAIA VERONIKA A" }),
    );
  });
});
