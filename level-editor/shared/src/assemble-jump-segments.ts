import type { Vec3 } from "./scene.ts";
import type { AssetJumpSegment } from "./asset-gameplay.ts";
import {
  trimJumpEdges,
  snapJumpEdges,
  type createJumpClearance,
  type Interval,
} from "./jump-clearance.ts";

export interface PlacedJumpSegment {
  id: string;
  long: boolean;
  join?: Vec3;
  attachment?: AssetJumpSegment["attachment"];
  /** Automatic surface ledges exclude the authored inset when deciding whether a gap exists. */
  surfaceInset?: number;
  edge: { zone: string; a: Vec3; b: Vec3 };
}

type Edge = PlacedJumpSegment["edge"];
// Asset coordinates include elevation in Y; matching uses the horizontal map plane.
const project = (p: Vec3) => [p[0], p[1] - p[2]] as const;

/** Facing parallel edges connect only over their shared span, within both assets' limits. */
function geometricEdges(a: PlacedJumpSegment, b: PlacedJumpSegment): [Edge, Edge] | null {
  const ar = a.attachment,
    br = b.attachment;
  if (!ar || !br || a.long !== b.long || a.edge.zone === b.edge.zone) return null;
  const aa = project(a.edge.a),
    ab = project(a.edge.b),
    ba = project(b.edge.a),
    bb = project(b.edge.b);
  const av = [ab[0] - aa[0], ab[1] - aa[1]],
    bv = [bb[0] - ba[0], bb[1] - ba[1]];
  const al = Math.hypot(...av),
    bl = Math.hypot(...bv);
  if (al < 1e-4 || bl < 1e-4 || (av[0]! * bv[0]! + av[1]! * bv[1]!) / (al * bl) > -1 + 1e-8)
    return null;
  const axis = [av[0]! / al, av[1]! / al];
  const along = (p: readonly number[]) => (p[0]! - aa[0]) * axis[0]! + (p[1]! - aa[1]) * axis[1]!;
  const gapAt = (p: readonly number[]) => -(p[0]! - aa[0]) * axis[1]! + (p[1]! - aa[1]) * axis[0]!;
  if ([ba, bb].some((p) => gapAt(p) <= 1e-4 || gapAt(p) > Math.min(ar.maxGap, br.maxGap)))
    return null;
  if (
    a.surfaceInset !== undefined &&
    b.surfaceInset !== undefined &&
    Math.min(gapAt(ba), gapAt(bb)) <= a.surfaceInset + b.surfaceInset + 1e-4
  )
    return null;
  const startB = along(ba),
    endB = along(bb);
  const low = Math.max(0, endB),
    high = Math.min(al, startB);
  if (high - low < Math.max(ar.minOverlap, br.minOverlap)) return null;
  const interpolate = (edge: Edge, t: number): Vec3 =>
    edge.a.map((n, i) => n + t * (edge.b[i]! - n)) as Vec3;
  // Traversal carries distance along the source edge to the opposite edge's B end.
  // Equal, oppositely oriented spans keep every possible takeoff aligned to a landing.
  const left: Edge = {
    zone: a.edge.zone,
    a: interpolate(a.edge, low / al),
    b: interpolate(a.edge, high / al),
  };
  const right: Edge = {
    zone: b.edge.zone,
    a: interpolate(b.edge, (startB - high) / (startB - endB)),
    b: interpolate(b.edge, (startB - low) / (startB - endB)),
  };
  for (const rise of [right.b[2] - left.a[2], right.a[2] - left.b[2]])
    if (rise > Math.min(ar.maxRise, br.maxDrop) || -rise > Math.min(ar.maxDrop, br.maxRise))
      return null;
  return [left, right];
}

function matchingEdges(a: PlacedJumpSegment, b: PlacedJumpSegment): [Edge, Edge] | null {
  if (a.attachment || b.attachment) return geometricEdges(a, b);
  const aj = a.join,
    bj = b.join;
  return aj && bj && Math.hypot(...aj.map((n, i) => n - bj[i]!)) < 1e-4 ? [a.edge, b.edge] : null;
}

/** Rebuild connections from placed geometry; legacy exact sockets remain supported. */
export function assembleJumpSegments(
  segments: PlacedJumpSegment[],
  clearance?: ReturnType<typeof createJumpClearance>,
  obstructionLabel = "solid obstacles obstruct the flight",
) {
  const consumed = new Set<PlacedJumpSegment>();
  const unmatched: PlacedJumpSegment[] = [];
  const pairs: { id: string; long: boolean; edges: Edge[] }[] = [];
  const warnings: string[] = [];
  const candidatesToCompile: {
    segment: PlacedJumpSegment;
    other: PlacedJumpSegment;
    edges: [Edge, Edge];
  }[] = [];
  const emitted = new Set<string>();
  for (const [index, segment] of segments.entries()) {
    if (consumed.has(segment)) continue;
    const candidates = segments.flatMap((other) => {
      if (other === segment) return [];
      const edges = matchingEdges(segment, other);
      return edges ? [{ other, edges }] : [];
    });
    if (!candidates.length) {
      unmatched.push(segment);
      continue;
    }
    if (segment.attachment) {
      for (const { other, edges } of candidates) {
        if (segments.indexOf(other) <= index) continue;
        const key = JSON.stringify([segment.long, edges]);
        if (emitted.has(key)) continue;
        emitted.add(key);
        candidatesToCompile.push({ segment, other, edges });
      }
      continue;
    }
    if (candidates.length !== 1 || consumed.has(candidates[0]!.other))
      throw new Error(`Jump ${segment.id}: join must match exactly one complementary edge`);
    const { other, edges } = candidates[0]!;
    if (segment.long !== other.long)
      throw new Error(`Jump ${segment.id}: paired long-jump rules disagree`);
    if (segment.edge.zone === other.edge.zone)
      throw new Error(`Jump ${segment.id}: both edges use the same landing zone`);
    consumed.add(segment);
    consumed.add(other);
    candidatesToCompile.push({ segment, other, edges });
  }
  for (const { segment, other, edges } of candidatesToCompile) {
    const id = segment.attachment ? `${segment.id}/to/${other.id}` : segment.id;
    if (!segment.attachment || !clearance) {
      pairs.push({ id, long: segment.long, edges });
      continue;
    }
    let blocked: Interval[];
    const body = {
      radius: Math.max(
        segment.attachment.clearance?.radius ?? 0,
        other.attachment!.clearance?.radius ?? 0,
      ),
      height: Math.max(
        segment.attachment.clearance?.height ?? 0,
        other.attachment!.clearance?.height ?? 0,
      ),
    };
    const snapped = snapJumpEdges(edges);
    const length = Math.hypot(snapped[0].b[0] - snapped[0].a[0], snapped[0].b[1] - snapped[0].a[1]);
    try {
      blocked = clearance(snapped, segment.long, body).map(([a, b]) => [
        Math.max(0, a - 1 / length),
        Math.min(1, b + 1 / length),
      ]);
    } catch (error) {
      if (!(error instanceof Error)) throw error;
      warnings.push(`Jump ${segment.id}: connection omitted: ${error.message}.`);
      continue;
    }
    let start = 0;
    const spans: Interval[] = [];
    for (const [low, high] of [...blocked, [1, 1] as Interval]) {
      if (low > start) spans.push([start, low]);
      start = Math.max(start, high);
    }
    const minimum = Math.max(segment.attachment.minOverlap, other.attachment!.minOverlap);
    const usable = spans.filter(([a, b]) => (b - a) * length >= minimum);
    let retained = 0;
    for (const [index, [a, b]] of usable.entries()) {
      const finalEdges = snapJumpEdges(trimJumpEdges(snapped, a, b));
      if (
        Math.hypot(
          finalEdges[0].b[0] - finalEdges[0].a[0],
          finalEdges[0].b[1] - finalEdges[0].a[1],
        ) < minimum
      )
        continue;
      if (clearance(finalEdges, segment.long, body).length) continue;
      pairs.push({
        id: usable.length === 1 ? id : `${id}/span-${index}`,
        long: segment.long,
        edges: finalEdges,
      });
      retained++;
    }
    if (blocked.length)
      warnings.push(
        `Jump ${segment.id}: ${obstructionLabel}; ${retained} usable span(s) retained.`,
      );
    else if (!retained)
      warnings.push(`Jump ${segment.id}: no usable span remains after movement-grid rounding.`);
  }
  return { pairs, unmatched, warnings };
}
