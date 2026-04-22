import { beforeEach, describe, expect, it, vi } from "vitest";

import { createCorrelationId } from "./correlationId";

describe("correlationId", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
  });

  it("creates COR-prefixed uppercase hexadecimal ids from crypto randomness", () => {
    const getRandomValues = vi.fn((array: Uint8Array) => {
      array.set([0x8f, 0x3a, 0x1c, 0x7d]);
      return array;
    });

    vi.stubGlobal("crypto", {
      getRandomValues,
    });

    expect(createCorrelationId()).toBe("COR-8F3A1C7D");
    expect(getRandomValues).toHaveBeenCalledTimes(1);
  });
});
