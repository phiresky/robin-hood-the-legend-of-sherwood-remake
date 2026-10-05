import test from "node:test";
import { createHash } from "node:crypto";
import assert from "node:assert/strict";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import {
  listProjectionAppearances,
  listProjectionAssets,
  prepareProjectionAsset,
  prepareProjectionPlacement,
  readPinnedAssetDescriptors,
} from "./projection-library.ts";
import { captureLoadedStateAppearance } from "./scene-assets.ts";
import { StateAppearancePlayer } from "./state-appearance-player.ts";
import { disposeObjectResources } from "./resources.ts";
import { insertProjectionAsset } from "./asset-commands.ts";
import { prepareMapCandidate } from "./map-candidate.ts";
import { expandStoredMap, parseStoredMap, serializeStoredMap, type Level3D } from "@rle/shared";
import { authorLightRegionAsset } from "../../pipeline/src/author-light-region-asset.ts";
import { authorAmbientSoundAsset } from "../../pipeline/src/author-ambient-sound-asset.ts";
import { compileMap } from "./map-compile.ts";

function fixture() {
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
  const descriptor = {
    version: 1,
    kind: "projection-mapped-asset",
    id: "house",
    name: "House",
    source_map: "Leicester",
    model: "model.glb",
    source_origin_scene: [20, -40, 0],
    source_origin_game: [20, 23, 0],
    parts: [
      { node: "building-000", name: "Wall", source_obstacle: 0, obstacle_local_game: obstacle },
    ],
  };
  const entry = {
    id: "house",
    name: "House",
    source_map: "Leicester",
    descriptor: "3d-assets/house/asset.json",
    model: "3d-assets/house/model.glb",
  };
  const files = new Map<string, File>();
  const descriptors = new Map<string, unknown>();
  let indexValue: { version: number; assets: Record<string, unknown>[] } | undefined;
  const json = (path: string, value: unknown) => {
    files.set(path, new File([JSON.stringify(value)], path));
    if (path.endsWith("/asset.json")) descriptors.set(path, value);
    if (path === "3d-assets/index.json")
      indexValue = value as { version: number; assets: Record<string, unknown>[] };
    if (!indexValue) return;
    const assets = indexValue.assets.map((asset) => {
      const stored = descriptors.get(`3d-assets/${asset.descriptor}`);
      if (!stored) return asset;
      const encoded = JSON.stringify(stored);
      return {
        ...asset,
        name: (stored as { name: string }).name,
        source_map: (stored as { source_map: string }).source_map,
        ...((stored as { model_scene?: string }).model_scene
          ? { model_scene: (stored as { model_scene: string }).model_scene }
          : {}),
        editor: stored,
        descriptor_sha256: createHash("sha256").update(encoded).digest("hex"),
      };
    });
    files.set(
      "3d-assets/index.json",
      new File([JSON.stringify({ version: 1, assets })], "index.json"),
    );
  };
  json(entry.descriptor, descriptor);
  files.set(entry.model, new File([new Uint8Array([3, 2, 1])], "model.glb"));
  json("3d-assets/index.json", {
    version: 1,
    assets: [
      { ...entry, descriptor: "house/asset.json", model: "house/model.glb" },
      {
        id: "york-house",
        name: "York House",
        source_map: "York",
        descriptor: "york/asset.json",
        model: "york/model.glb",
      },
    ],
  });
  const handle = (prefix: string): FileSystemDirectoryHandle =>
    ({
      async getDirectoryHandle(name: string) {
        const next = `${prefix}${name}/`;
        if (![...files.keys()].some((key) => key.startsWith(next)))
          throw new DOMException(next, "NotFoundError");
        return handle(next);
      },
      async getFileHandle(name: string) {
        const file = files.get(prefix + name);
        if (!file) throw new DOMException(name, "NotFoundError");
        return { getFile: async () => file };
      },
      async *entries() {
        for (const path of files.keys())
          if (path.startsWith(prefix) && !path.slice(prefix.length).includes("/"))
            yield [path.slice(prefix.length), { kind: "file" }];
      },
    }) as unknown as FileSystemDirectoryHandle;
  const asset = new THREE.Group(),
    root = new THREE.Group(),
    group = new THREE.Group();
  root.name = "map";
  group.userData.asset_group = "house";
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  mesh.name = "building-000";
  mesh.userData.source_obstacle = 0;
  asset.add(root);
  root.add(group);
  group.add(mesh);
  let disposed = 0;
  mesh.geometry.addEventListener("dispose", () => disposed++);
  return {
    files,
    json,
    entry,
    descriptor,
    directory: handle(""),
    asset,
    group,
    mesh,
    disposed: () => disposed,
  };
}

test("standalone index filters the current map and actual model parts receive namespaced keys", async (t) => {
  const f = fixture();
  const catalog = await listProjectionAssets(f.directory, "leicester");
  assert.deepEqual(
    catalog.map((entry) => entry.id),
    [f.entry.id],
  );
  assert.deepEqual(catalog[0]!.editor, f.descriptor);
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: f.asset }));
  const prepared = await prepareProjectionAsset(f.directory, f.entry, "Leicester");
  assert.equal(prepared.sources.get("asset:house:building-000"), f.mesh);
  assert.match(prepared.reference.model_sha256, /^[a-f0-9]{64}$/);
  assert.equal(f.disposed(), 0);
  disposeObjectResources([prepared.asset]);
  assert.equal(f.disposed(), 1);
});

test("authored light and sound GLBs load through the editor and retain gameplay on reopen", async () => {
  const options = {
    id: "house",
    name: "Environmental field",
    map: "Leicester",
    origin: [0, 0, 0] as [number, number, number],
  };
  const polygon = {
    points: [
      [0, 0],
      [20, 0],
      [20, 20],
      [0, 20],
    ] as [number, number][],
  };
  const support = {
    ...fixture().descriptor.parts[0]!.obstacle_local_game,
    projection_area: [0, 1] as [number, number],
    points: polygon.points.map(([x, y]) => ({ x, y: y + x / 2, z_bottom: 0, z_top: x / 2 })),
  };
  const light = await authorLightRegionAsset(
    { layer: 1, ambience: 4, polygon },
    [support],
    [{ polygon, is_lift: false, state_id: 0, flags: 0, skeleton_segments: [], obstacles: [] }],
    options,
  );
  assert.ok(light.descriptor.gameplay!.lights![0]!.receiverSegments?.length);
  const sound = await authorAmbientSoundAsset(
    {
      id: 1,
      active: true,
      source_kind: 2,
      delayed_params: [150, 500, 5],
      global: false,
      inner_distance: 10,
      outer_distance: 100,
      polyline: [[10, 10]],
      inner_volume: 100,
      outer_volume: 0,
      noise_covering_distance: 0,
      altitude: 0,
      ambience_filter: 255,
    },
    options,
  );
  for (const authored of [light, sound]) {
    const f = fixture();
    f.json(f.entry.descriptor, authored.descriptor);
    f.files.set(f.entry.model, new File([new Uint8Array(authored.model)], "model.glb"));
    const entry = { ...f.entry, model_scene: "default" };
    const prepared = await prepareProjectionAsset(f.directory, entry, "Leicester");
    const frame = prepared.sources.get(`asset:house:${authored.descriptor.parts[0]!.node}`)!;
    assert.ok(frame);
    assert.equal(frame.userData.gameplay_only, true);
    let meshes = 0;
    prepared.asset.traverse((node) => {
      if ((node as THREE.Mesh).isMesh) meshes++;
    });
    assert.equal(meshes, 0);
    const blank: Level3D = {
      version: 1,
      map: "Example",
      size: [1000, 1000],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      sceneAssets: [],
      groups: [],
      objects: [],
    };
    const placed = insertProjectionAsset(
      blank,
      prepared.descriptor,
      prepared.reference,
      [100, 200, 0],
    ).document;
    const descriptors = new Map([[prepared.descriptor.id, prepared.descriptor]]);
    const reopened = parseStoredMap(serializeStoredMap(placed, descriptors), descriptors);
    assert.equal(reopened.objects[0]!.kind, "scenery");
    assert.deepEqual(reopened.groups[0]!.transform, placed.groups[0]!.transform);
    const reloaded = await prepareProjectionAsset(
      f.directory,
      entry,
      "Leicester",
      prepared.reference,
    );
    assert.deepEqual(reloaded.descriptor, authored.descriptor);
    disposeObjectResources([prepared.asset, reloaded.asset]);
  }
});

test("explicit gameplay-only frames load and reopen without rendering a placeholder mesh", async (t) => {
  const f = fixture();
  const descriptor = {
    ...f.descriptor,
    parts: [
      {
        node: "scenery-navigation-frame",
        name: "Navigation boundary",
        scenery: true,
        gameplay_only: true,
      },
    ],
  };
  f.json(f.entry.descriptor, descriptor);
  f.group.remove(f.mesh);
  const frame = new THREE.Group();
  frame.name = "scenery-navigation-frame";
  frame.userData = { scenery: true, gameplay_only: true };
  f.group.add(frame);
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: f.asset }));
  const prepared = await prepareProjectionAsset(f.directory, f.entry, "Leicester");
  assert.equal(prepared.sources.get("asset:house:scenery-navigation-frame"), frame);
  const blank: Level3D = {
    version: 1,
    map: "Example",
    size: [1000, 1000],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    sceneAssets: [],
    groups: [],
    objects: [],
  };
  const placed = insertProjectionAsset(
    blank,
    prepared.descriptor,
    prepared.reference,
    [100, 200, 0],
  ).document;
  const descriptors = new Map([[prepared.descriptor.id, prepared.descriptor]]);
  const restored = parseStoredMap(serializeStoredMap(placed, descriptors), descriptors);
  assert.equal(restored.objects[0]!.kind, "scenery");
  assert.equal(restored.objects[0]!.obstacle, undefined);
  assert.deepEqual(restored.groups[0]!.transform, placed.groups[0]!.transform);
  frame.add(f.mesh);
  await assert.rejects(
    prepareProjectionAsset(f.directory, f.entry, "Leicester"),
    /Invalid gameplay-only frame/,
  );
  frame.remove(f.mesh);
  f.json(f.entry.descriptor, {
    ...descriptor,
    parts: [{ ...descriptor.parts[0], gameplay_only: undefined }],
  });
  await assert.rejects(
    prepareProjectionAsset(f.directory, f.entry, "Leicester"),
    /Standalone part has no mesh/,
  );
});

test("palette assets load from the index without reading descriptors", async (t) => {
  const f = fixture();
  f.files.delete(f.entry.descriptor);
  assert.deepEqual(
    (await listProjectionAssets(f.directory, "Leicester")).map((entry) => entry.id),
    [f.entry.id],
  );
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: f.asset }));
  const prepared = await prepareProjectionAsset(f.directory, f.entry, "Leicester");
  assert.equal(prepared.descriptor.parts[0]!.node, "building-000");
  disposeObjectResources([prepared.asset]);
});

test("descriptor changes warn, model changes reject, and bad model cleanup is owned", async (t) => {
  const f = fixture();
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: f.asset }));
  const prepared = await prepareProjectionAsset(f.directory, f.entry, "Leicester");
  assert.equal(
    (await prepareProjectionAsset(f.directory, f.entry, "York")).descriptor.source_map,
    "Leicester",
  );
  await assert.rejects(
    prepareProjectionAsset(f.directory, f.entry, "Leicester", {
      ...prepared.reference,
      model_sha256: "c".repeat(64),
    }),
    /model changed/,
  );
  f.json(f.entry.descriptor, { ...f.descriptor, name: "Edited" });
  const warn = t.mock.method(console, "warn", () => {});
  const updated = await prepareProjectionAsset(
    f.directory,
    f.entry,
    "Leicester",
    prepared.reference,
  );
  assert.equal(updated.descriptor.name, "Edited");
  assert.match(warn.mock.calls[0].arguments[0], /Asset descriptor changed: house/);
  const warnings: string[] = [];
  const descriptors = await readPinnedAssetDescriptors(
    f.directory,
    [prepared.reference],
    [],
    (message) => warnings.push(message),
  );
  assert.equal(descriptors.get("house")!.name, "Edited");
  assert.match(warnings[0], /Asset descriptor changed: house/);
  await assert.rejects(
    readPinnedAssetDescriptors(f.directory, [{ ...prepared.reference, id: "missing" }]),
    /Missing asset descriptor/,
  );
  await assert.rejects(
    readPinnedAssetDescriptors(f.directory, [
      { ...prepared.reference, descriptor: "3d-assets/moved.json" },
    ]),
    /descriptor path changed/,
  );
  disposeObjectResources([prepared.asset]);
  const bad = fixture();
  bad.mesh.userData.source_obstacle = 12;
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: bad.asset }));
  await assert.rejects(prepareProjectionAsset(bad.directory, bad.entry, "Leicester"), /Unexpected/);
  assert.equal(bad.disposed(), 1);
});

test("saved version 2 asset placements reload before document validation", async (t) => {
  const f = fixture();
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: f.asset }));
  const prepared = await prepareProjectionAsset(f.directory, f.entry, "Leicester");
  const base: Level3D = {
    version: 1,
    map: "York",
    size: [100, 100],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    sceneAssets: [],
    groups: [],
    objects: [],
  };
  const inserted = insertProjectionAsset(
    base,
    prepared.descriptor,
    prepared.reference,
    [50, 50, 0],
  );
  inserted.document.objects[0]!.obstacle.points[0]!.x = 1;
  const compact = serializeStoredMap(
    inserted.document,
    new Map([[prepared.descriptor.id, prepared.descriptor]]),
  );
  assert.equal((compact as { version: number }).version, 2);
  assert.equal("objects" in (compact as object), false);
  f.json("scenes/York.rhlos-map.json", compact);
  f.json("scenes/York-volumes.scene.json", {
    version: 1,
    map: "York",
    size: [100, 100],
    camera: base.camera,
    placements: [],
  });
  f.files.set("scenes/York-volumes.scene.glb", new File([new Uint8Array([7])], "map.glb"));
  f.files.delete(f.entry.descriptor);
  let calls = 0;
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => {
    calls++;
    return { scene: f.asset };
  });
  const candidate = await prepareMapCandidate("York", f.directory, null);
  assert.equal(calls, 1);
  assert.equal(candidate.sources.get("asset:house:building-000"), f.mesh);
  assert.equal(candidate.document.groups[0].transform.dx, 50);
  assert.equal(candidate.document.objects[0]!.obstacle.points[0]!.x, 1);
  assert.deepEqual(candidate.document.assetSources, [prepared.reference]);
  assert.deepEqual(candidate.warnings, []);
  f.json(f.entry.descriptor, { ...f.descriptor, name: "Updated house" });
  const updated = await prepareMapCandidate("York", f.directory, null);
  assert.match(updated.warnings[0], /Asset descriptor changed: house/);
  assert.equal(updated.sources.get("asset:house:building-000"), f.mesh);
  assert.equal(updated.document.objects[0]!.obstacle.points[0]!.x, 1);
  disposeObjectResources([candidate.asset, updated.asset]);
  assert.equal(f.disposed(), 1);
});

test("static model variants load one endpoint, retain endpoint obstacles, and coexist on save/reload", async (t) => {
  const f = fixture();
  const appliedParts = [
    {
      ...f.descriptor.parts[0],
      name: "Lowered deck",
      obstacle_local_game: {
        ...f.descriptor.parts[0].obstacle_local_game,
        solid: false,
      },
    },
  ];
  f.json(f.entry.descriptor, {
    ...f.descriptor,
    state_variants: {
      initial: { name: "Raised", model: "model.glb" },
      applied: { name: "Lowered", model: "lowered.glb", parts: appliedParts },
    },
  });
  f.files.set("3d-assets/house/lowered.glb", new File([new Uint8Array([8, 9])], "lowered.glb"));
  const palette = await listProjectionAssets(f.directory, "Leicester");
  assert.deepEqual(
    palette.map((entry) => entry.name),
    ["House"],
  );
  const entries = await listProjectionAppearances(f.directory, palette[0]!);
  assert.deepEqual(
    entries.map((entry) => entry.name),
    ["Raised", "Lowered"],
  );
  const loaded: number[][] = [];
  t.mock.method(GLTFLoader.prototype, "parseAsync", async (bytes: ArrayBuffer) => {
    loaded.push([...new Uint8Array(bytes)]);
    return { scene: f.asset };
  });
  const raised = await prepareProjectionAsset(f.directory, entries[0], "Leicester");
  const lowered = await prepareProjectionAsset(f.directory, entries[1], "Leicester");
  assert.deepEqual(loaded, [
    [3, 2, 1],
    [8, 9],
  ]);
  assert.equal(lowered.reference.state_variant, "applied");
  assert.equal(lowered.descriptor.parts[0].obstacle_local_game.solid, false);
  assert.ok(lowered.sources.has("asset:house--state-applied:building-000"));
  let document: Level3D = {
    version: 1,
    map: "Leicester",
    size: [100, 100],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    sceneAssets: [],
    groups: [],
    objects: [],
  };
  for (const prepared of [raised, lowered])
    document = insertProjectionAsset(
      document,
      prepared.descriptor,
      prepared.reference,
      [50, 50, 0],
    ).document;
  assert.equal(document.assetSources!.length, 2);
  assert.notEqual(document.objects[0].node, document.objects[1].node);
  const descriptors = new Map([
    [raised.descriptor.id, raised.descriptor],
    [lowered.descriptor.id, lowered.descriptor],
  ]);
  const stored = serializeStoredMap(document, descriptors) as {
    assetSources: { id: string; appearances?: unknown[] }[];
    placements: { assets: string[]; appearances?: Record<string, string[]> }[];
  };
  assert.deepEqual(
    stored.assetSources.map((source) => source.id),
    ["house"],
  );
  assert.equal(stored.assetSources[0]!.appearances?.length, 2);
  assert.deepEqual(
    stored.placements.map((placement) => placement.assets),
    [["house"], ["house"]],
  );
  assert.deepEqual(stored.placements[1]!.appearances, { house: ["applied"] });
  assert.deepEqual((expandStoredMap(stored).assetSources as unknown[]).length, 2);
  assert.deepEqual(parseStoredMap(stored, descriptors), document);
  const appliedOnly = insertProjectionAsset(
    {
      ...document,
      groups: [],
      objects: [],
      assetSources: [],
    },
    lowered.descriptor,
    lowered.reference,
    [0, 0, 0],
  ).document;
  const appliedStored = serializeStoredMap(appliedOnly, descriptors) as typeof stored;
  assert.deepEqual(
    appliedStored.assetSources.map((source) => source.id),
    ["house"],
  );
  assert.deepEqual(appliedStored.placements[0]!.appearances, { house: ["applied"] });
  assert.deepEqual(parseStoredMap(appliedStored, descriptors), appliedOnly);
  assert.throws(
    () => parseStoredMap({ ...stored, assetSources: document.assetSources }, descriptors),
    /Separate appearance source is unsupported/,
  );
  const reloaded = await prepareProjectionAsset(
    f.directory,
    lowered.reference,
    "Leicester",
    lowered.reference,
  );
  assert.deepEqual(reloaded.reference, lowered.reference);
  await assert.rejects(
    prepareProjectionAsset(f.directory, { ...entries[1], model: f.entry.model }, "Leicester"),
    /path mismatch/,
  );
  f.files.set(lowered.reference.model, new File([new Uint8Array([7])], "lowered.glb"));
  await assert.rejects(
    prepareProjectionAsset(f.directory, lowered.reference, "Leicester", lowered.reference),
    /model changed/,
  );
});

test("gameplay endpoint insertion loads both models, saves both pins and compiles independent copies", async (t) => {
  const f = fixture(),
    applied = fixture();
  const part = { ...f.descriptor.parts[0]!, node: "building-001", source_obstacle: 1 };
  const descriptor = {
    ...f.descriptor,
    state_variants: {
      initial: { name: "Raised", model: "model.glb" },
      applied: { name: "Lowered", model: "lowered.glb", parts: [part] },
    },
    gameplay: {
      version: 1,
      collision: "none",
      doors: [],
      surfaces: [
        {
          id: "ground",
          node: "building-000",
          polygon: [
            [0, 0],
            [10, 0],
            [0, 10],
          ],
          height: 0,
        },
      ],
      movementTransitions: [
        {
          id: "bridge",
          node: "building-000",
          waypoint: [1, 1, 0],
          active: true,
          definitive: false,
          initial: [],
          applied: [],
          applyPolygon: [],
          noApplyPolygon: [],
          appearances: ["state"],
        },
      ],
    },
  };
  f.json(f.entry.descriptor, descriptor);
  f.files.set("3d-assets/house/lowered.glb", new File([new Uint8Array([8, 9])], "lowered.glb"));
  applied.mesh.name = part.node;
  applied.mesh.userData.source_obstacle = 1;
  let calls = 0;
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({
    scene: calls++ === 0 ? f.asset : applied.asset,
  }));
  const entry = (await listProjectionAssets(f.directory, "Leicester"))[0]!;
  assert.deepEqual(
    (await listProjectionAppearances(f.directory, entry)).map((e) => e.id),
    ["house"],
  );
  const prepared = await prepareProjectionPlacement(f.directory, entry, "New map");
  assert.equal(prepared.additionalAssets.length, 1);
  assert.equal(prepared.sources.size, 2);
  const empty: Level3D = {
    version: 1,
    map: "New map",
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    size: [2000, 2000],
    objects: [],
    groups: [],
    sceneAssets: [],
  };
  const place = (document: Level3D, x: number) =>
    insertProjectionAsset(
      document,
      prepared.descriptor,
      prepared.reference,
      [x, 100, 0],
      prepared.additionalAssets,
    ).document;
  assert.throws(
    () => insertProjectionAsset(empty, prepared.descriptor, prepared.reference, [100, 100, 0]),
    /pinned applied model/,
  );
  const once = place(empty, 100),
    twice = place(once, 200);
  assert.equal(twice.assetSources!.length, 2);
  assert.equal(twice.objects.length, 4);
  assert.ok(twice.groups.every((g) => g.states === undefined && g.patches?.house?.state));
  const descriptors = new Map([
    [prepared.reference.id, prepared.descriptor],
    ...prepared.additionalAssets.map((m) => [m.reference.id, m.descriptor] as const),
  ]);
  assert.deepEqual(parseStoredMap(serializeStoredMap(twice, descriptors), descriptors), twice);
  assert.equal(
    compileMap(twice, [0, 0, 2000, 2000], descriptors).descriptor.asset_geometry!
      .movement_transitions!.length,
    2,
  );
  const bad = structuredClone(prepared.additionalAssets);
  bad[0]!.reference.model_sha256 = "c".repeat(64);
  assert.throws(
    () => insertProjectionAsset(once, prepared.descriptor, prepared.reference, [300, 100, 0], bad),
    /different revision/,
  );
  disposeObjectResources([prepared.asset]);
  assert.equal(f.disposed(), 1);
  assert.equal(applied.disposed(), 1);
  const failed = fixture();
  failed.json(failed.entry.descriptor, descriptor);
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: failed.asset }));
  const failedEntry = (await listProjectionAssets(failed.directory, "Leicester"))[0]!;
  await assert.rejects(prepareProjectionPlacement(failed.directory, failedEntry, "New map"));
  assert.equal(
    failed.disposed(),
    1,
    "a missing applied model must retire the prepared initial model",
  );
});

for (const variants of ["state_variants", "standalone_variants"] as const)
  for (const state of ["initial", "applied"] as const)
    test(`saved ${variants} ${state} reloads with the model's base group identity`, async (t) => {
      const f = fixture();
      f.json(f.entry.descriptor, {
        ...f.descriptor,
        [variants]: {
          [state]: {
            name: "Endpoint",
            model: "model.glb",
            parts: [
              {
                ...f.descriptor.parts[0],
                obstacle_local_game: {
                  ...f.descriptor.parts[0].obstacle_local_game,
                  solid: false,
                },
              },
            ],
          },
        },
      });
      t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: f.asset }));
      const [catalog] = await listProjectionAssets(f.directory, "Leicester");
      const entries = await listProjectionAppearances(f.directory, catalog);
      const entry = entries.find((entry) => entry.state_variant === state)!;
      const prepared = await prepareProjectionAsset(f.directory, entry, "Leicester");
      const blank: Level3D = {
        version: 1,
        map: "Leicester",
        size: [100, 100],
        camera: { kind: "oblique-orthographic", elevation_deg: 35 },
        sceneAssets: [],
        groups: [],
        objects: [],
      };
      const { document } = insertProjectionAsset(
        blank,
        prepared.descriptor,
        prepared.reference,
        [50, 50, 0],
      );
      f.json(
        "scenes/Leicester.rhlos-map.json",
        serializeStoredMap(document, new Map([[entry.id, prepared.descriptor]])),
      );
      const candidate = await prepareMapCandidate("Leicester", f.directory, null);
      assert.deepEqual(candidate.document, document);
      assert.equal(candidate.sources.get(`asset:${entry.id}:building-000`), f.mesh);
      assert.equal(candidate.document.objects[0].obstacle.solid, false);
      // A different asset's group must still be rejected by the pinned loading path.
      f.group.userData.asset_group = "other-house";
      await assert.rejects(
        prepareMapCandidate("Leicester", f.directory, null),
        /Standalone group mismatch/,
      );
    });

test("supplemental mission models retain profile provenance without inventing an obstacle index", async (t) => {
  const f = fixture();
  const { source_obstacle: _source_obstacle, ...part } = f.descriptor.parts[0];
  const mission = {
    ...part,
    node: "mission-second-drawbridge",
    mission_profile: "Derby - Pont_levis02",
  };
  f.json(f.entry.descriptor, { ...f.descriptor, parts: [mission] });
  f.mesh.name = mission.node;
  delete f.mesh.userData.source_obstacle;
  f.mesh.userData.mission_patch_profile = mission.mission_profile;
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: f.asset }));
  const prepared = await prepareProjectionAsset(f.directory, f.entry, "Leicester");
  const base: Level3D = {
    version: 1,
    map: "Leicester",
    size: [100, 100],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    sceneAssets: [],
    groups: [],
    objects: [],
  };
  const result = insertProjectionAsset(base, prepared.descriptor, prepared.reference, [0, 0, 0]);
  assert.equal(result.document.objects[0].kind, "mission");
  assert.deepEqual(result.document.objects[0].source, {
    map: "Leicester",
    mission_profile: mission.mission_profile,
  });
  f.mesh.userData.source_obstacle = 267;
  await assert.rejects(prepareProjectionAsset(f.directory, f.entry, "Leicester"), /Unexpected/);
});

test("manifest mission metadata preserves sources and saved deletions remain deleted", async (t) => {
  const f = fixture();
  f.group.userData.asset_name = "House";
  f.mesh.userData.part_name = "Wall";
  const bridgeGroup = new THREE.Group();
  bridgeGroup.userData = { asset_group: "second-drawbridge", asset_name: "Second drawbridge" };
  const bridge = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  bridge.name = "mission-second-drawbridge";
  bridge.userData = {
    part_name: "Raised endpoint",
    mission_patch_profile: "Derby - Pont_levis02",
    obstacle_local_game: f.descriptor.parts[0].obstacle_local_game,
  };
  bridgeGroup.add(bridge);
  f.asset.children[0].add(bridgeGroup);
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: f.asset }));
  const transform = { dx: 0, dy: 0, dz: 0, rot_deg: 0 };
  f.files.set("3d-assets/base.glb", new File([new Uint8Array([7])], "base.glb"));
  f.json("scenes/Leicester.rhlos-map.json", {
    version: 1,
    map: "Leicester",
    sourceMap: "Leicester",
    size: [100, 100],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    sceneAssets: [
      {
        id: "base",
        role: "objects",
        model: "3d-assets/base.glb",
        model_sha256: createHash("sha256")
          .update(new Uint8Array([7]))
          .digest("hex"),
        resources: [],
      },
    ],
    groups: [
      { id: "house", transform },
      { id: "second-drawbridge", transform },
    ],
    objects: [
      {
        id: "building-000",
        node: "building-000",
        kind: "building",
        group: "house",
        transform,
        source: { map: "Leicester", obstacle: 0 },
        obstacle: f.descriptor.parts[0].obstacle_local_game,
      },
      {
        id: bridge.name,
        node: bridge.name,
        kind: "mission",
        group: "second-drawbridge",
        transform,
        source: { map: "Leicester", mission_profile: "Derby - Pont_levis02" },
        obstacle: bridge.userData.obstacle_local_game,
      },
    ],
  });
  f.json("Leicester.rhp.json", {
    format: "Fullgame",
    misc: {},
    sight_obstacles: [f.descriptor.parts[0].obstacle_local_game],
    patches: [],
    animations: [],
    material_sectors: [],
    light_sectors: [],
    elevation_lines: [],
    masks: [],
    sound_sources: [],
    jump_zones: [],
    jump_line_pairs: [],
    lifts: [],
    buildings: [],
    motion_data: { layers: [], graph_bytes: [] },
  });
  const candidate = await prepareMapCandidate("Leicester", f.directory, {
    maps: new Set(["Leicester"]),
    levelsDir: f.directory,
  });
  assert.equal(candidate.document.objects.length, 2);
  assert.equal(candidate.document.groups.length, 2);
  assert.equal(candidate.sources.get(bridge.name), bridge);
  const part = candidate.document.objects.find((object) => object.kind === "mission")!;
  assert.deepEqual(part.source, { map: "Leicester", mission_profile: "Derby - Pont_levis02" });
  assert.equal(part.group, "second-drawbridge");
  f.json("scenes/Leicester.rhlos-map.json", candidate.document);
  const saved = await prepareMapCandidate("Leicester", f.directory, null);
  assert.equal(saved.document.objects.length, 2);
  f.json("scenes/Leicester.rhlos-map.json", {
    ...candidate.document,
    objects: candidate.document.objects.map((object) =>
      object.kind === "mission"
        ? { ...object, source: { map: "Leicester", mission_profile: "Wrong profile" } }
        : object,
    ),
  });
  await assert.rejects(
    prepareMapCandidate("Leicester", f.directory, null),
    /Mission source profile mismatch/,
  );
  f.json("scenes/Leicester.rhlos-map.json", {
    ...candidate.document,
    objects: candidate.document.objects.filter((object) => object.kind !== "mission"),
  });
  assert.equal(
    (await prepareMapCandidate("Leicester", f.directory, null)).document.objects.length,
    1,
  );
  f.json("scenes/Leicester.rhlos-map.json", candidate.document);
  bridge.userData.source_obstacle = 267;
  await assert.rejects(
    prepareMapCandidate("Leicester", f.directory, null),
    /Source obstacle mismatch/,
  );
});

test("shared catalog lists assets from every source level", async () => {
  const f = fixture();
  f.json("3d-assets/york/asset.json", {
    ...f.descriptor,
    id: "york-house",
    name: "York House",
    source_map: "York",
  });
  const entries = await listProjectionAssets(f.directory);
  assert.deepEqual(
    entries.map((entry) => entry.source_map),
    ["Leicester", "York"],
  );
});

test("standalone component metadata must match the pinned scoped descriptor", async (t) => {
  const f = fixture();
  const name = "building-000--component-west";
  f.json(f.entry.descriptor, {
    ...f.descriptor,
    parts: [{ ...f.descriptor.parts[0], node: name, source_components: ["west"] }],
  });
  f.mesh.name = name;
  f.mesh.userData.source_components = ["west"];
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: f.asset }));
  const prepared = await prepareProjectionAsset(f.directory, f.entry, "York");
  assert.ok(prepared.sources.has("asset:house:" + name));
  f.mesh.userData.source_components = ["east"];
  await assert.rejects(prepareProjectionAsset(f.directory, f.entry, "York"), /Unexpected/);
});

test("additional complete variants retain the covered base and pin each endpoint on reload", async (t) => {
  const f = fixture();
  const endpointParts = [
    {
      ...f.descriptor.parts[0],
      name: "Open door",
      obstacle_local_game: { ...f.descriptor.parts[0].obstacle_local_game, solid: false },
    },
  ];
  f.json(f.entry.descriptor, {
    ...f.descriptor,
    standalone_variants: {
      initial: { name: "Door closed", model: "closed.glb" },
      applied: { name: "Door open", model: "open.glb", parts: endpointParts },
    },
  });
  f.files.set("3d-assets/house/closed.glb", new File([new Uint8Array([4])], "closed.glb"));
  f.files.set("3d-assets/house/open.glb", new File([new Uint8Array([5])], "open.glb"));
  const palette = await listProjectionAssets(f.directory, "Leicester");
  assert.deepEqual(
    palette.map((entry) => entry.id),
    ["house"],
  );
  const entries = await listProjectionAppearances(f.directory, palette[0]!);
  assert.deepEqual(
    entries.map((entry) => entry.id),
    ["house", "house--state-initial", "house--state-applied"],
  );
  assert.equal(entries[0].model, f.entry.model);
  const loaded: number[][] = [];
  t.mock.method(GLTFLoader.prototype, "parseAsync", async (bytes: ArrayBuffer) => {
    loaded.push([...new Uint8Array(bytes)]);
    return { scene: f.asset };
  });
  const base = await prepareProjectionAsset(f.directory, entries[0], "York");
  const initial = await prepareProjectionAsset(f.directory, entries[1], "York");
  const applied = await prepareProjectionAsset(f.directory, entries[2], "York");
  assert.deepEqual(loaded, [[3, 2, 1], [4], [5]]);
  assert.equal(base.reference.state_variant, undefined);
  assert.equal(initial.reference.state_variant, "initial");
  assert.equal(applied.descriptor.parts[0].obstacle_local_game.solid, false);
  assert.deepEqual(
    (await prepareProjectionAsset(f.directory, applied.reference, "York", applied.reference))
      .reference,
    applied.reference,
  );
  await assert.rejects(
    prepareProjectionAsset(f.directory, { ...entries[2], model: f.entry.model }, "York"),
    /path mismatch/,
  );
});

test("scene selectors are descriptor-bound and pinned in saved references", async (t) => {
  const f = fixture();
  f.json(f.entry.descriptor, {
    ...f.descriptor,
    model_scene: "base",
    standalone_variants: {
      initial: { name: "Closed", model: "model.glb", model_scene: "closed" },
      applied: { name: "Open", model: "model.glb", model_scene: "open" },
    },
  });
  f.json("3d-assets/index.json", {
    version: 1,
    assets: [
      {
        ...f.entry,
        descriptor: "house/asset.json",
        model: "house/model.glb",
        model_scene: "base",
        preview_model: "house/preview.glb",
      },
    ],
  });
  const palette = await listProjectionAssets(f.directory);
  assert.deepEqual(
    palette.map((entry) => entry.model_scene),
    ["base"],
  );
  const entries = await listProjectionAppearances(f.directory, palette[0]!);
  assert.deepEqual(
    entries.map((entry) => entry.model_scene),
    ["base", "closed", "open"],
  );
  assert.equal(entries[2].preview_model, "3d-assets/house/preview.glb");
  const json = new TextEncoder().encode(
    JSON.stringify({
      asset: { version: "2.0" },
      scenes: [{ name: "base" }, { name: "closed" }, { name: "open" }],
    }),
  );
  const bytes = new Uint8Array(20 + Math.ceil(json.length / 4) * 4);
  const header = new DataView(bytes.buffer);
  header.setUint32(0, 0x46546c67, true);
  header.setUint32(4, 2, true);
  header.setUint32(8, bytes.length, true);
  header.setUint32(12, bytes.length - 20, true);
  header.setUint32(16, 0x4e4f534a, true);
  bytes.fill(32, 20);
  bytes.set(json, 20);
  f.files.set(f.entry.model, new File([bytes], "model.glb"));
  t.mock.method(GLTFLoader.prototype, "parseAsync", async (buffer: ArrayBuffer) => {
    const view = new DataView(buffer),
      parsed = JSON.parse(
        new TextDecoder().decode(new Uint8Array(buffer, 20, view.getUint32(12, true))),
      );
    assert.equal(parsed.scenes.length, 1);
    assert.equal(parsed.scenes[0].name, "open");
    return { scene: f.asset };
  });
  const prepared = await prepareProjectionAsset(f.directory, entries[2], "York");
  assert.equal(prepared.reference.model_scene, "open");
  const blank: Level3D = {
    version: 1,
    map: "York",
    size: [100, 100],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    sceneAssets: [],
    groups: [],
    objects: [],
  };
  const document = insertProjectionAsset(
    blank,
    prepared.descriptor,
    prepared.reference,
    [0, 0, 0],
  ).document;
  assert.throws(
    () =>
      insertProjectionAsset(
        document,
        prepared.descriptor,
        { ...prepared.reference, model_scene: "closed" },
        [0, 0, 0],
      ),
    /different revision/,
  );
  assert.deepEqual(
    (await prepareProjectionAsset(f.directory, prepared.reference, "York", prepared.reference))
      .reference,
    prepared.reference,
  );
  await assert.rejects(
    prepareProjectionAsset(f.directory, { ...entries[2], model_scene: "closed" }, "York"),
    /scene mismatch/,
  );
  await assert.rejects(
    prepareProjectionAsset(f.directory, entries[2], "York", {
      ...prepared.reference,
      model_scene: "closed",
    }),
    /saved reference mismatch/,
  );
  await assert.rejects(
    prepareProjectionAsset(f.directory, entries[2], "York", {
      ...prepared.reference,
      model_sha256: "c".repeat(64),
    }),
    /model changed/,
  );
});

test("indexed lossy models load without receipts or published model reads on reload", async (t) => {
  const f = fixture();
  const sha = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
  const published = new Uint8Array([3, 2, 1]),
    lossy = new Uint8Array([9, 9]);
  f.json("3d-assets/index.json", {
    version: 1,
    assets: [
      {
        ...f.entry,
        descriptor: "house/asset.json",
        model: "house/model.glb",
        lossy_model: "house/lossy.glb",
      },
    ],
  });
  f.files.set("3d-assets/house/lossy.glb", new File([lossy], "lossy.glb"));
  const [entry] = await listProjectionAssets(f.directory);
  assert.equal(entry.lossy_model, "3d-assets/house/lossy.glb");
  const loaded: number[][] = [];
  t.mock.method(GLTFLoader.prototype, "parseAsync", async (bytes: ArrayBuffer) => {
    loaded.push([...new Uint8Array(bytes)]);
    return { scene: f.asset };
  });
  // A new insertion pins the published hash and displays the lossy model.
  const inserted = await prepareProjectionAsset(f.directory, entry, "Leicester");
  assert.equal(inserted.reference.model_sha256, sha(published));
  assert.equal(inserted.reference.model, f.entry.model);
  // A saved pin never reads the published model while its lossy model is current.
  f.files.delete(f.entry.model);
  const reloaded = await prepareProjectionAsset(
    f.directory,
    entry,
    "Leicester",
    inserted.reference,
  );
  assert.deepEqual(reloaded.reference, inserted.reference);
  assert.deepEqual(loaded, [
    [9, 9],
    [9, 9],
  ]);
  // A missing indexed derivative is an error.
  f.files.delete("3d-assets/house/lossy.glb");
  await assert.rejects(
    prepareProjectionAsset(f.directory, entry, "Leicester", inserted.reference),
    /lossy\.glb/,
  );
});

test("published catalog supports insertion without original models or receipts", async (t) => {
  const f = fixture();
  const model_sha256 = createHash("sha256")
    .update(new Uint8Array([3, 2, 1]))
    .digest("hex");
  f.json("3d-assets/index.json", {
    version: 1,
    assets: [
      {
        ...f.entry,
        descriptor: "house/asset.json",
        model: "house/model.glb",
        lossy_model: "house/lossy.glb",
        model_sha256,
      },
    ],
  });
  f.files.delete(f.entry.model);
  f.files.set("3d-assets/house/lossy.glb", new File([new Uint8Array([9, 9])], "lossy.glb"));
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: f.asset }));
  const [entry] = await listProjectionAssets(f.directory);
  const inserted = await prepareProjectionAsset(f.directory, entry, "Leicester");
  assert.equal(inserted.reference.model_sha256, model_sha256);
  await assert.rejects(
    prepareProjectionAsset(f.directory, entry, "Leicester", {
      ...inserted.reference,
      model_sha256: "0".repeat(64),
    }),
    /model changed/,
  );
});

test("projection direct and shared loaders retain clips on the extracted logical group", async (t) => {
  for (const shared of [false, true]) {
    const f = fixture();
    if (shared) f.json(f.entry.descriptor, { ...f.descriptor, resources: [] });
    const node = new THREE.Object3D();
    node.name = "phase_unique";
    f.mesh.add(node);
    const clip = new THREE.AnimationClip("native", 0.08, [
      new THREE.VectorKeyframeTrack(
        "phase_unique.scale",
        [0, 0.08],
        [1, 1, 1, 2, 2, 2],
        THREE.InterpolateDiscrete,
      ),
    ]);
    const mock = t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({
      scene: f.asset,
      animations: [clip],
      parser: {
        associations: new Map([[node, { nodes: 0 }]]),
        json: { nodes: [{ name: "restored/phase" }] },
      },
    }));
    const prepared = await prepareProjectionAsset(f.directory, f.entry, "Leicester");
    assert.equal(node.name, "restored/phase");
    assert.equal(prepared.asset.animations.length, 1);
    f.group.removeFromParent();
    const player = new StateAppearancePlayer(captureLoadedStateAppearance(f.group)!);
    player.select("native", { mode: "clamp", terminalTick: 2 });
    player.seek(2);
    assert.equal(player.content.getObjectByName("restored/phase")!.scale.x, 2);
    assert.equal(node.scale.x, 1);
    player.dispose();
    disposeObjectResources([prepared.asset, f.group]);
    mock.mock.restore();
  }
});
