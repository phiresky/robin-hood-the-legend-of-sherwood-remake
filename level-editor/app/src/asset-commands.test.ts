import test from "node:test";
import assert from "node:assert/strict";
import {
  IDENTITY_TRANSFORM,
  compactAssetInstances,
  hydrateAssetInstances,
  parseLevel3D,
  parseProjectionAssetDescriptor,
  parseStoredMap,
  partPivot,
  serializeStoredMap,
  transformedObstacle,
  type Level3D,
  type ProjectionAssetDescriptor,
  type ExternalAssetSource,
} from "@rle/shared";
import { insertProjectionAsset } from "./asset-commands.ts";
import { duplicateSelection, deleteSelection, patchGroup } from "./document-commands.ts";

export function assetFixture() {
  const obstacle = {
    points: [
      { x: 0, y: 0, z_bottom: 0, z_top: 10 },
      { x: 10, y: 0, z_bottom: 0, z_top: 10 },
      { x: 0, y: 10, z_bottom: 0, z_top: 10 },
    ],
    opaque: true,
    solid: true,
    mouse: true,
    show_shadow_polygon: true,
    default_material: 0,
    material_indices: [],
    projection_area: null,
  };
  const descriptor: ProjectionAssetDescriptor = {
    version: 1,
    kind: "projection-mapped-asset",
    id: "house",
    name: "House",
    source_map: "Leicester",
    model: "model.glb",
    source_origin_scene: [20, -40, 0],
    source_origin_game: [20, 23, 0],
    parts: [0, 1].map((n) => ({
      node: `building-00${n}`,
      name: n ? "Roof" : "Wall",
      source_obstacle: n,
      obstacle_local_game: structuredClone(obstacle),
      default_hidden: n === 1,
    })),
  };
  const reference: ExternalAssetSource = {
    id: "house",
    descriptor: "3d-assets/house/asset.json",
    model: "3d-assets/house/model.glb",
    descriptor_sha256: "a".repeat(64),
    model_sha256: "b".repeat(64),
  };
  const document: Level3D = {
    version: 1,
    map: "Leicester",
    sceneAssets: [],
    size: [100, 100],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    groups: [],
    objects: [],
  };
  return { descriptor, reference, document };
}

test("standalone insertion creates a complete independent group and preserves default visibility", () => {
  const { descriptor, reference, document } = assetFixture();
  const before = structuredClone(document);
  const one = insertProjectionAsset(document, descriptor, reference, [50, 40, 0]);
  const two = insertProjectionAsset(one.document, descriptor, reference, [80, 20, 3]);
  assert.deepEqual(document, before);
  assert.equal(two.document.assetSources?.length, 1);
  assert.equal(two.document.groups.length, 2);
  assert.equal(new Set(two.document.objects.map((part) => part.id)).size, 4);
  assert.equal(two.document.objects[1].hidden, true);
  assert.equal(two.document.objects[0].node, "asset:house:building-000");
  assert.deepEqual(two.document.groups[0].transform, { dx: 50, dy: 40, dz: 0, rot_deg: 0 });
  two.document.objects[2].obstacle.points[0].x = 99;
  assert.equal(two.document.objects[0].obstacle.points[0].x, 0);
  assert.equal(descriptor.parts[0].obstacle_local_game.points[0].x, 0);
  const moved = patchGroup(one.document, one.selection.id, {
    transform: { ...IDENTITY_TRANSFORM, dx: 7 },
  });
  assert.equal(one.document.groups[0].transform.dx, 50);
  const duplicate = duplicateSelection(moved, one.selection);
  assert.equal(duplicate.document.objects.length, 4);
  assert.equal(deleteSelection(duplicate.document, duplicate.selection).objects.length, 2);
});

test("inserted appearance controls are independent and survive stored-map round trips", () => {
  const { descriptor, reference, document } = assetFixture();
  const ids = { house: ["appearance-1", "appearance-2"] };
  const first = insertProjectionAsset(document, descriptor, reference, [0, 0, 0], [], ids);
  const second = insertProjectionAsset(first.document, descriptor, reference, [50, 0, 0], [], ids);
  const [a, b] = second.document.groups;
  assert.equal(a!.patches!.house!["appearance-1"], `${a!.id}/appearance/house/appearance-1`);
  assert.notEqual(a!.patches!.house!["appearance-1"], b!.patches!.house!["appearance-1"]);
  assert.notEqual(a!.patches!.house!["appearance-1"], a!.patches!.house!["appearance-2"]);
  const descriptors = new Map([[descriptor.id, descriptor]]);
  const saved = serializeStoredMap(second.document, descriptors);
  const reopened = parseStoredMap(saved, descriptors);
  assert.deepEqual(
    reopened.groups.map((g) => g.patches),
    second.document.groups.map((g) => g.patches),
  );
  assert.deepEqual(ids, { house: ["appearance-1", "appearance-2"] });
  assert.throws(
    () =>
      insertProjectionAsset(document, descriptor, reference, [0, 0, 0], [], {
        unknown: ["appearance-1"],
      }),
    /unplaced asset/,
  );
  assert.throws(
    () => insertProjectionAsset(document, descriptor, reference, [0, 0, 0], [], { house: [""] }),
    /Invalid placement appearance/,
  );
});

test("saved asset instances inherit unchanged subparts and retain edited overrides", () => {
  const { descriptor, reference, document } = assetFixture();
  const inserted = insertProjectionAsset(document, descriptor, reference, [50, 40, 0]).document;
  const descriptors = new Map([[descriptor.id, descriptor]]);
  const compact = compactAssetInstances(inserted, descriptors) as Level3D;
  assert.equal(compact.objects[0]!.obstacle, undefined);
  assert.equal(compact.objects[0]!.source, undefined);
  assert.equal(compact.objects[0]!.name, undefined);
  assert.equal(compact.objects[1]!.hidden, undefined);
  assert.deepEqual(hydrateAssetInstances(compact, descriptors), inserted);

  const edited = structuredClone(inserted);
  edited.objects[0]!.obstacle.points[0]!.x = 99;
  edited.objects[0]!.name = "My wall";
  edited.objects[1]!.hidden = false;
  const saved = compactAssetInstances(edited, descriptors) as Level3D;
  assert.deepEqual(saved.objects[0]!.obstacle, edited.objects[0]!.obstacle);
  assert.equal(saved.objects[0]!.name, "My wall");
  assert.equal(saved.objects[1]!.hidden, false);
  assert.deepEqual(hydrateAssetInstances(saved, descriptors), edited);
  delete edited.objects[0]!.name;
  delete edited.objects[1]!.hidden;
  const cleared = compactAssetInstances(edited, descriptors);
  assert.deepEqual(hydrateAssetInstances(cleared, descriptors), edited);
  assert.throws(() => hydrateAssetInstances(compact, new Map()), /Missing pinned asset descriptor/);
});

test("version 2 stores placements and only exceptional part records", () => {
  const { descriptor, reference, document } = assetFixture();
  const placed = insertProjectionAsset(document, descriptor, reference, [50, 40, 0]).document;
  const descriptors = new Map([[descriptor.id, descriptor]]);
  const simple = serializeStoredMap(placed, descriptors) as {
    version: number;
    placements: { parts?: Record<string, unknown>; removed?: string[]; copies?: unknown[] }[];
    objects?: unknown;
  };
  assert.equal(simple.version, 2);
  assert.equal(simple.objects, undefined);
  assert.equal(simple.placements[0]!.parts, undefined);
  assert.deepEqual(parseStoredMap(simple, descriptors), placed);

  const edited = structuredClone(placed);
  edited.groups[0]!.patches = { house: { "appearance-1": "patch-001" } };
  edited.objects[0]!.transform = { ...IDENTITY_TRANSFORM, dx: 7 };
  edited.objects.splice(1, 1);
  const copied = duplicateSelection(edited, { kind: "part", id: edited.objects[0]!.id }).document;
  const stored = serializeStoredMap(copied, descriptors) as typeof simple;
  assert.equal(Object.keys(stored.placements[0]!.parts ?? {}).length, 1);
  assert.equal(stored.placements[0]!.removed?.length, 1);
  assert.equal(stored.placements[0]!.copies?.length, 1);
  assert.deepEqual(parseStoredMap(stored, descriptors), copied);

  const duplicated = duplicateSelection(placed, {
    kind: "group",
    id: placed.groups[0]!.id,
  }).document;
  assert.deepEqual(
    parseStoredMap(serializeStoredMap(duplicated, descriptors), descriptors),
    duplicated,
  );
});

test("saved source pins derive scene selection and resources from the descriptor", () => {
  const { descriptor, reference, document } = assetFixture();
  descriptor.model_scene = "default";
  descriptor.resources = [{ path: "3d-assets/blobs/shared.bin", sha256: "c".repeat(64) }];
  reference.model_scene = "default";
  reference.resources = structuredClone(descriptor.resources);
  const placed = insertProjectionAsset(document, descriptor, reference, [0, 0, 0]).document;
  const descriptors = new Map([[descriptor.id, descriptor]]);
  const ground = {
    ...descriptor,
    id: "ground",
    model: "model.glb",
    editor_usage: "map-background" as const,
    parts: [],
  };
  descriptors.set(ground.id, ground);
  placed.sceneAssets.push({
    id: ground.id,
    role: "ground",
    descriptor: "3d-assets/ground/asset.json",
    descriptor_sha256: "d".repeat(64),
    model: "3d-assets/ground/model.glb",
    model_sha256: "e".repeat(64),
    model_scene: "default",
    resources: structuredClone(ground.resources),
  });
  const saved = serializeStoredMap(placed, descriptors) as {
    assetSources: { model_scene?: string; resources?: unknown }[];
    sceneAssets: { model_scene?: string; resources?: unknown }[];
  };
  assert.equal(saved.assetSources[0]!.model_scene, undefined);
  assert.equal(saved.assetSources[0]!.resources, undefined);
  assert.equal(saved.sceneAssets[0]!.model_scene, undefined);
  assert.equal(saved.sceneAssets[0]!.resources, undefined);
  assert.deepEqual(parseStoredMap(saved, descriptors), placed);
  assert.throws(
    () =>
      parseStoredMap(
        { ...saved, assetSources: [{ ...saved.assetSources[0], model_scene: "wrong" }] },
        descriptors,
      ),
    /scene differs from descriptor/,
  );
});

test("placement sequence defines object order without a separate permutation", () => {
  const { descriptor, reference, document } = assetFixture();
  const first = insertProjectionAsset(document, descriptor, reference, [50, 40, 0]).document;
  const second = insertProjectionAsset(first, descriptor, reference, [80, 20, 0]).document;
  const [firstWall, firstRoof, secondWall, secondRoof] = second.objects;
  const { group: _group, ...loosePart } = structuredClone(firstWall!);
  const loose = { ...loosePart, id: "loose-wall" };
  const interleaved = {
    ...second,
    objects: [firstWall!, loose, secondWall!, firstRoof!, secondRoof!],
  };
  const descriptors = new Map([[descriptor.id, descriptor]]);
  const stored = serializeStoredMap(interleaved, descriptors) as {
    order?: number[];
    placements: { id: string }[];
  };
  assert.equal(stored.order, undefined);
  assert.deepEqual(
    stored.placements.map((placement) => placement.id),
    [first.groups[0]!.id, loose.id, second.groups[1]!.id],
  );
  const restored = parseStoredMap(stored, descriptors);
  assert.deepEqual(
    restored.objects.map((part) => part.id),
    [firstWall!.id, firstRoof!.id, loose.id, secondWall!.id, secondRoof!.id],
  );
  assert.deepEqual(
    [...restored.objects].sort((a, b) => a.id.localeCompare(b.id)),
    [...interleaved.objects].sort((a, b) => a.id.localeCompare(b.id)),
  );
  assert.throws(
    () => parseStoredMap({ ...stored, order: [0, 1, 2, 3, 4] }, descriptors),
    /Obsolete stored object order/,
  );
});

test("saved maps retain reveal labels without legacy game state copies", () => {
  const { document } = assetFixture();
  document.sceneMetadata = {
    assetOrigins: { house: [10, 20, 0] },
    reveal: {
      version: 1,
      source_map: "Derby",
      patches: [{ id: "patch-001", name: "Opened room", state_source_game: { active: true } }],
      mission_patches: [{ id: "mission-001", states: { initial: { frames: [1, 2] } } }],
    },
  };
  const saved = compactAssetInstances(document, new Map()) as Level3D;
  assert.deepEqual(saved.sceneMetadata, {
    reveal: { patches: [{ id: "patch-001", name: "Opened room" }] },
  });
  assert.deepEqual(document.sceneMetadata.assetOrigins, { house: [10, 20, 0] });
  assert.deepEqual(document.sceneMetadata?.reveal, {
    version: 1,
    source_map: "Derby",
    patches: [{ id: "patch-001", name: "Opened room", state_source_game: { active: true } }],
    mission_patches: [{ id: "mission-001", states: { initial: { frames: [1, 2] } } }],
  });
});

test("authored scenery parts insert, save and reload without a game obstacle", () => {
  const { descriptor, reference, document } = assetFixture();
  const scenery: ProjectionAssetDescriptor = {
    ...descriptor,
    parts: [descriptor.parts[0]!, { node: "foliage-oak", name: "Painted tree", scenery: true }],
  };
  const inserted = insertProjectionAsset(document, scenery, reference, [50, 40, 0]).document;
  const tree = inserted.objects.find((part) => part.node === "asset:house:foliage-oak")!;
  assert.equal(tree.kind, "scenery");
  assert.deepEqual(tree.source, { map: "Leicester" });
  assert.equal("obstacle" in tree, false);
  assert.deepEqual(partPivot(tree), [0, 0]);
  assert.throws(() => transformedObstacle(inserted, tree), /no game obstacle/);
  const descriptors = new Map([[scenery.id, scenery]]);
  const compact = compactAssetInstances(inserted, descriptors) as Level3D;
  assert.equal(compact.objects[1]!.kind, undefined);
  assert.deepEqual(hydrateAssetInstances(compact, descriptors), inserted);
  assert.throws(
    () =>
      parseLevel3D({
        ...inserted,
        objects: [{ ...tree, obstacle: descriptor.parts[0]!.obstacle_local_game }],
      }),
    /no game obstacle/,
  );
  assert.throws(
    () => parseLevel3D({ ...inserted, objects: [{ ...tree, kind: "building" }] }),
    /scenery source requires scenery kind/,
  );
  for (const bad of [
    { obstacle_local_game: descriptor.parts[0]!.obstacle_local_game },
    { source_obstacle: 3 },
    { mission_profile: "Map - Tree" },
    { node: "building-009" },
  ])
    assert.throws(() =>
      parseProjectionAssetDescriptor({
        ...scenery,
        parts: [{ ...scenery.parts[1]!, ...bad }],
      }),
    );
});

test("changed revisions and invalid placements fail without edits", () => {
  const { descriptor, reference, document } = assetFixture();

  const inserted = insertProjectionAsset(document, descriptor, reference, [0, 0, 0]);
  assert.throws(
    () =>
      insertProjectionAsset(
        inserted.document,
        descriptor,
        { ...reference, model_sha256: "c".repeat(64) },
        [0, 0, 0],
      ),
    /different revision/,
  );
  assert.throws(
    () => insertProjectionAsset(document, descriptor, reference, [NaN, 0, 0]),
    /placement/,
  );
  assert.equal(document.groups.length, 0);
});

test("map backgrounds cannot be inserted as editable instances", () => {
  const { descriptor, reference, document } = assetFixture();
  const ground = {
    ...descriptor,
    editor_usage: "map-background" as const,
    parts: [],
    components: [{ source_node: "ground" }],
  };
  assert.throws(
    () => insertProjectionAsset(document, ground, reference, [0, 0, 0]),
    /cannot be inserted/,
  );
  assert.equal(document.groups.length, 0);
});

test("shared assets keep source provenance when placed in a different level", () => {
  const { descriptor, reference, document } = assetFixture();
  const result = insertProjectionAsset(
    { ...document, map: "York" },
    descriptor,
    reference,
    [12, 34, 5],
  );
  assert.equal(result.document.map, "York");
  assert.equal(result.document.objects[0].source.map, "Leicester");
  assert.equal(result.document.groups[0].transform.dx, 12);
});

test("inserting a split part retains its scoped footprint and source provenance", () => {
  const { descriptor, reference, document } = assetFixture();
  descriptor.parts = [
    {
      ...descriptor.parts[0],
      node: "building-000--component-west",
      source_obstacle: 0,
      source_components: ["west"],
    },
  ];
  const result = insertProjectionAsset(document, descriptor, reference, [10, 20, 0]);
  assert.deepEqual(result.document.objects[0].source, {
    map: "Leicester",
    obstacle: 0,
    components: ["west"],
  });
  assert.deepEqual(result.document.objects[0].obstacle, descriptor.parts[0].obstacle_local_game);
  assert.equal(result.document.objects[0].node, "asset:house:building-000--component-west");
});

test("authored placement ground height keeps foundations below the requested terrain", () => {
  const { descriptor, reference, document } = assetFixture();
  const authored = parseProjectionAssetDescriptor({
    ...descriptor,
    gameplay: {
      version: 1,
      collision: "none",
      surfaces: [],
      doors: [],
      placementGroundHeight: 50,
    },
  });
  const inserted = insertProjectionAsset(document, authored, reference, [50, 50, 80]);
  assert.equal(inserted.document.groups[0]!.transform.dz, 30);
  assert.equal(document.groups.length, 0);
  const saved = serializeStoredMap(inserted.document, new Map([[authored.id, authored]]));
  const reopened = parseStoredMap(saved, new Map([[authored.id, authored]]));
  assert.equal(reopened.groups[0]!.transform.dz, 30);
  for (const height of [NaN, Infinity, "50"])
    assert.throws(
      () =>
        insertProjectionAsset(
          document,
          parseProjectionAssetDescriptor({
            ...authored,
            gameplay: {
              version: 1,
              collision: "none",
              surfaces: [],
              doors: [],
              placementGroundHeight: height,
            },
          }),
          reference,
          [50, 50, 80],
        ),
      /placement ground height/,
    );
});

test("elevated asset local geometry rests on the requested terrain height", () => {
  const { descriptor, reference, document } = assetFixture();
  for (const part of descriptor.parts)
    for (const p of part.obstacle_local_game!.points) {
      p.z_bottom += 45;
      p.z_top += 45;
    }
  const inserted = insertProjectionAsset(document, descriptor, reference, [50, 50, 80]);
  assert.equal(inserted.document.groups[0]!.transform.dz, 35);
  assert.equal(
    transformedObstacle(inserted.document, inserted.document.objects[0]!).points[0]!.z_bottom,
    80,
  );
});
