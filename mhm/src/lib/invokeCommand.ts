import { invoke } from "@tauri-apps/api/core";

import { createAppErrorException, normalizeAppError } from "./appError";

export async function invokeCommand<TResponse>(
  command: string,
  args?: Record<string, unknown>,
): Promise<TResponse> {
  try {
    return await invoke<TResponse>(command, args);
  } catch (error) {
    throw createAppErrorException(normalizeAppError(error), error);
  }
}
