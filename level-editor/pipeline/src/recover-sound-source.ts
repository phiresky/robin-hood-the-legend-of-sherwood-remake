import type { AssetSoundSource } from "../../shared/src/asset-gameplay.ts";
import type { Point, SoundSource } from "../../shared/src/level.ts";
import { distanceToPolygon } from "./recovery-elevation.ts";
import { isDeepStrictEqual } from "node:util";

export interface SoundOwnerDeclaration {
  source: number;
  owner: string;
  node: string;
  reason: string;
  /** Pin the complete source record for this one-time ownership decision. */
  sound: SoundSource;
}

/** Resolve reviewed acoustic ownership without storing source indices in runtime assets. */
export function declaredSoundOwners<T>(
  entries: SoundOwnerDeclaration[],
  sources: SoundSource[],
  frames: (asset: string, node: string) => T[],
  claimed: ReadonlySet<number> = new Set(),
): Map<number, T> {
  const result = new Map<number, T>();
  for (const entry of entries) {
    const source = sources[entry.source];
    if (
      !Number.isInteger(entry.source) ||
      entry.source < 0 ||
      !source ||
      source.global ||
      !entry.owner.trim() ||
      !entry.node.trim() ||
      !entry.reason.trim() ||
      result.has(entry.source) ||
      claimed.has(entry.source)
    )
      throw new Error(
        "Sound ownership needs one unclaimed local source and a reviewed asset frame",
      );
    if (!isDeepStrictEqual(source, entry.sound))
      throw new Error(`Sound ownership source changed: ${entry.source}`);
    const matches = frames(entry.owner, entry.node);
    if (matches.length !== 1)
      throw new Error(`Sound ownership needs one pinned frame: ${entry.owner}:${entry.node}`);
    result.set(entry.source, matches[0]!);
  }
  return result;
}

/** Overlapping parts of one asset share ownership. Pick a stable local frame;
 * separate assets still require an explicit authoring decision. */
export function uniqueSoundOwner<T extends { asset: string; node: string }>(
  candidates: readonly T[],
): T | undefined {
  if (!candidates.length || new Set(candidates.map((owner) => owner.asset)).size !== 1)
    return undefined;
  return [...candidates].sort((a, b) => (a.node < b.node ? -1 : a.node > b.node ? 1 : 0))[0];
}

/** Check segment interiors between boundary crossings of concave footprints. */
export function containsSoundPolyline(points: Point[], boundary: Point[]): boolean {
  if (
    !points.length ||
    boundary.length < 3 ||
    points.some((p) => distanceToPolygon(p, boundary) > 1e-7)
  )
    return false;
  const cross = (a: Point, b: Point) => a[0] * b[1] - a[1] * b[0];
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1]!,
      b = points[i]!,
      direction: Point = [b[0] - a[0], b[1] - a[1]];
    const breaks = [0, 1];
    for (let j = 0; j < boundary.length; j++) {
      const c = boundary[j]!,
        d = boundary[(j + 1) % boundary.length]!;
      const edge: Point = [d[0] - c[0], d[1] - c[1]],
        offset: Point = [c[0] - a[0], c[1] - a[1]];
      const denominator = cross(direction, edge);
      if (Math.abs(denominator) < 1e-10) continue;
      const t = cross(offset, edge) / denominator,
        u = cross(offset, direction) / denominator;
      if (t > 0 && t < 1 && u >= 0 && u <= 1) breaks.push(t);
    }
    breaks.sort((a, b) => a - b);
    for (let j = 1; j < breaks.length; j++) {
      const t = (breaks[j - 1]! + breaks[j]!) / 2;
      if (distanceToPolygon([a[0] + t * direction[0], a[1] + t * direction[1]], boundary) > 1e-7)
        return false;
    }
  }
  return true;
}

/** The caller supplies an explicit owner and converts map points to that part's local frame. */
export function recoverSoundSource(
  raw: SoundSource,
  id: string,
  node: string,
  localize: (point: [number, number, number]) => [number, number, number],
): AssetSoundSource {
  const value: AssetSoundSource = {
    id,
    node,
    sample: raw.id,
    active: raw.active,
    kind: raw.source_kind as AssetSoundSource["kind"],
    altitude: raw.altitude as AssetSoundSource["altitude"],
    ambiences: raw.ambience_filter,
    ...(raw.delayed_params ? { delay: [...raw.delayed_params] } : {}),
  };
  if (!raw.global) {
    if (
      !raw.polyline?.length ||
      [
        raw.inner_distance,
        raw.outer_distance,
        raw.inner_volume,
        raw.outer_volume,
        raw.noise_covering_distance,
      ].some((n) => n === null)
    )
      throw new Error(`Sound ${id} has incomplete spatial data`);
    value.spatial = {
      polyline: raw.polyline.map(([x, y]) => localize([x, y, 0])),
      ...(raw.polyline_breaks?.length ? { polylineBreaks: [...raw.polyline_breaks] } : {}),
      innerDistance: raw.inner_distance!,
      outerDistance: raw.outer_distance!,
      innerVolume: raw.inner_volume!,
      outerVolume: raw.outer_volume!,
      noiseCoveringDistance: raw.noise_covering_distance!,
    };
  }
  return value;
}
