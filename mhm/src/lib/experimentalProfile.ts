import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type BackendExperimentalRuntimeStatus = {
  experimental_runtime_enabled?: boolean;
  gateway_runtime_enabled?: boolean;
  agent_runtime_enabled?: boolean;
  gateway_disabled_by_override?: boolean;
  agent_disabled_by_override?: boolean;
};

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
  status: BackendExperimentalRuntimeStatus,
): ExperimentalRuntimeStatus {
  return {
    experimentalRuntimeEnabled: Boolean(status.experimental_runtime_enabled),
    gatewayRuntimeEnabled: Boolean(status.gateway_runtime_enabled),
    agentRuntimeEnabled: Boolean(status.agent_runtime_enabled),
    gatewayDisabledByOverride: Boolean(status.gateway_disabled_by_override),
    agentDisabledByOverride: Boolean(status.agent_disabled_by_override),
  };
}

export async function fetchExperimentalRuntimeStatus(): Promise<ExperimentalRuntimeStatus> {
  try {
    const status = await invoke<BackendExperimentalRuntimeStatus>(
      "get_experimental_runtime_status",
    );
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
