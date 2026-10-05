import { safeLibraryPath } from "./projection-assets.ts";

export interface NativeImageResource {
  path: string;
  sha256: string;
  width: number;
  height: number;
}
export interface NativePresentationElement {
  id: string;
  source: { kind: "map-animation" | "mission-target"; index: number; sha256: string };
  active: boolean;
  /** Empty frames retain an ordering boundary without claiming its appearance. */
  frames: (NativeImageResource & { offset: [number, number]; delay: number })[];
  loop: boolean;
  display_position: [number, number];
  sort_position: [number, number];
  display_order: number;
  creation_order: number;
  polyline: [number, number][];
}
/** Controlled source-art preview. Actors, scripts and physical depth are not represented. */
export interface NativeStatePresentationContract {
  version: 1;
  mission: string;
  mission_data_sha256: string;
  level_data_sha256: string;
  camera_elevation_deg: number;
  scope: "map-art-and-listed-effects";
  background: NativeImageResource;
  origin: [number, number];
  elements: NativePresentationElement[];
}

export function validateNativeStatePresentation(
  value: unknown,
): asserts value is NativeStatePresentationContract {
  const fail = (message: string): never => {
    throw new Error(`Native artwork preview: ${message}`);
  };
  const uint = (v: unknown) => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
  const hash = (v: unknown) => typeof v === "string" && /^[a-f0-9]{64}$/.test(v);
  const pair = (v: unknown) => Array.isArray(v) && v.length === 2 && v.every(Number.isFinite);
  const resource = (r: NativeImageResource) =>
    !!r &&
    safeLibraryPath(r.path) &&
    hash(r.sha256) &&
    uint(r.width) &&
    r.width > 0 &&
    uint(r.height) &&
    r.height > 0 &&
    r.width * r.height <= 67108864;
  if (!value || typeof value !== "object") fail("missing contract");
  const c = value as NativeStatePresentationContract;
  if (
    c.version !== 1 ||
    typeof c.mission !== "string" ||
    !c.mission ||
    !hash(c.mission_data_sha256) ||
    !hash(c.level_data_sha256) ||
    !Number.isFinite(c.camera_elevation_deg) ||
    c.camera_elevation_deg <= 0 ||
    c.camera_elevation_deg >= 90 ||
    c.scope !== "map-art-and-listed-effects" ||
    !resource(c.background) ||
    !pair(c.origin) ||
    !c.origin.every(Number.isSafeInteger) ||
    !Array.isArray(c.elements)
  )
    fail("invalid source binding");
  const ids = new Set<string>(),
    sources = new Set<string>(),
    orders = new Set<number>();
  for (const e of c.elements) {
    if (
      !e ||
      typeof e.id !== "string" ||
      !e.id ||
      ids.has(e.id) ||
      !e.source ||
      !["map-animation", "mission-target"].includes(e.source.kind) ||
      !uint(e.source.index) ||
      !hash(e.source.sha256)
    )
      fail("invalid or duplicate element");
    const key = `${e.source.kind}:${e.source.index}`;
    if (sources.has(key) || !uint(e.creation_order) || orders.has(e.creation_order))
      fail("duplicate source or creation order");
    ids.add(e.id);
    sources.add(key);
    orders.add(e.creation_order);
    if (
      typeof e.active !== "boolean" ||
      typeof e.loop !== "boolean" ||
      !pair(e.display_position) ||
      !pair(e.sort_position) ||
      !Number.isFinite(e.display_order) ||
      !Array.isArray(e.polyline) ||
      (e.polyline.length > 0 && e.polyline.length < 2) ||
      !e.polyline.every(pair) ||
      !Array.isArray(e.frames)
    )
      fail("invalid element placement");
    for (let i = 1; i < e.polyline.length; i++)
      if (e.polyline[i]![0] <= e.polyline[i - 1]![0]) fail("display polyline must increase in X");
    let duration = 0;
    for (const frame of e.frames) {
      if (
        !resource(frame) ||
        !pair(frame.offset) ||
        !frame.offset.every(Number.isSafeInteger) ||
        !uint(frame.delay)
      )
        fail("invalid frame");
      duration += frame.delay + 1;
      if (!Number.isSafeInteger(duration)) fail("invalid frame duration");
    }
  }
}

export function nativePresentationFrame(element: NativePresentationElement, tick: number): number {
  if (!Number.isSafeInteger(tick) || tick < 0) throw new Error("Invalid native preview tick");
  if (!element.frames.length) return -1;
  const duration = element.frames.reduce((sum, frame) => sum + frame.delay + 1, 0);
  let remaining = element.loop ? tick % duration : Math.min(tick, duration - 1);
  for (let index = 0; index < element.frames.length; index++) {
    const length = element.frames[index]!.delay + 1;
    if (remaining < length) return index;
    remaining -= length;
  }
  throw new Error("Native preview frame duration is inconsistent");
}

export function nativeElementBehind(polyline: [number, number][], point: [number, number]) {
  if (polyline.length < 2) throw new Error("Missing native display polyline");
  const [x, y] = point,
    first = polyline[0]!,
    last = polyline.at(-1)!;
  if (x < first[0]) return y < first[1];
  if (x > last[0]) return y < last[1];
  for (let i = 1; i < polyline.length; i++) {
    const a = polyline[i - 1]!,
      b = polyline[i]!;
    if (b[0] >= x) return (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]) < 0;
  }
  throw new Error("Invalid native display polyline");
}

export function nativePresentationOrder(elements: readonly NativePresentationElement[]) {
  const nonAnimations = elements
    .filter((e) => e.active && !e.polyline.length)
    .sort((a, b) => a.display_order - b.display_order || a.creation_order - b.creation_order);
  const animations = elements
    .filter((e) => e.active && e.polyline.length)
    .sort(
      (a, b) => Math.min(...a.polyline.map((p) => p[1])) - Math.min(...b.polyline.map((p) => p[1])),
    );
  const merge = (ordered: NativePresentationElement[]) => {
    const remaining = [...nonAnimations],
      result: NativePresentationElement[] = [];
    for (const animation of ordered) {
      for (let i = 0; i < remaining.length;) {
        if (nativeElementBehind(animation.polyline, remaining[i]!.sort_position))
          result.push(remaining.splice(i, 1)[0]!);
        else i++;
      }
      result.push(animation);
    }
    return result.concat(remaining);
  };
  const permutations = (group: NativePresentationElement[]): NativePresentationElement[][] => {
    if (group.length > 4) throw new Error("Unproven native animation ordering tie");
    if (!group.length) return [[]];
    return group.flatMap((entry, index) =>
      permutations(group.filter((_, i) => i !== index)).map((rest) => [entry, ...rest]),
    );
  };
  let variants: NativePresentationElement[][] = [[]];
  for (let start = 0; start < animations.length;) {
    const minimum = Math.min(...animations[start]!.polyline.map((p) => p[1]));
    let end = start + 1;
    while (
      end < animations.length &&
      Math.min(...animations[end]!.polyline.map((p) => p[1])) === minimum
    )
      end++;
    variants = variants.flatMap((prefix) =>
      permutations(animations.slice(start, end)).map((group) => prefix.concat(group)),
    );
    if (variants.length > 64) throw new Error("Unproven native animation ordering ties");
    start = end;
  }
  const result = merge(variants[0]!),
    visible = result
      .filter((e) => e.frames.length)
      .map((e) => e.id)
      .join("\0");
  // An unspecified tie is safe only if every possible order yields identical painted ordering.
  for (const variant of variants.slice(1))
    if (
      merge(variant)
        .filter((e) => e.frames.length)
        .map((e) => e.id)
        .join("\0") !== visible
    )
      throw new Error("Ambiguous native animation ordering tie");
  return result;
}
