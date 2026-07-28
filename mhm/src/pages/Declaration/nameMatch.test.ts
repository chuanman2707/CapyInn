import { describe, expect, it } from "vitest";

import { nameScore, normaliseName } from "./nameMatch";

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
