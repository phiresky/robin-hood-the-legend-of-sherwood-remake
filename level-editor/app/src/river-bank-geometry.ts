import * as THREE from "three";
import { unzlibSync } from "fflate";
import type { Level3D, LevelSpline, MapCamera } from "@rle/shared";
import {
  riverBankAt,
  type RiverBankSide,
  type RiverBankStyle,
} from "../../shared/src/river-banks.ts";
import { sampleSpline, splineCurve } from "../../shared/src/spline-sampling.ts";
import { drapeRibbonGeometry } from "./road-geometry.ts";
import tiles from "./terrain-textures/riverbanks.json" with { type: "json" };

const decoded = new Map<string, Uint8Array>();
function pixels(id: Exclude<RiverBankStyle, "none">) {
  let data = decoded.get(id);
  if (!data) {
    const tile = tiles[id],
      palette = Uint8Array.from(atob(tile.palette), (c) => c.charCodeAt(0));
    const indices = unzlibSync(Uint8Array.from(atob(tile.pixelsZlib), (c) => c.charCodeAt(0)));
    data = new Uint8Array(indices.length * 3);
    indices.forEach((index, i) => data!.set(palette.subarray(index * 3, index * 3 + 3), i * 3));
    decoded.set(id, data);
  }
  return data;
}
function phase(path: LevelSpline, side: RiverBankSide) {
  let hash = side === "left" ? 17 : 131;
  for (const c of path.id) hash = (Math.imul(hash, 31) + c.charCodeAt(0)) | 0;
  return (hash >>> 0) % 512;
}

/** Bank width changes the covered ground, never the size of the painted stones. */
export function riverBankTexture(
  path: LevelSpline,
  camera: MapCamera,
  side: RiverBankSide,
  preview = false,
) {
  const curve = splineCurve(path, camera),
    length = curve.getLength();
  const width = preview
    ? 64
    : Math.max(
        32,
        Math.min(512, Math.ceil(Math.max(...path.pointBanks!.map((p) => p[side].width)))),
      );
  const height = Math.max(2, Math.min(preview ? 512 : 4096, Math.ceil(length)));
  const data = new Uint8Array(width * height * 4),
    shift = phase(path, side);
  const period = path.closed ? Math.max(512, Math.round(length / 512) * 512) : length;
  for (let row = 0; row < height; row++) {
    const t = row / (height - 1),
      distance = t * period;
    const bank = riverBankAt(path, curve.getUtoTmapping(t, 0))[side];
    const sources = Object.entries(bank.mix)
      .filter(([id, weight]) => id !== "none" && weight > 0)
      .map(([id, weight]) => ({
        data: pixels(id as Exclude<RiverBankStyle, "none">),
        weight: weight,
      }));
    const wave =
      Math.sin(((distance + shift) * Math.PI * 2) / 128) * 0.035 +
      Math.sin(((distance + shift) * Math.PI * 2) / 32) * 0.02;
    for (let column = 0; column < width; column++) {
      const u = column / (width - 1),
        target = (row * width + column) * 4;
      const feather = Math.max(0, Math.min(1, u / 0.14, (1 - u + wave - 0.055) / 0.24));
      const endFade = path.closed ? 1 : Math.min(1, (t * length) / 12, ((1 - t) * length) / 12);
      let alpha = 0;
      const rgb = [0, 0, 0];
      // Four footprint samples suppress sparkling on long, downsampled reaches.
      for (const source of sources) {
        alpha += source.weight;
        for (let sample = 0; sample < 4; sample++) {
          const x =
            ((Math.floor(u * bank.width + shift + ((sample % 2) * bank.width) / width / 2) % 128) +
              128) %
            128;
          const y =
            ((Math.floor(distance + shift + (Math.floor(sample / 2) * period) / height / 2) % 512) +
              512) %
            512;
          for (let c = 0; c < 3; c++)
            rgb[c]! += (source.data[(y * 128 + x) * 3 + c]! * source.weight) / 4;
        }
      }
      for (let c = 0; c < 3; c++) data[target + c] = alpha ? Math.round(rgb[c]! / alpha) : 0;
      data[target + 3] = Math.round(255 * alpha * feather * endFade);
    }
  }
  const texture = new THREE.DataTexture(data, width, height);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.wrapS = texture.wrapT = THREE.ClampToEdgeWrapping;
  texture.magFilter = THREE.LinearFilter;
  texture.minFilter = THREE.LinearMipmapLinearFilter;
  texture.generateMipmaps = true;
  texture.needsUpdate = true;
  return texture;
}

export function riverBankGeometry(
  path: LevelSpline,
  camera: MapCamera,
  side: RiverBankSide,
  document?: Level3D,
  preview = false,
) {
  const samples = sampleSpline(path, camera, {
    spacing: preview ? 24 : 8,
    maxSamples: preview ? 256 : 4096,
  });
  const sine = Math.sin((camera.elevation_deg * Math.PI) / 180),
    cosine = Math.cos((camera.elevation_deg * Math.PI) / 180);
  const sign = side === "left" ? 1 : -1;
  const sections = path.closed ? path.points.length : path.points.length - 1;
  const length = samples.at(-1)!.distance;
  const pairs = samples.map((sample) => {
    const bank = riverBankAt(path, (sample.section + sample.fraction) / sections)[side];
    const normal = new THREE.Vector3(-sample.tangent.y, sample.tangent.x, 0).normalize();
    return [0, 1].map((u) => {
      const offset = sign * (sample.width / 2 + (u - 0.2) * bank.width) * sample.lateralScale;
      return {
        x: sample.position.x + normal.x * offset,
        y: -(sample.position.y + normal.y * offset) * sine,
        z: sample.position.z * cosine,
        offset: 0,
        u,
        v: length ? sample.distance / length : 0,
      };
    });
  });
  return drapeRibbonGeometry(pairs, camera, document, true);
}

export function addRiverBanks(
  mesh: THREE.Mesh,
  path: LevelSpline,
  camera: MapCamera,
  document?: Level3D,
  preview = false,
) {
  if (path.kind !== "river" || !path.pointBanks) return;
  for (const side of ["left", "right"] as const) {
    if (
      !path.pointBanks.some((p) =>
        Object.entries(p[side].mix).some(([id, w]) => id !== "none" && w > 0),
      )
    )
      continue;
    const texture = riverBankTexture(path, camera, side, preview);
    const bank = new THREE.Mesh(
      riverBankGeometry(path, camera, side, document, preview),
      new THREE.MeshBasicMaterial({
        map: texture,
        side: THREE.DoubleSide,
        transparent: true,
        depthWrite: false,
        polygonOffset: true,
        polygonOffsetFactor: -3,
        polygonOffsetUnits: -3,
      }),
    );
    bank.name = `riverbank:${side}`;
    bank.renderOrder = 2;
    bank.userData.noSunShadow = true;
    mesh.add(bank);
  }
}
