// Prepare visible return-click probes without changing the exported map geometry.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { isDeepStrictEqual } from "node:util";

const [input] = process.argv.slice(2);
assert.ok(input, "Supply a completed neighboring-asset export and native click report");
const manifest = JSON.parse(await fs.readFile(path.join(input, "diagnostics.json")));
const report = JSON.parse(
  await fs.readFile(path.join(input, "actor-building-terrain-click-report.json")),
);
assert.equal(manifest.complete, true);
assert.equal(report.audit_finished, true);
const output = await fs.mkdtemp("work/needs-review/map-compile/visible-building-approaches-");
const inside = ([x, y], ring) => {
  let result = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const a = ring[i],
      b = ring[j];
    if (a[1] > y !== b[1] > y && x < ((b[0] - a[0]) * (y - a[1])) / (b[1] - a[1]) + a[0])
      result = !result;
  }
  return result;
};
const clearance = (p, ring) =>
  Math.min(
    ...ring.map((a, i) => {
      const b = ring[(i + 1) % ring.length];
      const dx = b[0] - a[0],
        dy = b[1] - a[1];
      const t = Math.max(
        0,
        Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy || 1)),
      );
      return Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
    }),
  );
const reviews = [];
for (const row of manifest.results) {
  const bytes = await fs.readFile(path.join(input, row.file));
  const geometry = JSON.parse(bytes).asset_geometry;
  const areas = [];
  let sector = 0;
  geometry.motion_data.layers.forEach((layer, index) => {
    for (const area of layer) {
      areas.push({ ...area, layer: index, sector });
      sector += 1 + area.obstacles.length;
    }
  });
  for (const probe of row.building_approaches) {
    const previous = report.results.find(
      (r) =>
        r.file === row.file &&
        r.probe.layer === probe.layer &&
        r.probe.sector === probe.sector &&
        isDeepStrictEqual(r.probe.point_out, probe.point_out) &&
        r.probe.source_world.every((v, i) => Math.abs(v - probe.source_world[i]) < 1e-7),
    );
    assert.ok(previous, "Every probe must have an existing native result");
    if (!previous.error?.startsWith("return click selects a different terrain sector:")) continue;
    const target = areas.find((a) => a.layer === probe.layer && a.sector === probe.sector);
    assert.ok(target);
    const height = probe.source_world[2];
    const receivers = geometry.sight_obstacles.filter(
      (s) =>
        s.projection_area?.[0] === probe.sector &&
        s.projection_area[1] === probe.layer &&
        s.points.every((p) => Math.abs(p.z_top - height) < 1e-4),
    );
    assert.ok(receivers.length, "This diagnostic only supports flat receiving floors");
    const source = [probe.source_world[0], probe.source_world[1] - height];
    const candidates = [];
    for (let y = -128; y <= 128; y += 8)
      for (let x = -128; x <= 128; x += 8) candidates.push([source[0] + x, source[1] + y]);
    candidates.sort(
      (a, b) =>
        Math.hypot(a[0] - source[0], a[1] - source[1]) -
        Math.hypot(b[0] - source[0], b[1] - source[1]),
    );
    const selected = candidates.find(
      (p) =>
        inside(p, target.polygon.points) &&
        clearance(p, target.polygon.points) >= 12 &&
        receivers.some((s) =>
          inside(
            p,
            s.points.map((v) => [v.x, v.y - v.z_top]),
          ),
        ) &&
        target.obstacles.every(
          (o) => !inside(p, o.polygon.points) && clearance(p, o.polygon.points) >= 12,
        ) &&
        areas.every((a) => a.layer <= target.layer || !inside(p, a.polygon.points)),
    );
    assert.ok(selected, `No visible return destination for ${row.file}:${probe.sector}`);
    reviews.push({
      file: row.file,
      original: structuredClone(probe),
      originalError: previous.error,
      selectedScreen: selected,
    });
    probe.source_world = [selected[0], selected[1] + height, height];
  }
  await fs.writeFile(path.join(output, row.file), bytes);
}
await fs.writeFile(
  path.join(output, "visible-approach-review.json"),
  JSON.stringify(
    { input, scope: "visible-flat-floor-probes-awaiting-native-route-verification", reviews },
    null,
    2,
  ),
);
await fs.writeFile(
  path.join(output, "diagnostics.json"),
  JSON.stringify({ ...manifest, sourceDiagnostics: input }, null, 2),
);
console.log(output);
