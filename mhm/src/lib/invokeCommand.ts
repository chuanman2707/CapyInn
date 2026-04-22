import { invoke } from "@tauri-apps/api/core";

import { createAppErrorException, normalizeAppError } from "./appError";

export async function invokeCommand<TResponse>(
  command: string,
  args?: Record<string, unknown>,
  options?: { correlationId?: string },
): Promise<TResponse> {
  try {
    const payload =
      options?.correlationId === undefined
        ? args
        : { ...(args ?? {}), correlationId: options.correlationId };

    return await invoke<TResponse>(command, payload);
  } catch (error) {
    throw createAppErrorException(normalizeAppError(error), error, {
      correlation_id: options?.correlationId,
    });
  }
}
