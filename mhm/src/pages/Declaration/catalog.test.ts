import { describe, expect, it } from "vitest";

import { findingText } from "./catalog";

describe("findingText", () => {
  it("dịch mã thành câu tiếng người kèm hướng sửa", () => {
    expect(
      findingText({ code: "E02", severity: "blocking", link_id: "l1", message: "x" }),
    ).toContain("một chữ");
    expect(
      findingText({ code: "W02", severity: "warning", link_id: "l1", message: "x" }),
    ).toContain("điện thoại");
  });

  it("mã lạ rơi về message gốc, không bao giờ rỗng", () => {
    expect(
      findingText({ code: "E99", severity: "blocking", link_id: "l1", message: "Lỗi gốc." }),
    ).toBe("Lỗi gốc.");
  });
});
