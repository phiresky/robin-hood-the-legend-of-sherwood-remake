// Connect independently inserted assets by their local sockets, without saved map placements.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame } from "../shared/src/geometry.ts";

const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const ids = ["york-market-red-canopy-stall", "york-market-stepped-green-canopy-stall"];
const requireLevelSockets = process.argv.includes("--require-level-sockets");
const positional = process.argv.slice(2).filter((arg) => arg !== "--require-level-sockets");
assert.ok(positional.length <= 1);
const edits = positional.length ? JSON.parse(await fs.readFile(`${positional[0]}/edits.json`)) : [];
assert.ok(edits.every((edit) => ids.includes(edit.asset)));
const assets = [];
for (const [side, id] of ids.entries()) {
  const base = `3d-assets/york/${id}`;
  const bytes = await fs.readFile(`library/${base}/asset.json`);
  const descriptor = JSON.parse(bytes);
  const edit = edits.find((edit) => edit.asset === id);
  if (edit) {
    assert.equal(edit.descriptorSha256, hash(bytes));
    assert.equal(edit.modelSha256, hash(await fs.readFile(`library/${base}/model.glb`)));
    descriptor.gameplay = edit.gameplay;
  }
  const segment = descriptor.gameplay.jumpSegments.find(
    (entry) => entry.id === `jump-pair-12-side-${side}`,
  );
  assert.ok(segment?.join);
  const reference = {
    id,
    descriptor: `${base}/asset.json`,
    descriptor_sha256: hash(bytes),
    model: `${base}/model.glb`,
    model_sha256: hash(await fs.readFile(`library/${base}/model.glb`)),
    model_scene: descriptor.model_scene,
    resources: descriptor.resources,
  };
  assets.push({ descriptor, reference, segment });
}
const worldSocket = (document, group, asset) => {
  const part = document.objects.find(
    (part) =>
      part.group === group.id && part.node === `asset:${asset.descriptor.id}:${asset.segment.node}`,
  );
  assert.ok(part);
  const matrix = partMatrix(camera, document, part);
  const local = gameToScene(camera, ...asset.segment.join);
  return sceneToGame(
    camera,
    [0, 1, 2].map(
      (row) =>
        matrix[row] * local[0] +
        matrix[row + 4] * local[1] +
        matrix[row + 8] * local[2] +
        matrix[row + 12],
    ),
  );
};
const output = await fs.mkdtemp("work/needs-review/map-compile/inserted-canopy-jumps-");
console.log(output);
const results = [];
const detached = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "inserted-asset-socket-connection-not-complete-gameplay-parity",
      complete,
      requireLevelSockets,
      gameplayStage: positional[0],
      assets: assets.map((asset) => asset.reference),
      results,
      detached,
    }),
  );
await save(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: "Inserted canopy socket assemblies",
      size: [1800, 1800],
      camera,
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 1800, 1800], 900, height),
    };
    for (const x of [450.25, 1300.25]) {
      const groups = [];
      for (const asset of assets) {
        const inserted = insertProjectionAsset(document, asset.descriptor, asset.reference, [
          x,
          900.75,
          height,
        ]);
        document = inserted.document;
        const group = document.groups.find((group) => group.id === inserted.selection.id);
        group.transform.rot_deg = rotation;
        groups.push(group);
      }
      const a = worldSocket(document, groups[0], assets[0]),
        b = worldSocket(document, groups[1], assets[1]);
      if (requireLevelSockets)
        assert.ok(
          Math.abs(a[2] - b[2]) < 1e-6,
          "Default insertion must align socket heights without vertical correction",
        );
      groups[1].transform.dx += a[0] - b[0];
      groups[1].transform.dy += a[1] - b[1];
      groups[1].transform.dz += a[2] - b[2];
      const aligned = worldSocket(document, groups[1], assets[1]);
      assert.ok(
        Math.hypot(...aligned.map((value, axis) => value - a[axis])) < 1e-6,
        JSON.stringify({ a, b, aligned, transform: groups[1].transform }),
      );
    }
    const compiled = compileMap(
      document,
      [0, 0, 1800, 1800],
      new Map(assets.map((asset) => [asset.descriptor.id, asset.descriptor])),
      { bestEffort: true },
    );
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    const pairs = compiled.descriptor.asset_geometry.jump_line_pairs ?? [];
    assert.equal(pairs.length, 2, "Each copied assembly must retain its own connection");
    results.push({ file, rotation, height, pairs: pairs.length, warnings: compiled.warnings });
    const separated = structuredClone(document);
    const movingGroups = new Set(
      separated.objects
        .filter((part) => part.node.startsWith(`asset:${ids[1]}:`))
        .map((part) => part.group),
    );
    for (const group of separated.groups) if (movingGroups.has(group.id)) group.transform.dx += 100;
    const disconnected = compileMap(
      separated,
      [0, 0, 1800, 1800],
      new Map(assets.map((asset) => [asset.descriptor.id, asset.descriptor])),
      { bestEffort: true },
    );
    assert.equal(
      disconnected.descriptor.asset_geometry.jump_line_pairs?.length ?? 0,
      0,
      "Moved sockets must not leave stale jumps",
    );
    assert.ok(
      disconnected.warnings.filter(
        (warning) => warning.includes("jump-pair-12") && warning.includes("no matching edge"),
      ).length >= 4,
    );
    const detachedFile = `detached-${file}`;
    await fs.writeFile(`${output}/${detachedFile}`, JSON.stringify(disconnected.descriptor));
    detached.push({
      file: detachedFile,
      rotation,
      height,
      pairs: 0,
      warnings: disconnected.warnings,
    });
    await save(false);
  }
await save(true);
