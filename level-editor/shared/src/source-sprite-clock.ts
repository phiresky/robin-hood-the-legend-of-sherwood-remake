/** Frame-local counters are authoritative even when an owner is paused or frozen. */
export interface SourceSpriteClock {
  frame: number;
  count: number;
}

/** Convert one observed animation phase; callers bind the exact source row and phase. */
export function sourceSpriteTick(
  frames: readonly { delay: number }[],
  observed: SourceSpriteClock,
): number {
  if (
    !frames.length ||
    !Number.isSafeInteger(observed.frame) ||
    observed.frame < 0 ||
    observed.frame >= frames.length ||
    !Number.isSafeInteger(observed.count) ||
    observed.count < 0 ||
    observed.count > 65535 ||
    frames.some((f) => !Number.isSafeInteger(f.delay) || f.delay < 0 || f.delay > 65535)
  )
    throw new Error("Invalid source sprite clock or frame table");
  if (observed.count === 65535) {
    if (observed.frame !== 0) throw new Error("Unbound reverse or noninitial sprite reset");
    return 0;
  }
  if (observed.count > frames[observed.frame]!.delay)
    throw new Error("Source sprite counter exceeds its frame delay");
  return (
    frames.slice(0, observed.frame).reduce((tick, f) => tick + f.delay + 1, 0) + observed.count
  );
}
