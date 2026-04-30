import { invoke } from "@tauri-apps/api/core";

import { createAppErrorException, normalizeAppError } from "./appError";
import { captureCommandFailure, type MonitoringContext } from "./crashReporting/commandFailure";

export function createIdempotencyKey(command: string): string {
  const random =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return `${command}:${random}`;
}

export async function invokeCommand<TResponse>(
  command: string,
  args?: Record<string, unknown>,
  options?: { correlationId?: string; monitoringContext?: MonitoringContext },
): Promise<TResponse> {
  try {
    const payload =
      options?.correlationId === undefined
        ? args
        : { ...(args ?? {}), correlationId: options.correlationId };

    return await invoke<TResponse>(command, payload);
  } catch (error) {
    const appError = normalizeAppError(error);
    void captureCommandFailure({
      command,
      appError,
      correlationId: options?.correlationId,
      monitoringContext: options?.monitoringContext,
    }).catch(() => {});

    throw createAppErrorException(appError, error, {
      correlation_id: options?.correlationId,
    });
  }
}

export async function invokeWriteCommand<TResponse>(
  command: string,
  args?: Record<string, unknown>,
  options?: { correlationId?: string; monitoringContext?: MonitoringContext },
): Promise<TResponse> {
  return invokeCommand<TResponse>(
    command,
    {
      ...(args ?? {}),
      idempotencyKey: createIdempotencyKey(command),
    },
    options,
  );
}
