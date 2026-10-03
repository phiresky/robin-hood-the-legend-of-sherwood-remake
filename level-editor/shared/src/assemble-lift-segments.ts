import type { Vec3 } from "./scene.ts";

export interface PlacedLiftSegment {
  id: string;
  type: number;
  direction: number;
  joins: Vec3[];
}

export class UnavailableLiftJoin extends Error {
  readonly segments: string[];
  constructor(message: string, segments: string[]) {
    super(message);
    this.segments = segments;
  }
}

/** Match authored sockets after placement; never bind by scene or source identities. */
export function assembleLiftSegments(segments: PlacedLiftSegment[]) {
  const parent = segments.map((_, i) => i);
  const root = (i: number): number => (parent[i] === i ? i : root(parent[i]!));
  const sockets = segments.flatMap((segment, owner) =>
    segment.joins.map((point) => ({ owner, point })),
  );
  for (const socket of sockets) {
    const matches = sockets.filter(
      (other) =>
        other !== socket && Math.hypot(...socket.point.map((n, i) => n - other.point[i]!)) < 1e-4,
    );
    if (matches.length !== 1 || matches[0]!.owner === socket.owner)
      throw new UnavailableLiftJoin(
        `Lift ${segments[socket.owner]!.id}: join must match exactly one other segment`,
        [...new Set([socket.owner, ...matches.map((match) => match.owner)])].map(
          (owner) => segments[owner]!.id,
        ),
      );
    const other = matches[0]!.owner;
    const a = segments[socket.owner]!,
      b = segments[other]!;
    if (a.type !== b.type || a.direction !== b.direction)
      throw new UnavailableLiftJoin(
        `Lift join ${a.id}/${b.id}: traversal type and direction disagree`,
        [a.id, b.id],
      );
    parent[root(other)] = root(socket.owner);
  }
  const identities = new Map(segments.map((s, i) => [s.id, segments[root(i)]!.id]));
  return { identities, lifts: segments.filter((_, i) => root(i) === i) };
}
