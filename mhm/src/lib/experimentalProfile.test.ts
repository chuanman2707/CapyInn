import { beforeEach, describe, expect, it } from "vitest";

import { clearMockResponses, setMockResponse } from "@/__mocks__/tauri-core";
import {
  DISABLED_EXPERIMENTAL_RUNTIME_STATUS,
  fetchExperimentalRuntimeStatus,
} from "./experimentalProfile";

describe("experimentalProfile", () => {
  beforeEach(() => {
    clearMockResponses();
  });

  it("normalizes backend experimental runtime status", async () => {
    setMockResponse("get_experimental_runtime_status", () => ({
      experimental_runtime_enabled: true,
      gateway_runtime_enabled: true,
      agent_runtime_enabled: false,
      gateway_disabled_by_override: false,
      agent_disabled_by_override: true,
    }));

    await expect(fetchExperimentalRuntimeStatus()).resolves.toEqual({
      experimentalRuntimeEnabled: true,
      gatewayRuntimeEnabled: true,
      agentRuntimeEnabled: false,
      gatewayDisabledByOverride: false,
      agentDisabledByOverride: true,
    });
  });

  it("falls back to disabled profile when status command fails", async () => {
    setMockResponse("get_experimental_runtime_status", () => {
      throw new Error("runtime status unavailable");
    });

    await expect(fetchExperimentalRuntimeStatus()).resolves.toEqual(
      DISABLED_EXPERIMENTAL_RUNTIME_STATUS,
    );
  });
});
