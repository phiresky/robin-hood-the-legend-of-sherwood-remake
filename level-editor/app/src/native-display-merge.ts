import type { NativePixels } from "./native-state-presentation.ts";

export interface NativeOrderedDraw {
  identity: string;
  rank: number;
  order: number;
  mapPosition: readonly [number, number];
  polyline: readonly (readonly [number, number])[];
  draw: { pixels: NativePixels; x: number; y: number };
}

export function behindDisplayPolyline(
  line: NativeOrderedDraw["polyline"],
  point: NativeOrderedDraw["mapPosition"],
) {
  if (line.length < 2) throw new Error("Display polyline needs two points");
  const [x, y] = point.map(Math.fround),
    first = line[0]!,
    last = line.at(-1)!;
  if (x! < first[0]) return y! < first[1];
  if (x! > last[0]) return y! < last[1];
  let right = 1;
  while (line[right]![0] < x!) right++;
  const a = line[right - 1]!,
    b = line[right]!;
  const cross = Math.fround(
    Math.fround(Math.fround(b[0] - a[0]) * Math.fround(y! - a[1])) -
      Math.fround(Math.fround(b[1] - a[1]) * Math.fround(x! - a[0])),
  );
  return cross < 0;
}

function overlapsAlpha(a: NativeOrderedDraw, b: NativeOrderedDraw) {
  const x0 = Math.max(a.draw.x, b.draw.x),
    y0 = Math.max(a.draw.y, b.draw.y);
  const x1 = Math.min(a.draw.x + a.draw.pixels.width, b.draw.x + b.draw.pixels.width);
  const y1 = Math.min(a.draw.y + a.draw.pixels.height, b.draw.y + b.draw.pixels.height);
  for (let y = y0; y < y1; y++)
    for (let x = x0; x < x1; x++) {
      const ai = ((y - a.draw.y) * a.draw.pixels.width + x - a.draw.x) * 4 + 3;
      const bi = ((y - b.draw.y) * b.draw.pixels.width + x - b.draw.x) * 4 + 3;
      if (a.draw.pixels.data[ai] && b.draw.pixels.data[bi]) return true;
    }
  return false;
}

/** Merge scalar entities against each display polyline. Unspecified ties require pixel equivalence. */
export function mergeNativeDisplay<T extends NativeOrderedDraw>(input: readonly T[]) {
  const scalar: T[] = [],
    animated: T[] = [];
  const identities = new Set<string>();
  for (const row of input) {
    if (
      !row.identity ||
      identities.has(row.identity) ||
      !row.mapPosition.every((n) => Number.isFinite(Math.fround(n))) ||
      !Number.isFinite(Math.fround(row.order)) ||
      !Number.isSafeInteger(row.rank) ||
      ![row.draw.x, row.draw.y].every(Number.isSafeInteger) ||
      ![row.draw.pixels.width, row.draw.pixels.height].every(
        (n) => Number.isSafeInteger(n) && n > 0,
      ) ||
      row.draw.pixels.data.length !== row.draw.pixels.width * row.draw.pixels.height * 4 ||
      row.polyline.some(
        (p, i) =>
          !p.every((n) => Number.isFinite(Math.fround(n))) ||
          (i > 0 && p[0] < row.polyline[i - 1]![0]),
      )
    )
      throw new Error("Invalid current display geometry");
    identities.add(row.identity);
    if (row.polyline.length === 1) throw new Error("Display polyline needs two points");
    (row.polyline.length ? animated : scalar).push(row);
  }
  scalar.sort((a, b) => Math.fround(a.order) - Math.fround(b.order) || a.rank - b.rank);
  const minY = (row: T) => Math.min(...row.polyline.map((p) => Math.fround(p[1])));
  animated.sort((a, b) => minY(a) - minY(b));
  const groups: T[][] = [];
  for (const row of animated) {
    const group = groups.at(-1);
    if (group && minY(group[0]!) === minY(row)) group.push(row);
    else groups.push([row]);
  }
  let variants: T[][] = [[]];
  const permutations = (rows: T[]): T[][] =>
    rows.length === 0
      ? [[]]
      : rows.flatMap((row, i) =>
          permutations(rows.filter((_, j) => j !== i)).map((rest) => [row, ...rest]),
        );
  for (const group of groups) {
    if (group.length > 4)
      throw new Error("Unresolved display tie exceeds bounded equivalence proof");
    const orders = permutations(group);
    if (variants.length * orders.length > 32) throw new Error("Too many unresolved display orders");
    variants = variants.flatMap((prefix) => orders.map((order) => [...prefix, ...order]));
  }
  const merge = (animations: T[]) => {
    let remaining = [...scalar];
    const result: T[] = [];
    for (const animation of animations) {
      const front: T[] = [];
      for (const row of remaining) {
        if (behindDisplayPolyline(animation.polyline, row.mapPosition)) result.push(row);
        else front.push(row);
      }
      remaining = front;
      result.push(animation);
    }
    return [...result, ...remaining];
  };
  const orders = variants.map(merge),
    selected = orders[0]!;
  for (const alternate of orders.slice(1)) {
    const positions = new Map(alternate.map((row, i) => [row.identity, i]));
    for (let i = 0; i < selected.length; i++)
      for (let j = i + 1; j < selected.length; j++) {
        const a = selected[i]!,
          b = selected[j]!;
        if (positions.get(a.identity)! > positions.get(b.identity)! && overlapsAlpha(a, b))
          throw new Error(`Unresolved overlapping display tie: ${a.identity}/${b.identity}`);
      }
  }
  return {
    rows: selected,
    tieProof: { variants: orders.length, changedOrderPairsHaveDisjointAlpha: true },
  };
}
