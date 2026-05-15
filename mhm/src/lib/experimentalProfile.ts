import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type BackendExperimentalRuntimeStatus = {
  experimental_runtime_enabled: boolean;
  gateway_runtime_enabled: boolean;
  agent_runtime_enabled: boolean;
  gateway_disabled_by_override: boolean;
  agent_disabled_by_override: boolean;
};

const BACKEND_EXPERIMENTAL_RUNTIME_STATUS_KEYS = [
  "experimental_runtime_enabled",
  "gateway_runtime_enabled",
  "agent_runtime_enabled",
  "gateway_disabled_by_override",
  "agent_disabled_by_override",
] as const;

export type ExperimentalRuntimeStatus = {
  experimentalRuntimeEnabled: boolean;
  gatewayRuntimeEnabled: boolean;
  agentRuntimeEnabled: boolean;
  gatewayDisabledByOverride: boolean;
  agentDisabledByOverride: boolean;
};

export const DISABLED_EXPERIMENTAL_RUNTIME_STATUS: ExperimentalRuntimeStatus = {
  experimentalRuntimeEnabled: false,
  gatewayRuntimeEnabled: false,
  agentRuntimeEnabled: false,
  gatewayDisabledByOverride: false,
  agentDisabledByOverride: false,
};

function normalizeExperimentalRuntimeStatus(
  status: unknown,
): ExperimentalRuntimeStatus {
  if (!isBackendExperimentalRuntimeStatus(status)) {
    return DISABLED_EXPERIMENTAL_RUNTIME_STATUS;
  }

  return {
    experimentalRuntimeEnabled: status.experimental_runtime_enabled,
    gatewayRuntimeEnabled: status.gateway_runtime_enabled,
    agentRuntimeEnabled: status.agent_runtime_enabled,
    gatewayDisabledByOverride: status.gateway_disabled_by_override,
    agentDisabledByOverride: status.agent_disabled_by_override,
  };
}

function isBackendExperimentalRuntimeStatus(
  status: unknown,
): status is BackendExperimentalRuntimeStatus {
  if (!isRecord(status)) {
    return false;
  }

  return BACKEND_EXPERIMENTAL_RUNTIME_STATUS_KEYS.every(
    (key) => typeof status[key] === "boolean",
  );
}

function isRecord(status: unknown): status is Record<string, unknown> {
  return (
    typeof status === "object" &&
    status !== null
  );
}

export async function fetchExperimentalRuntimeStatus(): Promise<ExperimentalRuntimeStatus> {
  try {
    const status = await invoke<unknown>("get_experimental_runtime_status");
    return normalizeExperimentalRuntimeStatus(status);
  } catch {
    return DISABLED_EXPERIMENTAL_RUNTIME_STATUS;
  }
}

export function useExperimentalRuntimeStatus(enabled: boolean): ExperimentalRuntimeStatus {
  const [status, setStatus] = useState<ExperimentalRuntimeStatus>(
    DISABLED_EXPERIMENTAL_RUNTIME_STATUS,
  );

  useEffect(() => {
    let cancelled = false;

    if (!enabled) {
      setStatus(DISABLED_EXPERIMENTAL_RUNTIME_STATUS);
      return () => {
        cancelled = true;
      };
    }

    fetchExperimentalRuntimeStatus().then((nextStatus) => {
      if (!cancelled) {
        setStatus(nextStatus);
      }
    });

    return () => {
      cancelled = true;
    };
  }, [enabled]);

  return status;
}
