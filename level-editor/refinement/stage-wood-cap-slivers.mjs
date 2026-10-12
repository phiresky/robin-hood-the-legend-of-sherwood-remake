// Offline repair of numerical fragments in mesh-derived draft wood collision.
// This deliberately does not remove arbitrary thin volumes during map export.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const ids = process.argv.slice(2);
assert.ok(ids.length && ids.every((id) => !id.startsWith("-")), "Supply draft wood asset IDs");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8"));
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const output = await fs.mkdtemp("work/map-compile/wood-cap-slivers-");
console.log(output);
const edits = [],
  reviews = [],
  results = [];
const save = async (complete) => {
  await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
  await fs.writeFile(`${output}/review.json`, JSON.stringify(reviews));
  await fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      complete,
      results,
    }),
  );
};
await save(false);
for (const id of ids) {
  const entry = index.assets.find((entry) => entry.id === id);
  assert.ok(entry, id);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(hash(bytes), entry.descriptor_sha256);
  const descriptor = JSON.parse(bytes);
  const gameplay = descriptor.gameplay;
  assert.ok(
    gameplay.draft.issues.some((issue) => /^Mesh-derived (wood|log|bridge) collision/.test(issue)),
  );
  assert.ok(
    gameplay.volumes.length &&
      gameplay.volumes.every((volume) =>
        /^(wood-\d+|timber-\d+-face-\d+-cell-\d+|deck-body-\d+)$/.test(volume.id),
      ),
  );
  const measures = gameplay.volumes.map((volume) => {
    const points = volume.shape.points,
      a = points[0];
    let area = 0,
      enclosed = 0;
    for (let i = 1; i + 1 < points.length; i++) {
      const b = points[i],
        c = points[i + 1];
      const triangle = Math.abs((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)) / 2;
      area += triangle;
      enclosed +=
        (triangle * [a, b, c].reduce((sum, point) => sum + point.z_top - point.z_bottom, 0)) / 3;
    }
    const longest = Math.max(
      ...points.map((point, i) => {
        const next = points[(i + 1) % points.length];
        return Math.hypot(next.x - point.x, next.y - point.y);
      }),
    );
    assert.ok(area > 0 && enclosed >= 0 && longest > 0);
    return { id: volume.id, area, enclosed, width: (2 * area) / longest };
  });
  const discarded = measures.filter(
    (measure) => !measure.id.startsWith("deck-body-") && measure.width < 0.001,
  );
  const sum = (rows, key) => rows.reduce((sum, row) => sum + row[key], 0);
  const areaRatio = sum(discarded, "area") / sum(measures, "area");
  const volumeRatio = sum(discarded, "enclosed") / sum(measures, "enclosed");
  // A candidate must retain at least 99.99% of both footprint and enclosed
  // volume. Contact probes are still needed: these totals are not a gap proof.
  assert.ok(areaRatio < 0.0001 && volumeRatio < 0.0001, `${id}: sliver repair exceeds loss budget`);
  const removed = new Set(discarded.map((measure) => measure.id));
  gameplay.volumes = gameplay.volumes.filter((volume) => !removed.has(volume.id));
  assert.ok(gameplay.volumes.length);
  gameplay.draft.issues.push(
    `Offline cap repair removed ${removed.size} mesh-decomposition slivers narrower than 0.001 game units; footprint loss ${sum(discarded, "area")} square units and volume loss ${sum(discarded, "enclosed")} cubic units. Contact continuity and rendered integration remain under review.`,
  );
  validateAssetGameplay(gameplay, descriptor);
  const modelSha256 = hash(await fs.readFile(`library/3d-assets/${entry.model}`));
  edits.push({ asset: id, descriptorSha256: hash(bytes), modelSha256, gameplay });
  reviews.push({ asset: id, discarded, areaRatio, volumeRatio, retained: gameplay.volumes.length });
  await save(false);
  const reference = {
    id,
    role: "objects",
    model: `3d-assets/${entry.model}`,
    model_sha256: modelSha256,
    descriptor: `3d-assets/${entry.descriptor}`,
    descriptor_sha256: hash(bytes),
    model_scene: entry.model_scene,
    resources: descriptor.resources ?? [],
  };
  for (const elevation of [0, 40])
    for (const rotation of [0, 37, 90, 180, 270]) {
      const empty = {
        version: 1,
        map: id,
        camera: { kind: "oblique-orthographic", elevation_deg: 35 },
        size: [4000, 4000],
        objects: [],
        groups: [],
        sceneAssets: [],
        assetSources: [],
        terrain: createTerrainGrid([0, 0, 4000, 4000], 4000, elevation),
      };
      const { document } = insertProjectionAsset(empty, descriptor, reference, [
        1991,
        958.246618458,
        elevation,
      ]);
      document.groups[0].transform.rot_deg = rotation;
      const compiled = compileMap(document, [0, 0, 4000, 4000], new Map([[id, descriptor]]), {
        bestEffort: false,
      });
      const file = `${id}-${elevation}-${rotation}.level.json`;
      await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
      await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
      results.push({ file, warnings: compiled.warnings });
      await save(false);
    }
  console.log(`${id}: ${gameplay.volumes.length} retained volumes, ten placements compiled`);
}
await save(true);
