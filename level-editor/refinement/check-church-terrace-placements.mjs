import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { groupCentroid, gameTransformMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [staged, mode] = process.argv.slice(2);
assert.ok(staged, "Provide combined church and terrace edits");
assert.ok(mode === undefined || mode === "--published", "Unknown verification mode");
const edits = JSON.parse(await fs.readFile(`${staged}/edits.json`, "utf8"));
const ids = new Set(["leicester-church-side-tower", "leicester-lower-bailey-terrace"]);
assert.deepEqual(new Set(edits.map((e) => e.asset)), ids);
const original = await readStoredMap("library/scenes/leicester.rhlos-map.json", "library");
const sources = original.assetSources.filter((s) => ids.has(s.id));
const assets = await pinnedDescriptors("library", sources, []);
for (const edit of edits) {
  if (mode === "--published") assert.deepEqual(assets.get(edit.asset).gameplay, edit.gameplay);
  else {
    assert.equal(sources.find((s) => s.id === edit.asset).descriptor_sha256, edit.descriptorSha256);
    assets.get(edit.asset).gameplay = edit.gameplay;
  }
}
const output = await fs.mkdtemp("work/map-compile/church-terrace-placements-");
const results = [];
const disconnected = [];
for (const elevation of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    const document = {
      version: 1,
      map: "Church terrace placement",
      camera: original.camera,
      size: [4000, 4000],
      objects: structuredClone(original.objects.filter((p) => ids.has(p.group))),
      groups: structuredClone(original.groups.filter((g) => ids.has(g.id))),
      sceneAssets: [],
      assetSources: sources,
      terrain: createTerrainGrid([0, 0, 4000, 4000], 1000, elevation),
    };
    const matrix = gameTransformMatrix(
      document.camera,
      { dx: 500, dy: 1000, dz: elevation, rot_deg: rotation },
      [1500, 1000],
    );
    for (const group of document.groups) {
      assert.equal(group.transform.rot_deg, 0);
      const pivot = groupCentroid(document.objects.filter((p) => p.group === group.id));
      const [x, y, z] = gameToScene(
        document.camera,
        pivot[0] + group.transform.dx,
        pivot[1] + group.transform.dy,
        group.transform.dz,
      );
      const placed = sceneToGame(document.camera, [
        matrix[0] * x + matrix[4] * y + matrix[8] * z + matrix[12],
        matrix[1] * x + matrix[5] * y + matrix[9] * z + matrix[13],
        matrix[2] * x + matrix[6] * y + matrix[10] * z + matrix[14],
      ]);
      group.transform = {
        dx: placed[0] - pivot[0],
        dy: placed[1] - pivot[1],
        dz: placed[2],
        rot_deg: rotation,
      };
    }
    const compiled = compileMap(document, [0, 0, 4000, 4000], assets, { bestEffort: true });
    assert.equal(
      compiled.descriptor.asset_geometry.lifts.filter((l) => l.physical_navigation).length,
      2,
    );
    const file = `church-terrace-${elevation}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({ file, map: file, warnings: compiled.warnings });
    for (const change of ["missing terrace", "raised terrace"]) {
      const broken = structuredClone(document);
      const terrace = "leicester-lower-bailey-terrace";
      if (change === "missing terrace") {
        broken.objects = broken.objects.filter((p) => p.group !== terrace);
        broken.groups = broken.groups.filter((g) => g.id !== terrace);
      } else broken.groups.find((g) => g.id === terrace).transform.dz += 20;
      const rejected = compileMap(broken, [0, 0, 4000, 4000], assets, { bestEffort: true });
      assert.ok(
        rejected.warnings.some(
          (warning) => warning.includes("building-191") && warning.includes("traversal omitted"),
        ),
        `${file}: ${change} must reject the external stair connection`,
      );
      disconnected.push({ rotation, elevation, change, warnings: rejected.warnings });
    }
  }
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "static-geometry-only-not-gameplay-parity",
    complete: true,
    results,
  }),
);
await fs.writeFile(`${output}/disconnected.json`, JSON.stringify(disconnected));
console.log(output);
