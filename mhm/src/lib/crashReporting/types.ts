export interface CrashReportSummary {
  bundle_id: string;
  crash_type: string;
  occurred_at: string;
  app_version: string;
  environment: string;
  platform: string;
  arch: string;
  installation_id: string;
  message: string;
  stacktrace: string[];
  module_hint: string | null;
  attempt_count: number;
}

export interface JsCrashReportInput {
  crash_type: "js_unhandled_error" | "unhandled_rejection";
  message: string;
  stacktrace: string[];
  module_hint: string | null;
}
