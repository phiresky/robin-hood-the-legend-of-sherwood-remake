import test from "node:test";
import assert from "node:assert/strict";
import { sceneryThumbnailFixture } from "../tests/scenery-thumbnail-fixture.ts";
import {
  assetTags,
  assetType,
  filterAssets,
  REFINED_LEVELS,
  REFINED_LEVELS_FILTER,
} from "./asset-library.ts";

const entries = [
  {
    id: "house",
    name: "Town House",
    source_map: "York",
    descriptor: "house.json",
    model: "house.glb",
    tags: ["stone"],
  },
  {
    id: "tree",
    name: "Oak Tree",
    source_map: "Derby",
    descriptor: "tree.json",
    model: "tree.glb",
    asset_type: "Vegetation",
  },
  {
    id: "tower",
    name: "Tower",
    source_map: "Derby",
    descriptor: "tower.json",
    model: "tower.glb",
    asset_type: "Building",
  },
];
test("animated scenery stays visible in the default library while empty gameplay frames stay hidden", async () => {
  const { descriptor } = await sceneryThumbnailFixture();
  const entry = {
    id: descriptor.id,
    name: descriptor.name,
    source_map: descriptor.source_map,
    descriptor: "fire/asset.json",
    model: "fire/model.glb",
    editor: descriptor,
  };
  assert.deepEqual(filterAssets([entry], "", "", REFINED_LEVELS_FILTER), [entry]);
  descriptor.gameplay!.animations = [];
  assert.deepEqual(filterAssets([entry], "", "", REFINED_LEVELS_FILTER), []);
  assert.deepEqual(filterAssets([entry], "", "", REFINED_LEVELS_FILTER, true), [entry]);
});
test("type, source and multiword search combine over the shared library", () => {
  assert.equal(filterAssets(entries, "", "", "").length, 3);
  assert.deepEqual(
    filterAssets(entries, "", "Building", "Derby").map((entry) => entry.id),
    ["tower"],
  );
  assert.deepEqual(
    filterAssets(entries, "stone york", "Building", "").map((entry) => entry.id),
    ["house"],
  );
  assert.equal(filterAssets(entries, "oak", "Building", "").length, 0);
  assert.equal(assetType({ ...entries[0], asset_type: "Prop" }), "Prop");
  assert.deepEqual(assetTags(entries[0]), ["Building", "York", "stone"]);
});

test("refined-level filter includes reviewed sources regardless of case and combines with other filters", () => {
  const refined = [...REFINED_LEVELS, "Sketchfab"].map((source_map) => ({
    ...entries[0]!,
    id: source_map.toLowerCase(),
    source_map: source_map.toLowerCase(),
  }));
  const catalog = [
    ...refined,
    ...["York", "Croisement01", "Croisement02", "Croisement03", "Wychford", "Future level"].map(
      (source_map) => ({ ...entries[0]!, id: source_map, source_map }),
    ),
  ];
  assert.deepEqual(filterAssets(catalog, "", "", REFINED_LEVELS_FILTER), refined);
  assert.equal(filterAssets(catalog, "", "", "").length, catalog.length);
  assert.deepEqual(
    filterAssets(catalog, "stone nottingham", "Building", REFINED_LEVELS_FILTER).map(
      (entry) => entry.id,
    ),
    ["nottingham"],
  );
  assert.deepEqual(filterAssets(catalog, "", "Vegetation", REFINED_LEVELS_FILTER), []);
  assert.deepEqual(
    filterAssets(catalog, "", "", "York").map((entry) => entry.id),
    ["York"],
  );
});
