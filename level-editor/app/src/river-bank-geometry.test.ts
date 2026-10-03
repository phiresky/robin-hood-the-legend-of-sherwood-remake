import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { parseLevel3D, createTerrainGrid, type Level3D, type LevelSpline } from "@rle/shared";
import { editRiverBanks, riverBankAt } from "../../shared/src/river-banks.ts";
import { riverBankGeometry, riverBankTexture } from "./river-bank-geometry.ts";
import { riverMesh } from "./spline-geometry.ts";
import { insertSplinePoint } from "./spline-insertion.ts";
import { SplineLayer } from "./spline-layer.ts";
import { disposeObjectResources } from "./resources.ts";

const camera = { kind: "oblique-orthographic" as const, elevation_deg: 35 };
const base: LevelSpline = {
  id: "river",
  name: "River",
  kind: "river",
  closed: false,
  curved: false,
  points: [
    [0, 0, 0],
    [200, 0, 0],
    [400, 0, 0],
  ],
  width: 80,
  repeatLength: 100,
};
const river: LevelSpline = {
  ...base,
  pointBanks: editRiverBanks(base, [0, 1, 2], "both", { width: 32, mix: { small_stones: 1 } }),
};
const document: Level3D = {
  version: 1,
  map: "Test",
  sceneAssets: [],
  size: [1000, 1000],
  camera,
  objects: [],
  groups: [],
  splines: [river],
};

test("bank widths are independent of water widths and the two banks stay on opposite sides", () => {
  for (const width of [40, 200])
    for (const side of ["left", "right"] as const) {
      const geometry = riverBankGeometry({ ...river, width }, camera, side);
      const p = geometry.getAttribute("position");
      const ys = Array.from({ length: p.count }, (_, i) => p.getY(i));
      assert.ok(Math.abs(Math.max(...ys) - Math.min(...ys) - 32) < 0.001);
      assert.equal(Math.sign(ys[0]!), side === "left" ? 1 : -1);
      assert.ok(Math.abs(Math.min(...ys.map(Math.abs)) - (width / 2 - 6.4)) < 0.001);
      geometry.dispose();
    }
});
test("point and section edits preserve the opposite bank and insertion preserves transitions", () => {
  const path = {
    ...river,
    pointBanks: editRiverBanks(river, [1, 2], "left", { width: 64, mix: { vegetation: 1 } }),
  };
  assert.deepEqual(path.pointBanks[0], river.pointBanks![0]);
  assert.deepEqual(path.pointBanks[1]!.right, river.pointBanks![1]!.right);
  const inserted = insertSplinePoint(path, 0, 0.25);
  assert.deepEqual(inserted.pointBanks![1], riverBankAt(path, 0.125));
  assert.equal(inserted.pointBanks![1]!.left.width, 40);
  assert.deepEqual(inserted.pointBanks![1]!.left.mix, { small_stones: 0.75, vegetation: 0.25 });
  const closed = { ...path, closed: true };
  assert.deepEqual(insertSplinePoint(closed, 2, 0.5).pointBanks![3], riverBankAt(closed, 2.5 / 3));
});
test("bank designs round-trip and reject malformed data without affecting old documents", () => {
  assert.deepEqual(
    parseLevel3D(JSON.parse(JSON.stringify(document))).splines![0]!.pointBanks,
    river.pointBanks,
  );
  assert.doesNotThrow(() => parseLevel3D({ ...document, splines: [base] }));
  for (const patch of [
    { width: 0 },
    { width: Infinity },
    { mix: { imaginary: 1 } },
    { mix: { plain: 0.2 } },
    { mix: { plain: -1, vegetation: 2 } },
  ]) {
    const malformed = {
      ...river,
      pointBanks: river.pointBanks!.map((p) => ({ ...p, left: { ...p.left, ...patch } })),
    };
    assert.throws(() => parseLevel3D({ ...document, splines: [malformed] }));
  }
  assert.throws(() => parseLevel3D({ ...document, splines: [{ ...river, pointBanks: [] }] }));
  assert.throws(() => parseLevel3D({ ...document, splines: [{ ...river, kind: "road" }] }));
});
test("bank art is deterministic, feathered, side-varied, and independent of water width", () => {
  const a = riverBankTexture(river, camera, "left"),
    b = riverBankTexture({ ...river, width: 220 }, camera, "left");
  const right = riverBankTexture(river, camera, "right");
  assert.deepEqual(a.image.data, b.image.data);
  assert.notDeepEqual(a.image.data, right.image.data);
  const pixels = a.image.data,
    width = a.image.width,
    height = a.image.height;
  for (let row = 0; row < height; row++) {
    assert.equal(pixels[row * width * 4 + 3], 0);
    assert.equal(pixels[(row * width + width - 1) * 4 + 3], 0);
  }
  assert.ok(pixels.some((v, i) => i % 4 === 3 && v === 255));
  const none = riverBankTexture(
    { ...river, pointBanks: editRiverBanks(river, [0, 1, 2], "left", { mix: { none: 1 } }) },
    camera,
    "left",
  );
  assert.ok(none.image.data.every((v, i) => i % 4 !== 3 || v === 0));
  for (const texture of [a, b, right, none]) texture.dispose();
});
test("banks drape across terrain ridges and rebuild with terrain edits, including bake objects", () => {
  const terrain = createTerrainGrid([-100, -100, 600, 200], 50);
  for (const v of terrain.vertices) v.position[2] = v.position[0] === 200 ? 60 : 10;
  const uncarved = { ...river, channel: { enabled: false, bedDepth: 24, bankSlope: 1 } };
  const doc = { ...document, terrain, splines: [uncarved] };
  const layer = new SplineLayer();
  layer.sync([uncarved], camera, new Map(), doc);
  const original = layer.bakeObjects()[0]!;
  const bank = original.getObjectByName("riverbank:left") as THREE.Mesh;
  assert.ok(bank);
  const p = bank.geometry.getAttribute("position"),
    cosine = Math.cos((35 * Math.PI) / 180);
  assert.ok(
    Array.from({ length: p.count }, (_, i) => p.getZ(i)).some(
      (z) => Math.abs(z - (60 / cosine + 0.8)) < 0.001,
    ),
  );
  const changed = {
    ...doc,
    terrain: {
      ...terrain,
      vertices: terrain.vertices.map((v) => ({
        ...v,
        position: [v.position[0], v.position[1], 30] as [number, number, number],
      })),
    },
  };
  let disposed = 0;
  bank.geometry.addEventListener("dispose", () => disposed++);
  layer.sync([uncarved], camera, new Map(), changed);
  assert.notEqual(layer.bakeObjects()[0], original);
  assert.equal(disposed, 1);
  layer.clear();
  const plain = riverMesh(base, camera);
  assert.equal(plain.children.length, 0);
  disposeObjectResources([plain]);
});
