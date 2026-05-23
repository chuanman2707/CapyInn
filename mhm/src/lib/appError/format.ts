import { normalizeAppError } from "./normalize";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function getLocalCorrelationId(error: unknown): string | null {
  if (!isRecord(error)) {
    return null;
  }

  const { correlation_id } = error;
  return typeof correlation_id === "string" ? correlation_id : null;
}

export function formatAppError(error: unknown): string {
  const normalized = normalizeAppError(error);
  const localCorrelationId = getLocalCorrelationId(error);
  const trackingLine = localCorrelationId ? `\nMã theo dõi: ${localCorrelationId}` : "";

  if (normalized.kind === "user") {
    return `${normalized.message}${trackingLine}`;
  }

  if (normalized.support_id) {
    return `${normalized.message} (Mã hỗ trợ: ${normalized.support_id})${trackingLine}`;
  }

  return `${normalized.message}${trackingLine}`;
}
