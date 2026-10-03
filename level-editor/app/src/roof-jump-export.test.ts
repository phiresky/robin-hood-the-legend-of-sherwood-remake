import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { parseLevel3D, parseProjectionAssetDescriptor } from "../../shared/src/validation.ts";
import { compileMap } from "./map-compile.ts";

test("complete relocated houses generate roof connections with all their collision intact", async () => {
  const input = JSON.parse(
    await readFile(
      new URL("../../shared/test-fixtures/complete-roof-jumps.json", import.meta.url),
      "utf8",
    ),
  );
  const document = parseLevel3D(input.document);
  const asset = parseProjectionAssetDescriptor(input.asset);
  const assets = new Map([[asset.id, asset]]);
  const compile = () =>
    compileMap(document, [0, 0, 2000, 2000], assets, { bestEffort: true }).descriptor;
  const compiled = compile();
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-jump-complete-roofs.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compiled, expected);
  assert.equal(compiled.asset_geometry!.jump_line_pairs!.length, 1);
  assert.ok(
    compiled.asset_geometry!.sight_obstacles!.filter((s) => s.solid).length >=
      asset.parts.length * 2,
  );
  const copy = document.groups.find((group) => group.id === "copy")!;
  const initial = { ...copy.transform };
  for (const [dx, dy] of [
    [-5, 6],
    [5, -6],
  ]) {
    copy.transform = { ...initial, dx: initial.dx + dx!, dy: initial.dy + dy! };
    assert.ok(compile().asset_geometry!.jump_line_pairs?.length);
  }
  copy.transform = { ...initial, dx: initial.dx + 500 };
  assert.equal(compile().asset_geometry!.jump_line_pairs, undefined);
  copy.transform = initial;
  delete asset.gameplay!.surfaces[0]!.jump;
  const withoutRule = compile();
  assert.equal(withoutRule.asset_geometry!.jump_line_pairs, undefined);
  assert.deepEqual(
    withoutRule.asset_geometry!.sight_obstacles,
    compiled.asset_geometry!.sight_obstacles,
  );
});
