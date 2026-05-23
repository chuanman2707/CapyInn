export function buildInvokePayload(
  args: Record<string, unknown> | undefined,
  correlationId: string | undefined,
): Record<string, unknown> | undefined {
  return correlationId === undefined ? args : { ...(args ?? {}), correlationId };
}
