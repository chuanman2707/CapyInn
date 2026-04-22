import { invoke } from "@tauri-apps/api/core";

import { normalizeAppError, type AppError } from "../appError";
import {
  hasRemoteCrashReporting,
  submitCommandFailureEvent,
  type CommandFailureRemoteEvent,
} from "./sentry";

export type MonitoringContext =
  | {
      guest_count: number;
      nights: number;
      source: string | null;
      notes_present: boolean;
    }
  | {
      settlement_mode: string;
    }
  | {
      nights: number;
      deposit_present: boolean;
      source: string | null;
      notes_present: boolean;
    }
  | {
      notes_present: boolean;
    };

const MONITORED_COMMANDS = new Set([
  "check_in",
  "check_out",
  "create_reservation",
  "run_night_audit",
]);

function isMonitoredCommand(command: string): boolean {
  return MONITORED_COMMANDS.has(command);
}

function hasCorrelationId(value: string | null | undefined): value is string {
  return typeof value === "string" && value.length > 0;
}

function hasMonitoringContext(value: MonitoringContext | null | undefined): value is MonitoringContext {
  return value !== undefined && value !== null;
}

function toRemoteEvent(
  command: string,
  appError: AppError,
  correlationId: string,
  monitoringContext: MonitoringContext,
): CommandFailureRemoteEvent {
  return {
    command,
    code: appError.code,
    kind: appError.kind,
    support_id: appError.support_id,
    correlation_id: correlationId,
    context: monitoringContext,
  };
}

export async function captureCommandFailure(
  command: string,
  error: unknown,
  options?: {
    correlationId?: string | null;
    monitoringContext?: MonitoringContext | null;
  },
): Promise<void> {
  if (!isMonitoredCommand(command)) {
    return;
  }

  const correlationId = options?.correlationId;
  const monitoringContext = options?.monitoringContext;
  if (!hasCorrelationId(correlationId) || !hasMonitoringContext(monitoringContext)) {
    return;
  }

  if (!hasRemoteCrashReporting()) {
    return;
  }

  const crashReportingEnabled = await invoke<boolean>("get_crash_reporting_preference");
  if (!crashReportingEnabled) {
    return;
  }

  await submitCommandFailureEvent(
    toRemoteEvent(command, normalizeAppError(error), correlationId, monitoringContext),
  );
}
