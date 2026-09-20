import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import type { ProtoLevel } from "@rle/shared";
import { MissionEntities, readMission } from "./mission.ts";
import { projectSpritePixel } from "./sprite-profiles.ts";

function directory(files: Record<string, unknown>, prefix = ""): FileSystemDirectoryHandle {
  return {
    async *entries() {
      const names = new Set(Object.keys(files).filter((key) => key.startsWith(prefix)).map((key) => key.slice(prefix.length).split("/")[0]!));
      for (const name of names) {
        const isDir = Object.keys(files).some((key) => key.startsWith(`${prefix}${name}/`));
        yield [name, isDir ? directory(files, `${prefix}${name}/`) : { kind: "file" }];
      }
    },
    kind: "directory",
    async getDirectoryHandle(name: string) {
      const path = `${prefix}${name}/`;
      if (!Object.keys(files).some((key) => key.startsWith(path))) throw new DOMException(path, "NotFoundError");
      return directory(files, path);
    },
    async getFileHandle(name: string) {
      const data = files[prefix + name];
      if (data === undefined) throw new DOMException(name, "NotFoundError");
      return { getFile: async () => new File([JSON.stringify(data)], name) };
    },
  } as unknown as FileSystemDirectoryHandle;
}
const camera = { kind: "oblique-orthographic" as const, elevation_deg: 35 };
const level = { sight_obstacles: [{ points: [
  { x: 0, y: 0, z_top: 100 }, { x: 100, y: 0, z_top: 100 }, { x: 100, y: 100, z_top: 100 },
] }] } as ProtoLevel;

test("mission header resolves its actual map and rejects a missing map", async () => {
  const levelsDir = directory({ "A.rhm.json": { header: { map_filename: "Derby" } }, "B.rhm.json": { header: {} } });
  const index = { levelsDir, maps: new Set(["Derby"]) };
  assert.equal((await readMission(index, "A")).map, "Derby");
  await assert.rejects(readMission(index, "B"), /mission map/);
});

test("target Z overrides support, negative Z derives support, and mobile sprites use the initial waypoint", async () => {
  const root = directory({ "Data/Configuration/profile.cpf.json": {} });
  const index = { root, levelsDir: root, maps: new Set(["Derby"]) };
  const preview = await MissionEntities.load(index, { name: "A", map: "Derby", data: { header: { ambiance: 1 },
    targets: [
      { position_x: 10, position_y: 20, position_z: 40, obstacle_index: 0, filename: "missing", profile_name: "missing", action: 0 },
      { position_x: 10, position_y: 20, position_z: -1, obstacle_index: 0, filename: "missing", profile_name: "missing", action: 0 },
    ],
    mobile_elements: [{ path_index: 0, start_waypoint: 0, obstacle_index: 65535, sprites: [{ sprite: { frame_profile_name: "missing", profile_name: "missing", position_x: 5, position_y: 10 } }] }],
    hiking_paths: [{ waypoints: [{ x: 45, y: 50 }] }],
  } }, level, camera);
  assert.equal(preview.count, 3);
  const [explicit, derived, mobile] = preview.root.children;
  assert.ok(Math.abs(explicit!.position.y - 10 - 40 / Math.cos(35 * Math.PI / 180)) < 1e-7);
  assert.ok(Math.abs(derived!.position.y - 10 - 100 / Math.cos(35 * Math.PI / 180)) < 1e-7);
  assert.equal(mobile!.position.x, 50);
  let geometries = 0, materials = 0;
  preview.root.traverse((object) => {
    if (object instanceof THREE.Mesh) {
      object.geometry.addEventListener("dispose", () => geometries++);
      object.material.addEventListener("dispose", () => materials++);
    }
  });
  preview.dispose(); preview.dispose();
  assert.equal(geometries, 3); assert.equal(materials, 3);
  assert.equal(preview.count, 0);
});

test("missing support fails the mission instead of putting its marker at ground level", async () => {
  const root = directory({ "Data/Configuration/profile.cpf.json": {} });
  await assert.rejects(MissionEntities.load({ root, levelsDir: root, maps: new Set() }, {
    name: "A", map: "Derby", data: { header: { ambiance: 1 }, targets: [{ position_x: 10, position_y: 20, obstacle_index: 77, filename: "missing", profile_name: "missing", action: 0 }] },
  }, level, camera), /Missing entity support #77/);
});

test("only prone characters anchor directional projections; other profiles follow the camera", () => {
  for (const shape of ["upright-character", "prone-character", "cylinder-object", "low-object", "upright-scenery"] as const) {
    const preview = new MissionEntities();
    const geometry = new THREE.BufferGeometry();
    geometry.userData.spriteShape = shape;
    const material = new THREE.MeshBasicMaterial();
    const mesh = new THREE.Mesh(geometry, material);
    const shadow = new THREE.Mesh();
    mesh.add(shadow);
    const frames = new Map(Array.from({ length: 16 }, (_, direction) => [direction, { geometry, texture: new THREE.Texture() }]));
    Object.assign(preview, { actors: [{ mesh, shadow, frames, direction: 15 }] });
    const point = new THREE.Vector3(...projectSpritePixel(shape, 4, 12, { left: -12, top: 40, width: 24, height: 40 }, 35 * Math.PI / 180));
    const camera = new THREE.PerspectiveCamera();
    const view = (degrees: number) => {
      const radians = degrees * Math.PI / 180;
      camera.position.set(Math.sin(radians) * 1000, 500, Math.cos(radians) * 1000);
      camera.lookAt(mesh.position);
      camera.updateMatrixWorld();
      preview.update(camera);
      mesh.updateMatrixWorld(true);
      return point.clone().applyMatrix4(mesh.matrixWorld);
    };
    const start = view(0);
    const movement = view(10).distanceTo(start);
    assert.ok(shape === "prone-character" ? movement < 1e-10 : movement > 0.1, `${shape}: projection policy within a sector`);
    assert.equal(material.map, frames.get(15)!.texture);
    const next = view(12);
    assert.equal(material.map, frames.get(0)!.texture);
    assert.ok(next.distanceTo(start) > 0.1);
    const nextMovement = view(30).distanceTo(next);
    assert.ok(shape === "prone-character" ? nextMovement < 1e-10 : nextMovement > 0.1, `${shape}: projection policy after a frame transition`);
    assert.ok(Math.abs(shadow.getWorldQuaternion(new THREE.Quaternion()).y) < 1e-10);
    const single = frames.get(0)!;
    Object.assign(preview, { actors: [{ mesh, shadow, frames: new Map([[-1, single]]), direction: 15 }] });
    assert.ok(view(0).distanceTo(view(130)) < 1e-10, `${shape}: single-view sprites remain fixed`);
    preview.dispose();
    geometry.dispose(); material.dispose(); shadow.geometry.dispose();
    (shadow.material as THREE.Material).dispose();
    for (const frame of frames.values()) frame.texture.dispose();
  }
});
