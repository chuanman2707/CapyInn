import { describe, expect, it } from "vitest";

import { matchedExistingDeclarationMessage } from "./saveOutcome";

describe("matchedExistingDeclarationMessage", () => {
  it("nói khách đang nằm trong danh sách chờ khi chưa xuất file", () => {
    expect(matchedExistingDeclarationMessage("pending")).toMatch(/danh sách chờ/i);
  });

  it("nói khách đang chờ đối chiếu khi đã xuất file", () => {
    expect(matchedExistingDeclarationMessage("awaiting_reconciliation")).toMatch(
      /đối chiếu/i,
    );
  });
});
