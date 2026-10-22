import assert from "node:assert/strict";
import { isDeepStrictEqual } from "node:util";

/**
 * Strict deep equality for large compiled data with a bounded failure report.
 * The built-in assertion diff of multi-megabyte descriptors (long graph byte
 * arrays) can exhaust memory before it reports anything.
 */
export function assertSameData(actual: unknown, expected: unknown, limit = 20): void {
  if (isDeepStrictEqual(actual, expected)) return;
  const differences: string[] = [];
  const show = (value: unknown) => {
    const text = JSON.stringify(value) ?? String(value);
    return text.length > 120 ? `${text.slice(0, 117)}...` : text;
  };
  const walk = (a: unknown, b: unknown, path: string) => {
    if (differences.length >= limit || isDeepStrictEqual(a, b)) return;
    if (Array.isArray(a) && Array.isArray(b)) {
      if (a.length !== b.length)
        differences.push(`${path || "/"}: length ${a.length} → expected ${b.length}`);
      for (let i = 0; i < Math.min(a.length, b.length); i++) walk(a[i], b[i], `${path}[${i}]`);
      return;
    }
    if (
      a &&
      b &&
      typeof a === "object" &&
      typeof b === "object" &&
      !Array.isArray(a) &&
      !Array.isArray(b) &&
      Object.getPrototypeOf(a) === Object.getPrototypeOf(b)
    ) {
      const left = a as Record<string, unknown>;
      const right = b as Record<string, unknown>;
      for (const key of new Set([...Object.keys(left), ...Object.keys(right)])) {
        if (!(key in right)) differences.push(`${path}/${key}: unexpected ${show(left[key])}`);
        else if (!(key in left)) differences.push(`${path}/${key}: missing ${show(right[key])}`);
        else walk(left[key], right[key], `${path}/${key}`);
      }
      return;
    }
    differences.push(`${path || "/"}: ${show(a)} → expected ${show(b)}`);
  };
  walk(actual, expected, "");
  assert.fail(
    `Values differ${differences.length >= limit ? ` (first ${limit} differences)` : ""}:\n${
      differences.join("\n") || "(difference not located)"
    }`,
  );
}
