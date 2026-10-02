import test from "node:test";
import assert from "node:assert/strict";
import {
  connectedInteriorCompilerFixture,
  interiorAssetCompilerFixture,
} from "../test-fixtures/asset-gameplay.ts";
import { connectInteriors, placedInteriorOptions } from "./interior-connections.ts";
import { parseLevel3D } from "./validation.ts";
import { parseStoredMap, serializeStoredMap } from "./stored-level.ts";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";
import { assetVariantId } from "./projection-assets.ts";

const bounds: [number, number, number, number] = [0, 0, 2000, 2000];

test("connections to a visual state view resolve to that placement's shared gameplay room", () => {
  const { document, assets, annex } = connectedInteriorCompilerFixture();
  const expected = compileAssetGameplay(document, assets, bounds);
  annex.state_variants = { applied: { name: "Applied", model: annex.model, parts: annex.parts } };
  const alias = assetVariantId(annex.id, "applied");
  assets.set(alias, { ...annex, id: alias });
  document.assetSources!.push({
    ...document.assetSources!.find((source) => source.id === annex.id)!,
    id: alias,
    state_variant: "applied",
  });
  document.objects.find((part) => part.group === annex.id)!.node = `asset:${alias}:building-999`;
  document.interiorConnections![0]!.to.asset = alias;
  const before = structuredClone(document);
  assert.deepEqual(compileAssetGameplay(document, assets, bounds), expected);
  assert.deepEqual(document, before);
});

test("local rooms connect all their doors without editor links and keep different rooms separate", () => {
  const { document, assets, hut } = interiorAssetCompilerFixture();
  assert.equal(document.interiorConnections, undefined);
  assert.equal(placedInteriorOptions(document, assets).length, 1);
  let buildings = compileAssetGameplay(document, assets, bounds).buildings!;
  assert.equal(buildings.length, 1);
  assert.equal(buildings[0]!.Building.doors.length, 2);
  assert.equal(
    buildings[0]!.Building.doors[0]!.sector_in,
    buildings[0]!.Building.doors[1]!.sector_in,
  );
  const first = hut.gameplay!.interiors![0]!;
  hut.gameplay!.interiors!.push({ id: "upstairs", node: first.node, doors: [first.doors.pop()!] });
  buildings = compileAssetGameplay(document, assets, bounds).buildings!;
  assert.equal(buildings.length, 2);
  assert.notEqual(
    buildings[0]!.Building.doors[0]!.sector_in,
    buildings[1]!.Building.doors[0]!.sector_in,
  );
});

test("editor room connections survive save/reopen and independent asset movement", () => {
  const { document, assets } = connectedInteriorCompilerFixture();
  const saved = serializeStoredMap(document, assets);
  const restored = parseStoredMap(saved, assets);
  assert.deepEqual(restored.interiorConnections, document.interiorConnections);
  const part = restored.objects.find((part) => part.group === "annex")!;
  const before = compileAssetGameplay(restored, assets, bounds).buildings!;
  assert.equal(before.length, 1);
  part.transform.dx += 100;
  const after = compileAssetGameplay(restored, assets, bounds).buildings!;
  assert.equal(after.length, 1);
  const doors = after[0]!.Building.doors;
  assert.equal(doors.length, 3);
  assert.equal(doors[0]!.sector_in, doors[1]!.sector_in);
  assert.equal(
    Math.max(...doors.map((door) => door.point_out[0])),
    Math.max(...before[0]!.Building.doors.map((door) => door.point_out[0])) + 100,
  );
  assert.notEqual(doors[0]!.locked_pc, doors[1]!.locked_pc);
  restored.interiorConnections = [];
  assert.equal(compileAssetGameplay(restored, assets, bounds).buildings!.length, 2);
});

test("missing room definitions fail strict export and warn on best-effort export", () => {
  const { document, assets } = connectedInteriorCompilerFixture();
  document.interiorConnections![0]!.to.interior = "missing";
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /endpoint room is missing/);
  const result = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(result.buildings!.length, 2);
  assert.match(result.warnings!.join("\n"), /connection omitted/);
});

test("editor links reject stale placements, duplicate pairs and asset-local overrides", () => {
  const { document, assets } = connectedInteriorCompilerFixture();
  const link = document.interiorConnections![0]!;
  assert.equal(placedInteriorOptions(document, assets).length, 2);
  assert.throws(
    () => connectInteriors(document, link.to, link.from),
    /Duplicate interior connection/,
  );
  assert.throws(
    () => connectInteriors(document, link.from, { ...link.from, interior: "another" }),
    /defined by the asset/,
  );
  link.from.placement = "missing";
  assert.throws(() => parseLevel3D(document), /missing placement/);
});
