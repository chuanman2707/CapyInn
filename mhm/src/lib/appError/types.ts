export type AppErrorKind = "user" | "system";

export interface AppError {
  code: string;
  message: string;
  kind: AppErrorKind;
  support_id: string | null;
}

export interface AppErrorRegistryEntry {
  code: string;
  kind: AppErrorKind;
  defaultMessage: string;
}

export type NormalizedAppErrorException = Error &
  AppError & { correlation_id?: string | null; cause?: unknown };
