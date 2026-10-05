import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { compileMap } from "../app/src/map-compile.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { groupCentroid, groupParts, partMatrix } from "../shared/src/level3d.ts";
import { sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";
import { gameToScene } from "../shared/src/scene.ts";

const [staged, compositeCandidate, mode] = process.argv.slice(2);
assert.ok(mode === undefined || mode === "--published", "Unknown verification mode");
assert.ok(
  staged && compositeCandidate,
  "Provide component edits and composite candidate directories",
);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const edits = JSON.parse(await fs.readFile(`${staged}/edits.json`, "utf8"));
const load = async (id) => {
  const entry = index.find((value) => value.id === id);
  assert.ok(entry, id);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`, "utf8");
  assert.equal(hash(bytes), entry.descriptor_sha256);
  const descriptor = JSON.parse(bytes);
  const edit = edits.find((value) => value.asset === id);
  if (edit) {
    if (mode === "--published") assert.deepEqual(descriptor.gameplay, edit.gameplay);
    else {
      assert.equal(edit.descriptorSha256, entry.descriptor_sha256);
      descriptor.gameplay = edit.gameplay;
    }
  }
  const reference = {
    id,
    descriptor: `3d-assets/${entry.descriptor}`,
    descriptor_sha256: entry.descriptor_sha256,
    model: `3d-assets/${entry.model}`,
    model_sha256: hash(await fs.readFile(`library/3d-assets/${entry.model}`)),
    resources: descriptor.resources ?? [],
    ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
  };
  return { descriptor, reference };
};
const composite = await load("derby-great-keep");
composite.descriptor.gameplay = JSON.parse(
  await fs.readFile(`${compositeCandidate}/candidate.gameplay.json`, "utf8"),
);
const components = await Promise.all(
  [
    "derby-keep-central-gallery",
    "derby-keep-main-hall",
    "derby-keep-north-tower",
    "derby-keep-west-tower",
  ].map(load),
);
for (const component of components) {
  const deltas = component.descriptor.parts.map((part) => {
    const target = composite.descriptor.parts.find((value) => value.node === part.node);
    assert.ok(target, part.node);
    const a = part.obstacle_local_game.points[0],
      b = target.obstacle_local_game.points[0];
    return [b.x - a.x, b.y - a.y, b.z_bottom - a.z_bottom];
  });
  assert.ok(
    deltas.every((delta) => delta.every((v, i) => Math.abs(v - deltas[0][i]) < 1e-5)),
    `Component requires independent part frames: ${component.descriptor.id}`,
  );
  component.offset = deltas[0];
}
const assets = new Map(
  components.map((component) => [component.descriptor.id, component.descriptor]),
);
const output = await fs.mkdtemp("work/map-compile/keep-component-placements-");
const results = [];
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    const empty = {
      version: 1,
      map: "Keep component placement",
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      size: [4000, 4000],
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 4000, 4000], 1000, height),
    };
    const placement = [1700, 1800, height];
    const reference = insertProjectionAsset(
      empty,
      composite.descriptor,
      composite.reference,
      placement,
    );
    const base = reference.document.groups[0].transform;
    base.rot_deg = rotation;
    const pivot = groupCentroid(reference.document.objects);
    let document = empty;
    const angle = (rotation * Math.PI) / 180,
      cos = Math.cos(angle),
      sin = Math.sin(angle);
    const squash = Math.sin((empty.camera.elevation_deg * Math.PI) / 180);
    for (const component of components) {
      document = insertProjectionAsset(
        document,
        component.descriptor,
        component.reference,
        placement,
      ).document;
      const [dx, dy, dz] = component.offset;
      const group = document.groups.at(-1);
      const localPivot = groupCentroid(groupParts(document, group.id));
      const x = dx - pivot[0] + localPivot[0],
        y = dy - pivot[1] + localPivot[1];
      document.groups.at(-1).transform = {
        dx: base.dx + pivot[0] - localPivot[0] + x * cos - (y * sin) / squash,
        dy: base.dy + pivot[1] - localPivot[1] + x * sin * squash + y * cos,
        dz: base.dz + dz,
        rot_deg: rotation,
      };
      for (const part of groupParts(document, group.id)) {
        const node = part.node.split(":").at(-1);
        const target = reference.document.objects.find(
          (value) => value.node.split(":").at(-1) === node,
        );
        assert.ok(target, node);
        const placed = (doc, object) => {
          const p = object.obstacle.points[0];
          return sceneToGame(
            doc.camera,
            applyAffineMatrix(
              partMatrix(doc.camera, doc, object),
              gameToScene(doc.camera, p.x, p.y, p.z_bottom),
            ),
          );
        };
        const actual = placed(document, part),
          expected = placed(reference.document, target);
        assert.ok(
          actual.every((value, i) => Math.abs(value - expected[i]) < 1e-5),
          `Component frame diverges at ${node}, rotation ${rotation}: ${actual} / ${expected}`,
        );
      }
    }
    const compiled = compileMap(document, [0, 0, 4000, 4000], assets, { bestEffort: true });
    const file = `keep-components-${height}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    const geometry = compiled.descriptor.asset_geometry;
    results.push({
      file,
      map: file,
      lifts: geometry.lifts.length,
      physical: geometry.lifts.filter((lift) => lift.physical_navigation).length,
      controls: geometry.movement_transitions.length,
      warnings: compiled.warnings,
    });
  }
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({ scope: "static-geometry-only-not-gameplay-parity", complete: true, results }),
);
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(
  JSON.stringify({ output, placements: results.map(({ warnings, ...value }) => value) }, null, 2),
);
