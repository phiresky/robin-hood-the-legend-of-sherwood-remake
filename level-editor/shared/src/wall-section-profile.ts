import type { Vec3 } from "./scene.ts";

export interface WallSectionProfile {
  start: number;
  end: number;
  sections: { center: number; width: number }[];
}

/** Measure the rendered asset's thickness independently of its longitudinal bend. */
export function measureWallSections(
  triangles: Iterable<readonly Vec3[]>,
  axis: 0 | 1,
  start: number,
  end: number,
): WallSectionProfile {
  if (!Number.isFinite(start) || !Number.isFinite(end) || end - start <= 0.001)
    throw new Error("Wall source has no length along the selected axis");
  const cross = 1 - axis;
  const count = 64;
  const spans = Array.from({ length: count + 1 }, () => ({ min: Infinity, max: -Infinity }));
  for (const triangle of triangles) {
    if (triangle.length !== 3 || triangle.some((p) => p.some((n) => !Number.isFinite(n))))
      throw new Error("Wall source requires finite triangles");
    const low = Math.min(...triangle.map((p) => p[axis]));
    const high = Math.max(...triangle.map((p) => p[axis]));
    const first = Math.max(0, Math.ceil(((low - start) / (end - start)) * count));
    const last = Math.min(count, Math.floor(((high - start) / (end - start)) * count));
    for (let station = first; station <= last; station++) {
      const coordinate = start + ((end - start) * station) / count;
      const span = spans[station]!;
      for (let edge = 0; edge < 3; edge++) {
        const a = triangle[edge]!,
          b = triangle[(edge + 1) % 3]!;
        const av = a[axis],
          bv = b[axis];
        if (Math.abs(av - coordinate) < 1e-6) {
          span.min = Math.min(span.min, a[cross]!);
          span.max = Math.max(span.max, a[cross]!);
        }
        if ((av < coordinate && bv > coordinate) || (av > coordinate && bv < coordinate)) {
          const value = a[cross]! + ((b[cross]! - a[cross]!) * (coordinate - av)) / (bv - av);
          span.min = Math.min(span.min, value);
          span.max = Math.max(span.max, value);
        }
      }
    }
  }
  const valid = spans
    .map((span, index) => ({ ...span, index }))
    .filter((span) => span.max - span.min > 0.001);
  if (!valid.length) throw new Error("Wall source has no measurable cross-section");
  const sections = spans.map((span, index) => {
    if (span.max - span.min > 0.001)
      return { center: (span.min + span.max) / 2, width: span.max - span.min };
    // Tapered endpoints inherit the nearest nonempty section so repetitions join.
    if (index !== 0 && index !== count)
      throw new Error("Wall source has a gap; trim to a continuous section");
    const adjacent = index === 0 ? valid[0]! : valid.at(-1)!;
    return { center: (adjacent.min + adjacent.max) / 2, width: adjacent.max - adjacent.min };
  });
  return { start, end, sections };
}

export function wallSectionAt(profile: WallSectionProfile, along: number) {
  const sample = Math.min(1, Math.max(0, along)) * (profile.sections.length - 1);
  const first = Math.floor(sample),
    fraction = sample - first;
  const a = profile.sections[first]!,
    b = profile.sections[Math.min(first + 1, profile.sections.length - 1)]!;
  return {
    center: a.center + (b.center - a.center) * fraction,
    width: a.width + (b.width - a.width) * fraction,
  };
}
