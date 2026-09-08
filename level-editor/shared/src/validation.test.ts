import test from "node:test";
import assert from "node:assert/strict";
import { parseLevel3D, parseProtoLevel } from "./validation.ts";
import {
  applyAffineMatrix,
  signedPolygonArea,
  sceneToGame,
  sceneToGltf,
  gltfToScene,
} from "./geometry.ts";
import { gameToScene, type MapCamera } from "./scene.ts";
import { gameTransformMatrix, transformedObstacle } from "./level3d.ts";

const camera: MapCamera = { kind: "oblique-orthographic", elevation_deg: 35 };
function level() {
  return {
    format: "Fullgame",
    misc: {},
    sight_obstacles: [document().objects[0]!.obstacle],
    patches: [],
    animations: [],
    material_sectors: [],
    light_sectors: [],
    elevation_lines: [],
    masks: [],
    sound_sources: [],
    jump_zones: [],
    jump_line_pairs: [],
    lifts: [],
    buildings: [],
    motion_data: { layers: [], graph_bytes: [] },
    unknown_export_field: { data: [1, 2] },
  };
}
test("Rust subset validates format and consumed geometry, preserving opaque extensions", () => {
  const input = level();
  assert.deepEqual(JSON.parse(JSON.stringify(parseProtoLevel(input))), input);
  assert.throws(
    () => parseProtoLevel({ ...input, format: "Future" }),
    /level.format/,
  );
  assert.throws(
    () => parseProtoLevel({ ...input, sight_obstacles: [null] }),
    /sight_obstacles\[0\]/,
  );
  assert.throws(
    () =>
      parseProtoLevel({
        ...input,
        material_sectors: [
          { material: 2, polygon: { points: [[1, Infinity]] } },
        ],
      }),
    /material_sectors/,
  );
  assert.throws(
    () =>
      parseLevel3D(document(), {
        level: { ...parseProtoLevel(input), sight_obstacles: [] },
      }),
    /dangling obstacle/,
  );
});
test("patch and animation sprite fields fail at their source path", () => {
  const input = level();
  assert.throws(
    () =>
      parseProtoLevel({
        ...input,
        patches: [
          { active: true, integrate_in_background: false, element_fx: null },
        ],
      }),
    /level.patches\[0\].element_fx/,
  );
  assert.throws(
    () =>
      parseProtoLevel({
        ...input,
        animations: [
          {
            sprite: {
              frame_profile_name: "",
              profile_name: "",
              position_x: NaN,
              position_y: 0,
              elevation: 0,
            },
          },
        ],
      }),
    /level.animations\[0\].sprite.position_x/,
  );
});
function document() {
  return {
    version: 1,
    map: "York",
    glb: "york.glb",
    size: [100, 200],
    camera: { ...camera },
    groups: [],
    future_field: { keep: true },
    objects: [
      {
        id: "building-000",
        node: "building-000",
        kind: "building",
        source: { map: "York", obstacle: 0 },
        transform: { dx: 0, dy: 0, dz: 0, rot_deg: 0 },
        obstacle: {
          points: [
            { x: 1, y: 2, z_bottom: 0, z_top: 4 },
            { x: 8, y: 2, z_bottom: 0, z_top: 4 },
            { x: 3, y: 9, z_bottom: 0, z_top: 4 },
          ],
          opaque: true,
          solid: true,
          mouse: false,
          show_shadow_polygon: false,
          default_material: 0,
          material_indices: [],
          projection_area: {},
          unknown_flag: 77,
        },
      },
    ],
  };
}
test("versioned parsing preserves unknown fields exactly", () => {
  const original = document();
  const parsed = parseLevel3D(original, {
    map: "york",
    glb: "york.glb",
    nodes: new Set(["building-000"]),
  });
  assert.deepEqual(JSON.parse(JSON.stringify(parsed)), original);
});
test("invalid schemas and dangling or mismatched sources fail", () => {
  for (const mutate of [
    (d: any) => (d.version = 2),
    (d: any) => (d.objects[0].transform.dx = Infinity),
    (d: any) => (d.objects[0].group = "missing"),
    (d: any) => d.objects.push(d.objects[0]),
    (d: any) => (d.objects[0].source.map = "Other"),
    (d: any) => (d.camera.elevation_deg = 0),
  ]) {
    const d = document();
    mutate(d);
    assert.throws(() => parseLevel3D(d));
  }
  assert.throws(() => parseLevel3D(document(), { map: "Other" }));
  assert.throws(() => parseLevel3D(document(), { nodes: new Set() }));
  assert.throws(() =>
    parseLevel3D(
      { ...document(), provenance: { glb_sha256: "a".repeat(64) } },
      { glbSha256: "b".repeat(64) },
    ),
  );
});
test("asymmetric coordinates invert and preserve signed winding", () => {
  const game: [number, number, number] = [13, -27, 41];
  const scene = gameToScene(camera, ...game);
  sceneToGame(camera, scene).forEach((v, i) =>
    assert.ok(Math.abs(v - game[i]!) < 1e-10),
  );
  assert.deepEqual(gltfToScene(sceneToGltf(scene)), scene);
  const points: [number, number][] = [
    [1, 2],
    [8, 2],
    [3, 9],
  ];
  assert.equal(signedPolygonArea(points), 24.5);
  assert.equal(signedPolygonArea([...points].reverse()), -24.5);
});
test("scene matrix placement agrees with game obstacle transform used by bake", () => {
  const d = parseLevel3D(document());
  const o = d.objects[0]!;
  o.transform = { dx: 13, dy: -5, dz: 7, rot_deg: 33 };
  const pivot: [number, number] = [4, 13 / 3];
  const matrix = gameTransformMatrix(camera, o.transform, pivot);
  const expected = transformedObstacle(d, o);
  o.obstacle.points.forEach((p, i) => {
    const actual = sceneToGame(
      camera,
      applyAffineMatrix(matrix, gameToScene(camera, p.x, p.y, p.z_top)),
    );
    const q = expected.points[i]!;
    actual.forEach((v, k) =>
      assert.ok(Math.abs(v - [q.x, q.y, q.z_top][k]!) < 1e-9),
    );
  });
});
