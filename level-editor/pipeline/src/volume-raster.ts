import { sceneToMap, type MapCamera } from "@rle/shared";
import { cross, type Geometry } from "./volume-geometry";
export interface Owners {
  /** face id per map pixel, -1 = ground */
  owner: Int32Array;
  backfacing: Uint8Array;
  /** projected map position per vertex */
  px: Float32Array;
  py: Float32Array;
}

export function rasterOwners(
  g: Geometry,
  cam: MapCamera,
  mapW: number,
  mapH: number,
): Owners {
  const t = (cam.elevation_deg * Math.PI) / 180;
  const forward = [0, Math.cos(t), -Math.sin(t)];
  const nV = g.positions.length / 3;
  const px = new Float32Array(nV);
  const py = new Float32Array(nV);
  const pd = new Float32Array(nV);
  for (let i = 0; i < nV; i++) {
    const p: [number, number, number] = [
      g.positions[i * 3]!,
      g.positions[i * 3 + 1]!,
      g.positions[i * 3 + 2]!,
    ];
    const [x, y] = sceneToMap(cam, p);
    px[i] = x;
    py[i] = y;
    pd[i] = p[1] * forward[1]! + p[2] * forward[2]!; // larger = farther
  }
  const depth = new Float32Array(mapW * mapH).fill(Infinity);
  const owner = new Int32Array(mapW * mapH).fill(-1);
  const backfacing = new Uint8Array(g.faces.length);
  const triCount = g.tris.length / 3;
  for (let tIdx = 0; tIdx < triCount; tIdx++) {
    const ia = g.tris[tIdx * 3]!,
      ib = g.tris[tIdx * 3 + 1]!,
      ic = g.tris[tIdx * 3 + 2]!;
    const pa = [
      g.positions[ia * 3]!,
      g.positions[ia * 3 + 1]!,
      g.positions[ia * 3 + 2]!,
    ];
    const pb = [
      g.positions[ib * 3]!,
      g.positions[ib * 3 + 1]!,
      g.positions[ib * 3 + 2]!,
    ];
    const pc = [
      g.positions[ic * 3]!,
      g.positions[ic * 3 + 1]!,
      g.positions[ic * 3 + 2]!,
    ];
    const nrm = cross(
      [pb[0]! - pa[0]!, pb[1]! - pa[1]!, pb[2]! - pa[2]!],
      [pc[0]! - pa[0]!, pc[1]! - pa[1]!, pc[2]! - pa[2]!],
    );
    const face = g.faceOfTri[tIdx]!;
    if (
      nrm[0]! * forward[0]! + nrm[1]! * forward[1]! + nrm[2]! * forward[2]! >=
      0
    ) {
      backfacing[face] = 1;
      continue;
    }
    const ax = px[ia]!,
      ay = py[ia]!,
      bx = px[ib]!,
      by = py[ib]!,
      cx = px[ic]!,
      cy = py[ic]!;
    const minX = Math.max(0, Math.floor(Math.min(ax, bx, cx)));
    const maxX = Math.min(mapW - 1, Math.ceil(Math.max(ax, bx, cx)));
    const minY = Math.max(0, Math.floor(Math.min(ay, by, cy)));
    const maxY = Math.min(mapH - 1, Math.ceil(Math.max(ay, by, cy)));
    const area = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
    if (Math.abs(area) < 1e-9 || minX > maxX || minY > maxY) continue;
    const inv = 1 / area;
    for (let y = minY; y <= maxY; y++) {
      const yy = y + 0.5;
      for (let x = minX; x <= maxX; x++) {
        const xx = x + 0.5;
        const l0 = ((bx - xx) * (cy - yy) - (cx - xx) * (by - yy)) * inv;
        const l1 = ((cx - xx) * (ay - yy) - (ax - xx) * (cy - yy)) * inv;
        const l2 = 1 - l0 - l1;
        if (l0 < 0 || l1 < 0 || l2 < 0) continue;
        const d = l0 * pd[ia]! + l1 * pd[ib]! + l2 * pd[ic]!;
        const cell = y * mapW + x;
        if (d < depth[cell]!) {
          depth[cell] = d;
          owner[cell] = face;
        }
      }
    }
  }
  return { owner, backfacing, px, py };
}
