export function createCorrelationId(): string {
  const bytes = new Uint8Array(4);
  globalThis.crypto.getRandomValues(bytes);

  return `COR-${Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0"))
    .join("")
    .toUpperCase()}`;
}
