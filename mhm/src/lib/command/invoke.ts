import { invoke } from "@tauri-apps/api/core";

import { createAppErrorException, normalizeAppError } from "../appError";
import { captureCommandFailure, type MonitoringContext } from "../crashReporting/commandFailure";
import { createIdempotencyKey } from "./idempotency";
import { buildInvokePayload } from "./payload";

type InvokeCommandOptions = {
  correlationId?: string;
  monitoringContext?: MonitoringContext;
};

export async function invokeCommand<TResponse>(
  command: string,
  args?: Record<string, unknown>,
  options?: InvokeCommandOptions,
): Promise<TResponse> {
  try {
    const payload = buildInvokePayload(args, options?.correlationId);

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
  options?: InvokeCommandOptions,
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
