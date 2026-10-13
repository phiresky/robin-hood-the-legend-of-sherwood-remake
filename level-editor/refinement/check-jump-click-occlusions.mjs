// Explain ordinary-click misses only when a higher floor and a successful hidden
// click independently account for every failed native traversal case.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const [directory] = process.argv.slice(2);
assert.ok(directory);
const read = async (file) => JSON.parse(await fs.readFile(`${directory}/${file}`));
const ordinary = await read("actor-jump-click-report.json");
const hidden = await read("actor-jump-hidden-click-report.json");
assert.equal(ordinary.diagnostic_endpoint_override, false);
assert.equal(hidden.diagnostic_endpoint_override, false);
assert.equal(hidden.complete, true);
const failures = ordinary.results.filter((result) => !result.passed);
assert.ok(failures.length, "No ordinary-click failures to explain");
const results = [];
for (const failure of failures) {
  const match =
    /^click does not select jump overlay: MapPoint \{ x: ([\d.e+-]+), y: ([\d.e+-]+) \}, sector=SectorNumber\((\d+)\), layer=(\d+),/.exec(
      failure.error,
    );
  assert.ok(match, `Unexplained failure: ${failure.error}`);
  const point = [Number(match[1]), Number(match[2])];
  const topology = [Number(match[3]), Number(match[4])];
  const success = hidden.results.filter(
    (result) =>
      result.file === failure.file && result.line === failure.line && result.t === failure.t,
  );
  assert.equal(success.length, 1, "Each occluded case needs exactly one hidden-click traversal");
  assert.equal(success[0].passed, true);
  const geometry = (await read(failure.file)).asset_geometry;
  const pair = geometry.jump_line_pairs[Math.floor(failure.line / 2)];
  const line = failure.line % 2 === 0 ? pair.line1 : pair.line2;
  const zone = geometry.jump_zones[line.jump_zone_index];
  const at = ([sector, layer]) =>
    geometry.sight_obstacles.filter(
      (receiver) =>
        receiver.projection_area?.[0] === sector &&
        receiver.projection_area?.[1] === layer &&
        pointInGameplayPolygon(
          point,
          receiver.points.map((p) => [p.x, p.y - p.z_top]),
          true,
        ),
    );
  const receivingHeight = (receiver) =>
    planeHeight(
      heightPlane(
        (
          receiver.projection_plane ?? receiver.points.slice(0, 3).map((p) => [p.x, p.y, p.z_top])
        ).map(([x, y, z]) => [x, y - z, z]),
      ),
      point,
    );
  const covering = at(topology),
    targets = at([zone.sector, zone.layer]);
  assert.ok(covering.length && targets.length, "Both selected floors must cover the click");
  const coverHeight = Math.min(...covering.map(receivingHeight));
  const targetHeight = Math.max(...targets.map(receivingHeight));
  assert.ok(coverHeight > targetHeight + 0.001, "Selected floor is not above the hidden target");
  results.push({
    file: failure.file,
    line: failure.line,
    t: failure.t,
    point,
    coverHeight,
    targetHeight,
  });
}
const report = {
  scope: "ordinary-and-hidden-click-traversal-with-geometric-occlusion-not-rendered-contact",
  complete: true,
  ordinaryPasses: ordinary.results.filter((result) => result.passed).length,
  explainedHiddenPasses: results.length,
  results,
};
await fs.writeFile(`${directory}/explained-click-occlusions.json`, JSON.stringify(report), {
  flag: "wx",
});
console.log(
  JSON.stringify({ ordinaryPasses: report.ordinaryPasses, explainedHiddenPasses: results.length }),
);
