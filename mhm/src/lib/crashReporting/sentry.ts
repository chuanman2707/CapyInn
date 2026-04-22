import * as Sentry from "@sentry/browser";

import type { MonitoringContext } from "./commandFailure";
import type { CrashReportSummary } from "./types";
import type { AppError } from "../appError";

let initialized = false;

function getSentryDsn(): string {
  return typeof __SENTRY_DSN__ !== "undefined" ? __SENTRY_DSN__ : "";
}

function getSentryRelease(): string {
  return typeof __SENTRY_RELEASE__ !== "undefined" ? __SENTRY_RELEASE__ : "";
}

function getSentryEnvironment(): "development" | "production" {
  return typeof __SENTRY_ENVIRONMENT__ !== "undefined"
    ? __SENTRY_ENVIRONMENT__
    : "development";
}

export function hasRemoteCrashReporting(): boolean {
  return Boolean(getSentryDsn());
}

export interface CommandFailureRemoteEvent {
  command: string;
  appError: AppError;
  correlationId: string;
  monitoringContext: MonitoringContext;
}

function scrubValue(value: string): string {
  return value
    .replace(/CapyInn[\\/][^\s]+/g, "<runtime>/...")
    .replace(/\b\d{9,14}\b/g, "<redacted-number>");
}

export function ensureCrashReportingClient() {
  if (initialized || !hasRemoteCrashReporting()) {
    return;
  }

  Sentry.init({
    dsn: getSentryDsn(),
    release: getSentryRelease(),
    environment: getSentryEnvironment(),
    sendDefaultPii: false,
    beforeSend(event) {
      if (event.message) {
        event.message = scrubValue(event.message);
      }
      return event;
    },
  });

  initialized = true;
}

export async function submitCrashBundle(bundle: CrashReportSummary) {
  ensureCrashReportingClient();
  if (!hasRemoteCrashReporting()) {
    throw new Error("Sentry DSN is not configured");
  }

  Sentry.captureEvent({
    level: "fatal",
    message: scrubValue(bundle.message),
    timestamp: Date.parse(bundle.occurred_at) / 1000,
    tags: {
      crash_type: bundle.crash_type,
      module_hint: bundle.module_hint ?? "unknown",
    },
    contexts: {
      app: {
        app_version: bundle.app_version,
        environment: bundle.environment,
      },
      device: {
        arch: bundle.arch,
        platform: bundle.platform,
      },
    },
    extra: {
      bundle_id: bundle.bundle_id,
      occurred_at: bundle.occurred_at,
      app_version: bundle.app_version,
      environment: bundle.environment,
      platform: bundle.platform,
      arch: bundle.arch,
      installation_id: bundle.installation_id,
      attempt_count: bundle.attempt_count,
      stacktrace: bundle.stacktrace.map((frame) => scrubValue(frame)),
      module_hint: bundle.module_hint,
    },
  });

  await Sentry.flush(2000);
}

export async function submitCommandFailureEvent(event: CommandFailureRemoteEvent) {
  ensureCrashReportingClient();
  if (!hasRemoteCrashReporting()) {
    throw new Error("Sentry DSN is not configured");
  }

  const level = event.appError.kind === "user" ? "warning" : "error";

  Sentry.captureEvent({
    level,
    message: `Command failure: ${event.command} (${event.appError.code})`,
    fingerprint: ["command_failure", event.command, event.appError.code],
    tags: {
      event_type: "command_failure",
      command: event.command,
      code: event.appError.code,
      kind: event.appError.kind,
    },
    extra: {
      command: event.command,
      code: event.appError.code,
      kind: event.appError.kind,
      support_id: event.appError.support_id,
      correlation_id: event.correlationId,
      context: event.monitoringContext,
    },
  });

  await Sentry.flush(2000);
}
