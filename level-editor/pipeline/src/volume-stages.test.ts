import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { NodeIO } from "@gltf-transform/core";
import { ALL_EXTENSIONS } from "@gltf-transform/extensions";
import { type MapCamera, type SightObstacle } from "@rle/shared";
import { buildGeometry } from "./volume-geometry.ts";
import { rasterOwners } from "./volume-raster.ts";
import { buildTextures, synthesisOptions } from "./volume-fill.ts";
import { exportGlb } from "./volume-export.ts";

const cam: MapCamera = { kind: "oblique-orthographic", elevation_deg: 35 };
function obstacle(x: number, opaque: boolean): SightObstacle {
  return {
    points: [
      [x, 12],
      [x + 5, 12],
      [x + 5, 20],
      [x, 20],
    ].map(([x, y]) => ({ x: x!, y: y!, z_bottom: 0, z_top: 4 })),
    opaque,
    solid: true,
    mouse: true,
    show_shadow_polygon: false,
    default_material: 0,
    material_indices: [],
    projection_area: {},
  };
}
test("geometry, ownership and fill stages are deterministic on an asymmetric small scene", async (t) => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "volume-stages-test-"));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  const level = { sight_obstacles: [obstacle(3, false), obstacle(15, true)] };
  const build = () => buildGeometry(level, cam, false, false);
  const g = build();
  assert.deepEqual(g, build());
  assert.ok(g.faces.length > 0);
  const own = rasterOwners(g, cam, 32, 32);
  assert.deepEqual(own, rasterOwners(g, cam, 32, 32));
  assert.ok(own.owner.some((id) => id >= 0));
  const pixels = Buffer.from(
    Array.from({ length: 32 * 32 * 3 }, (_, i) => i % 251),
  );
  const textures = await buildTextures(
    g,
    own,
    cam,
    pixels,
    32,
    32,
    "none",
    dir,
  );
  assert.deepEqual(
    textures,
    await buildTextures(g, own, cam, pixels, 32, 32, "none", dir),
  );
  const exportNames = async (opaqueOnly: boolean) => {
    const geometry = buildGeometry(level, cam, opaqueOnly, false);
    const owners = rasterOwners(geometry, cam, 32, 32);
    const tex = await buildTextures(
      geometry,
      owners,
      cam,
      pixels,
      32,
      32,
      "none",
      dir,
    );
    const file = path.join(dir, `${opaqueOnly}.glb`);
    await exportGlb(file, geometry, tex, cam, [32, 32], "none");
    return (await new NodeIO().registerExtensions(ALL_EXTENSIONS).read(file))
      .getRoot()
      .listNodes()
      .map((node) => node.getName());
  };
  const all = await exportNames(false);
  const opaque = await exportNames(true);
  assert.ok(all.includes("building-000"));
  assert.ok(all.includes("building-001"));
  assert.ok(!opaque.includes("building-000"));
  assert.ok(opaque.includes("building-001"));
});
test("invalid synthesis concurrency fails before launching subprocesses", () => {
  for (const synthJobs of [0, -1, NaN, 1.5])
    assert.throws(() => synthesisOptions({ synthJobs }));
});
