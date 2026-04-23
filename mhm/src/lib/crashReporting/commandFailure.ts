import { invoke } from "@tauri-apps/api/core";

import type { AppError } from "../appError";
import {
  hasRemoteCrashReporting,
  submitCommandFailureEvent,
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

export interface CaptureCommandFailureInput {
  command: string;
  appError: AppError;
  correlationId?: string | null;
  monitoringContext?: MonitoringContext | null;
}

export async function captureCommandFailure(
  input: CaptureCommandFailureInput,
): Promise<void> {
  const { command, appError, correlationId, monitoringContext } = input;
  if (!isMonitoredCommand(command)) {
    return;
  }

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

  await submitCommandFailureEvent({
    command,
    appError,
    correlationId,
    monitoringContext,
  });
}
