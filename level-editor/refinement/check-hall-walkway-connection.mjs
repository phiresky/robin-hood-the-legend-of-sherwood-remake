import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import {
  createTerrainGrid,
  groupCentroid,
  groupParts,
  sceneToGame,
  serializeStoredMap,
  parseStoredMap,
} from "../shared/src/index.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";

// A reveal control's approach belongs to a separate raised walkway. Verify
// spatial binding from reusable assets, including independent copies and loss
// of the neighboring floor. No level descriptor is used to compile the scene.
const ids = ["derby-east-hall", "derby-east-bailey-west-curtain"];
const assets = new Map();
const references = new Map();
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
for (const id of ids) {
  const path = `3d-assets/derby/${id}/asset.json`;
  const bytes = await fs.readFile(`library/${path}`);
  const asset = JSON.parse(bytes);
  const model = `3d-assets/derby/${id}/${asset.model}`;
  assets.set(id, asset);
  references.set(id, {
    id,
    descriptor: path,
    descriptor_sha256: hash(bytes),
    model,
    model_sha256: hash(await fs.readFile(`library/${model}`)),
    model_scene: asset.model_scene,
    resources: asset.resources,
  });
}
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
// Stored asset origins establish one adjoining layout for this regression;
// the compiler receives only ordinary independently placed asset instances.
const origins = ids.map((id) => sceneToGame(camera, assets.get(id).source_origin_scene));
const bounds = [0, 0, 5000, 5000];
const output = await fs.mkdtemp("work/map-compile/hall-walkway-connection-");
console.log(output);
const results = [];
const rotations = [0, 37, 90, 270];
const states = ["joined", "separated"];
const report = () =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      complete: results.length === rotations.length * states.length,
      scope: "reveal-control-binding-not-full-asset-parity",
      expected_controls: rotations.length * 2,
      results,
    }),
  );
await report();
for (const rotation of rotations)
  for (const state of states) {
    let document = {
      version: 1,
      map: `Hall walkway ${rotation} ${state}`,
      size: [bounds[2], bounds[3]],
      camera,
      sceneAssets: [],
      groups: [],
      objects: [],
      terrain: createTerrainGrid(bounds, 500, 40),
    };
    for (const center of [
      [1300, 1300],
      [3600, 3600],
    ]) {
      const placed = [];
      for (const [index, id] of ids.entries()) {
        const gap = state === "separated" && index === 1 ? 700 : 0;
        document = insertProjectionAsset(document, assets.get(id), references.get(id), [
          center[0] + origins[index][0] - origins[0][0] + gap,
          center[1] + origins[index][1] - origins[0][1],
          40,
        ]).document;
        placed.push(document.groups.at(-1));
      }
      // Rotate the complete arrangement while retaining two independent groups.
      const angle = (rotation * Math.PI) / 180;
      const foreshortening = Math.sin((camera.elevation_deg * Math.PI) / 180);
      for (const group of placed) {
        const pivot = groupCentroid(groupParts(document, group.id));
        const x = group.transform.dx + pivot[0] - center[0];
        const y = (group.transform.dy + pivot[1] - center[1]) / foreshortening;
        group.transform.dx = center[0] - pivot[0] + x * Math.cos(angle) - y * Math.sin(angle);
        group.transform.dy =
          center[1] - pivot[1] + (x * Math.sin(angle) + y * Math.cos(angle)) * foreshortening;
        group.transform.rot_deg = rotation;
      }
    }
    const stored = serializeStoredMap(document, assets);
    document = parseStoredMap(stored, assets);
    const compiled = compileMap(document, bounds, assets, { bestEffort: true });
    const controls = (compiled.descriptor.asset_geometry.movement_transitions ?? []).filter(
      (control) => control.id.endsWith("/hall-reveal"),
    );
    const missing = compiled.warnings.filter((warning) => warning.includes("hall-reveal"));
    assert.equal(controls.length, state === "joined" ? 2 : 0, JSON.stringify(missing));
    assert.equal(missing.length, state === "joined" ? 0 : 2);
    if (controls.length) {
      for (const control of controls) {
        assert.equal(control.has_appearance, true);
        assert.equal(control.initial_sight.length, 1);
        assert.equal(control.applied_sight.length, 8);
      }
      assert.notDeepEqual(controls[0].waypoint, controls[1].waypoint);
      const left = [...controls[0].initial_sight, ...controls[0].applied_sight];
      assert.ok(
        [...controls[1].initial_sight, ...controls[1].applied_sight].every(
          (index) => !left.includes(index),
        ),
      );
    }
    const file = `hall-${rotation}-${state}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(stored));
    results.push({
      file,
      map: file,
      rotation,
      state,
      expected_controls: controls.length,
      warnings: compiled.warnings,
    });
    await report();
  }
