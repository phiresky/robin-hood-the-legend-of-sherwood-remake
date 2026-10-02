import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { IDENTITY_TRANSFORM } from "@rle/shared";
import {
  joinedNavigationCompilerFixture,
  partialNavigationCompilerFixture,
} from "../../shared/test-fixtures/asset-gameplay.ts";
import { compileMap } from "./map-compile.ts";

test("independently placed walkway copies match the native pathfinding fixture", async () => {
  const { document, assets } = joinedNavigationCompilerFixture();
  const groups = [...document.groups];
  for (const group of groups)
    document.groups.push({
      id: `${group.id}-copy`,
      transform: { ...IDENTITY_TRANSFORM, dx: 1000, dy: 100, rot_deg: 90 },
    });
  for (const part of [...document.objects].filter((part) => part.group))
    document.objects.push({
      ...structuredClone(part),
      id: `${part.id}-copy`,
      group: `${part.group}-copy`,
    });
  const descriptor = compileMap(document, [0, 0, 2000, 2000], assets).descriptor;
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-navigation-copies.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(descriptor, expected);
});

test("different walkway widths join along their placed overlap and detach when moved away", async () => {
  const f = partialNavigationCompilerFixture();
  const bounds: [number, number, number, number] = [0, 0, 2000, 2000];
  const compiled = compileMap(f.document, bounds, f.assets).descriptor;
  assert.equal(compiled.asset_geometry!.motion_data.layers.flat().length, 1);
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-navigation-partial.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compiled, expected);
  const group = f.document.groups.find((g) => g.id === "upper")!;
  group.transform.dy = 70;
  assert.equal(
    compileMap(f.document, bounds, f.assets).descriptor.asset_geometry!.motion_data.layers.flat()
      .length,
    2,
  );
  group.transform.dy = 10;
  group.transform.dx = 1;
  assert.equal(
    compileMap(f.document, bounds, f.assets).descriptor.asset_geometry!.motion_data.layers.flat()
      .length,
    2,
  );
  for (const invalid of [0, -1, Infinity, NaN]) {
    f.upper.gameplay!.surfaces[0]!.navigationJoinMinimumOverlap = invalid;
    assert.throws(() => compileMap(f.document, bounds, f.assets), /minimum overlap/);
  }
});
