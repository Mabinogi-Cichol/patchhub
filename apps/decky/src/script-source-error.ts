// Decky 3.3 reports backend failures as Python Exception and keeps the
// complete plugin error (including multiline Lua diagnostics) in this field.
export function scriptSourceError(failure: unknown): string {
  const traceback = (failure as { pythonTraceback?: unknown } | null)?.pythonTraceback;
  if (typeof traceback === "string") {
    const marker = "\nException: ";
    const start = traceback.indexOf(marker);
    if (start >= 0) return traceback.slice(start + marker.length).trimEnd();
  }
  return String(failure);
}
