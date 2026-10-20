import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { groupCentroid } from "../shared/src/level3d.ts";
import { compileMap } from "../app/src/map-compile.ts";
const [stage, mode] = process.argv.slice(2);
assert.ok(stage && (mode === undefined || mode === "--published"));
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const source = await readStoredMap("library/scenes/lincoln.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", source.assetSources, source.sceneAssets);
for (const edit of edits) {
  if (mode === "--published") {
    const { draft: _liveDraft, ...live } = assets.get(edit.asset).gameplay;
    const { draft: _stagedDraft, ...staged } = edit.gameplay;
    assert.deepEqual(live, staged);
  } else assets.get(edit.asset).gameplay = edit.gameplay;
}
const ids = [
  "lincoln-north-hall-stair",
  "lincoln-north-hall",
  "lincoln-north-curtain-wall-west",
  "lincoln-castle-hill-north-bailey-plateau",
];
const output = await fs.mkdtemp("work/needs-review/map-compile/north-hall-assemblies-");
const results = [];
const rejected = [];
console.log(output);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    const document = {
      version: 1,
      map: "north-hall-assembly",
      camera: source.camera,
      size: [5000, 4500],
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: source.assetSources,
    };
    const angle = (rotation * Math.PI) / 180;
    const sinT = Math.sin((source.camera.elevation_deg * Math.PI) / 180);
    const rotate = ([x, y]) => [
      x * Math.cos(angle) - (y * Math.sin(angle)) / sinT,
      x * Math.sin(angle) * sinT + y * Math.cos(angle),
    ];
    for (const [copy, center] of [
      [0, [1700, 1800]],
      [1, [3200, 2600]],
    ])
      for (const id of ids) {
        const group = structuredClone(source.groups.find((group) => group.id === id));
        const objects = source.objects.filter((object) => object.group === id);
        assert.ok(group && group.transform.rot_deg === 0);
        const pivot = groupCentroid(objects),
          rotated = rotate(pivot);
        const origin = rotate([group.transform.dx - 2076, group.transform.dy - 886.1755941623661]);
        group.id = `copy${copy}/${id}`;
        group.transform = {
          dx: center[0] + origin[0] - pivot[0] + rotated[0],
          dy: center[1] + origin[1] - pivot[1] + rotated[1],
          dz: group.transform.dz + height,
          rot_deg: rotation,
        };
        document.groups.push(group);
        document.objects.push(
          ...objects.map((object) => ({
            ...structuredClone(object),
            id: `copy${copy}/${object.id}`,
            group: group.id,
          })),
        );
      }
    const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
    assert.equal(compiled.descriptor.asset_geometry.lifts?.length, 2, compiled.warnings.join("\n"));
    const file = `north-hall-${height}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({ file, warnings: compiled.warnings });
    for (const kind of ["missing", "raised"]) {
      const changed = structuredClone(document);
      const wall = "copy0/lincoln-north-curtain-wall-west";
      if (kind === "missing") {
        changed.groups = changed.groups.filter((group) => group.id !== wall);
        changed.objects = changed.objects.filter((object) => object.group !== wall);
      } else changed.groups.find((group) => group.id === wall).transform.dz += 20;
      const invalid = compileMap(changed, [0, 0, ...changed.size], assets, { bestEffort: true });
      assert.equal(invalid.descriptor.asset_geometry.lifts?.length, 1, `${file} ${kind}`);
      rejected.push({ file, kind, warnings: invalid.warnings });
    }
  }
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "moved-assembly-static-compilation-not-traversal",
    complete: true,
    gameplayStage: stage,
    results,
  }),
);
await fs.writeFile(`${output}/rejected.json`, JSON.stringify(rejected));
