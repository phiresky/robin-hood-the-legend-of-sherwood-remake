/** One connection slot's async lifetime, independent of its Solid UI state. */
export function connectionAttempts() {
  let generation = 0;
  let disposed = false;
  return {
    begin(): () => boolean {
      const request = ++generation;
      return () => !disposed && request === generation;
    },
    dispose() {
      disposed = true;
      generation++;
    },
  };
}

/** Allocate before invoking the operation (in particular, before any picker or
 * startup restore await). Rejection and finalization share the same authority. */
export async function connectLatest(
  attempts: ReturnType<typeof connectionAttempts>,
  operation: (current: () => boolean) => Promise<void>,
  failed: (error: unknown) => void,
  finished: () => void = () => {},
): Promise<void> {
  const current = attempts.begin();
  if (!current()) return;
  try {
    await operation(current);
  } catch (error) {
    if (
      current() &&
      !(error instanceof DOMException && error.name === "AbortError")
    )
      failed(error);
  } finally {
    if (current()) finished();
  }
}
