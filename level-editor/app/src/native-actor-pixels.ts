import type { Mask } from "../../shared/src/level.ts";
import type { NativePixels } from "./native-state-presentation.ts";
import type { NativeShadowKey } from "../../shared/src/native-state-presentation.ts";

export interface ActorMaskSnapshot {
  id: string;
  mask: Mask;
  active: boolean;
}
export interface ActorMaskQuery {
  layer: number;
  mapPosition: [number, number];
  screenOrigin: [number, number];
  drawHidden: boolean;
  outlineColor: number;
  depth: 15 | 16;
  shadowKey: number;
  shadowStrength: number;
}
export function decodeActorMask(mask: Mask): Uint8Array {
  const [width, height] = mask.box_size;
  if (
    mask.box_top_left.length !== 2 ||
    !mask.box_top_left.every(Number.isSafeInteger) ||
    !Number.isSafeInteger(width) ||
    !Number.isSafeInteger(height) ||
    width < 1 ||
    height < 1 ||
    width * height > 16777216
  )
    throw new Error("Invalid mask dimensions");
  const data = mask.mask_data,
    pixels = new Uint8Array(width * height);
  let offset = 0;
  for (let y = 0; y < height; y++) {
    const length = data[offset++];
    if (length === undefined) throw new Error("Truncated mask row");
    const end = offset + length;
    let block = 0;
    while (offset < end) {
      const control = data[offset++];
      if (control === undefined || !(control & 127)) throw new Error("Empty mask run");
      const count = control & 127,
        repeated = !!(control & 128),
        value = repeated ? data[offset++] : undefined;
      for (let i = 0; i < count; i++) {
        const byte = repeated ? value : data[offset++];
        if (byte === undefined || offset > end || block >= Math.ceil(width / 8))
          throw new Error("Invalid mask run");
        for (let bit = 0; bit < 8 && block * 8 + bit < width; bit++)
          pixels[y * width + block * 8 + bit] = (byte >> (7 - bit)) & 1;
        block++;
      }
    }
    if (offset !== end) throw new Error("Invalid mask row length");
  }
  if (offset !== data.length) throw new Error("Trailing mask data");
  return pixels;
}
export function actorMaskApplies(
  mask: Mask,
  active: boolean,
  layer: number,
  point: [number, number],
): boolean {
  if (typeof active !== "boolean") throw new Error("Current mask activity is required");
  if (!active || mask.layer !== layer || !(mask.mask_type & 1)) return false;
  const line = mask.character_polyline;
  if (
    !line ||
    line.length < 2 ||
    line.some(
      (p, i) =>
        p.length !== 2 || p.some((v) => !Number.isFinite(v)) || (i > 0 && p[0] <= line[i - 1]![0]),
    )
  )
    throw new Error("Invalid character polyline");
  const [x, y] = point;
  if (!Number.isFinite(x) || !Number.isFinite(y)) throw new Error("Invalid actor map position");
  if (x < line[0]![0] || x > line[line.length - 1]![0]) return false;
  let i = 1;
  while (line[i]![0] < x) i++;
  const a = line[i - 1]!,
    b = line[i]!;
  return a[1] + ((x - a[0]) * (b[1] - a[1])) / (b[0] - a[0]) > y;
}
const pack = (r: number, g: number, b: number, depth: 15 | 16) =>
  depth === 16
    ? ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)
    : ((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3);
const unpack = (value: number, depth: 15 | 16): [number, number, number] => {
  const r = depth === 16 ? (value >> 11) & 31 : (value >> 10) & 31,
    g = depth === 16 ? (value >> 5) & 63 : (value >> 5) & 31,
    b = value & 31;
  return [
    (r << 3) | (r >> 2),
    depth === 16 ? (g << 2) | (g >> 4) : (g << 3) | (g >> 2),
    (b << 3) | (b >> 2),
  ];
};

/** Untouched legacy pixels stay packed through every ordered mask, including shadows. */
export function prepareActorPixels(
  source: NativePixels,
  query: ActorMaskQuery,
  masks: readonly ActorMaskSnapshot[],
) {
  const {
    depth,
    shadowKey,
    outlineColor,
    screenOrigin: [sx, sy],
    drawHidden,
  } = query;
  const transparent = depth === 16 ? 0x07c0 : 0x03e0;
  if (
    ![15, 16].includes(depth) ||
    !Number.isInteger(shadowKey) ||
    shadowKey < 0 ||
    shadowKey >= 2 ** depth ||
    shadowKey === transparent ||
    !Number.isInteger(outlineColor) ||
    outlineColor < 0 ||
    outlineColor >= 2 ** depth ||
    outlineColor === shadowKey ||
    outlineColor === transparent ||
    typeof drawHidden !== "boolean" ||
    !Number.isSafeInteger(sx) ||
    !Number.isSafeInteger(sy) ||
    !Number.isInteger(query.layer) ||
    query.screenOrigin.length !== 2 ||
    query.mapPosition.length !== 2 ||
    !query.mapPosition.every(Number.isFinite) ||
    !Number.isInteger(query.shadowStrength) ||
    query.shadowStrength < 0 ||
    query.shadowStrength > 100
  )
    throw new Error("Explicit current actor mask policy is required");
  if (
    !Number.isSafeInteger(source.width) ||
    !Number.isSafeInteger(source.height) ||
    source.width < 1 ||
    source.height < 1 ||
    source.width * source.height > 16777216 ||
    source.data.length !== source.width * source.height * 4
  )
    throw new Error("Invalid actor source pixels");
  const packed = new Uint16Array(source.width * source.height);
  for (let i = 0; i < packed.length; i++) {
    const r = source.data[i * 4]!,
      g = source.data[i * 4 + 1]!,
      b = source.data[i * 4 + 2]!,
      a = source.data[i * 4 + 3]!;
    if (a !== 0 && a !== 255)
      throw new Error("Filtered actor pixels are not a legacy source frame");
    const raw = pack(r, g, b, 16);
    packed[i] =
      !a || raw === 0x07c0 ? transparent : raw === 0x001f ? shadowKey : pack(r, g, b, depth);
  }
  const applied: { id: string; outlined: number; cleared: number }[] = [];
  const ids = new Set<string>();
  for (const { id, mask, active } of masks) {
    if (!id || ids.has(id)) throw new Error("Duplicate or missing ordered mask identity");
    ids.add(id);
    if (!actorMaskApplies(mask, active, query.layer, query.mapPosition)) continue;
    const [mx, my] = mask.box_top_left,
      [mw, mh] = mask.box_size;
    const x0 = Math.max(sx, mx),
      y0 = Math.max(sy, my),
      x1 = Math.min(sx + source.width, mx + mw),
      y1 = Math.min(sy + source.height, my + mh);
    if (x0 >= x1 || y0 >= y1) continue;
    const bitmap = decodeActorMask(mask);
    let outlined = 0,
      cleared = 0;
    for (let y = y0; y < y1; y++)
      for (let x = x0; x < x1; x++) {
        if (!bitmap[(y - my) * mw + x - mx]) continue;
        const i = (y - sy) * source.width + x - sx;
        const a = packed[i] === shadowKey ? transparent : packed[i]!;
        const next = x < x1 - 1 ? packed[i + 1]! : transparent;
        const b = next === shadowKey ? transparent : next;
        if (drawHidden && x < x1 - 1 && a !== b && (a === transparent || b === transparent)) {
          packed[i] = outlineColor;
          outlined++;
        } else {
          if (packed[i] !== transparent) cleared++;
          packed[i] = transparent;
        }
      }
    applied.push({ id, outlined, cleared });
  }
  const data = new Uint8Array(packed.length * 4);
  let shadowPixels = 0;
  for (let i = 0; i < packed.length; i++) {
    const value = packed[i]!;
    if (value === transparent) continue;
    if (value === shadowKey) shadowPixels++;
    data.set([...unpack(value, depth), 255], i * 4);
  }
  const shadow: NativeShadowKey = {
    rgb: unpack(shadowKey, depth),
    strength_percent: query.shadowStrength,
    pixel_format: depth === 16 ? "rgb565" : "rgb555",
  };
  return {
    pixels: { width: source.width, height: source.height, data },
    shadow,
    applied,
    shadowPixels,
  };
}
