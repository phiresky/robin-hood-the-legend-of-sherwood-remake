import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { StateMaterialFrames, retainPhaseMaterials } from "./state-material-frames.ts";
import { StateAppearancePlayer, captureStateAppearance } from "./state-appearance-player.ts";
import type { PhysicalMaterialFrame } from "../../shared/src/state-delivery.ts";
function fixture() {
  const source = new THREE.Group();
  const geometry = new THREE.PlaneGeometry();
  geometry.setAttribute("uv1", geometry.getAttribute("uv").clone());
  const mesh = new THREE.Mesh(geometry, new THREE.MeshBasicMaterial());
  mesh.name = "net";
  source.add(mesh);
  const textures = [new THREE.Texture(), new THREE.Texture()];
  textures.forEach((t) => (t.channel = 1));
  const materials = textures.map((map) => new THREE.MeshBasicMaterial({ map, transparent: true }));
  retainPhaseMaterials(
    source,
    new Map(
      materials.map((material, i) => [
        i,
        { material, name: `phase${i}`, texture_index: i, tex_coord: 1, alpha_mode: "BLEND" },
      ]),
    ),
  );
  const frames: PhysicalMaterialFrame[] = materials.map((_, i) => ({
    duration_ticks: 3,
    materials: [
      {
        node: "net",
        slot: 0,
        material_index: i,
        material_name: `phase${i}`,
        texture_index: i,
        tex_coord: 1,
        alpha_mode: "BLEND",
      },
    ],
  }));
  const template = captureStateAppearance(source, [
    new THREE.AnimationClip("motion", -1, [
      new THREE.VectorKeyframeTrack(`${mesh.uuid}.position`, [0, 6 / 25], [0, 0, 0, 6, 0, 0]),
    ]),
  ]);
  const player = new StateAppearancePlayer(template, true);
  player.select("motion", { mode: "loop", cycleTicks: 6 });
  return { source, mesh, textures, materials, frames, player };
}
test("phase materials retain UV/alpha and source clock boundaries beside continuous geometry", () => {
  const f = fixture();
  const binding = new StateMaterialFrames(f.source, f.player.content, f.frames);
  const clone = f.player.content.getObjectByName("net") as THREE.Mesh<
    THREE.BufferGeometry,
    THREE.MeshBasicMaterial
  >;
  const independent = f.source.clone(true).getObjectByName("net") as THREE.Mesh;
  let disposed = 0,
    textureDisposed = 0;
  f.textures.forEach((t) => t.addEventListener("dispose", () => textureDisposed++));
  for (const tick of [0, 2, 3, 5, 6, 9, 0]) {
    f.player.sampleExternalTick(tick);
    binding.sample(f.player.tick);
    assert.equal(clone.material.map, f.textures[f.player.tick < 3 ? 0 : 1]);
    assert.equal(clone.material.transparent, true);
    assert.ok(Math.abs(clone.position.x - f.player.tick) < 1e-5);
    assert.equal(clone.geometry, f.mesh.geometry);
    assert.equal(independent.material, f.mesh.material);
  }
  binding.sample(0);
  clone.material.addEventListener("dispose", () => disposed++);
  binding.sample(3);
  clone.material.addEventListener("dispose", () => disposed++);
  binding.dispose();
  binding.dispose();
  f.player.dispose();
  assert.equal(disposed, 2);
  assert.equal(textureDisposed, 0);
  assert.equal(clone.material, f.mesh.material);
  assert.throws(() => binding.sample(0), /disposed/);
});
test("phase binding rejects material identity, texture, slot, alpha and UV mismatches", () => {
  for (const key of ["material_index", "texture_index", "slot", "tex_coord"] as const) {
    const f = fixture();
    f.frames[0]!.materials[0]![key] = 7;
    assert.throws(
      () => new StateMaterialFrames(f.source, f.player.content, f.frames),
      /identity|slot/,
    );
    f.player.dispose();
  }
  for (const key of ["node", "material_name", "alpha_mode"] as const) {
    const f = fixture();
    Object.assign(f.frames[0]!.materials[0]!, { [key]: "wrong" });
    assert.throws(
      () => new StateMaterialFrames(f.source, f.player.content, f.frames),
      /identity|mesh/,
    );
    f.player.dispose();
  }
  const f = fixture();
  f.mesh.geometry.deleteAttribute("uv1");
  assert.throws(() => new StateMaterialFrames(f.source, f.player.content, f.frames), /UV/);
  f.player.dispose();
});
