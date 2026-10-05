import test from "node:test";
import assert from "node:assert/strict";
import {
  loadMissionStateCatalog,
  loadMissionStatePreview,
  parseMissionStateCatalog,
} from "./mission-state-catalog.ts";
import { missionStateDataHash } from "./mission-state-layer.ts";
import type { StateDeliveryContract } from "../../shared/src/state-delivery.ts";

function library(files: Map<string, string>, prefix = ""): FileSystemDirectoryHandle {
  return {
    async getDirectoryHandle(name: string) {
      const next = prefix + name + "/";
      if (![...files.keys()].some((key) => key.startsWith(next)))
        throw new DOMException("Missing", "NotFoundError");
      return library(files, next);
    },
    async getFileHandle(name: string) {
      const path = prefix + name;
      if (!files.has(path)) throw new DOMException("Missing", "NotFoundError");
      return {
        async getFile() {
          return new File([files.get(path)!], name);
        },
      };
    },
  } as unknown as FileSystemDirectoryHandle;
}
async function fixture() {
  const files = new Map<string, string>();
  const pin = async (path: string, value: unknown) => {
    const text = JSON.stringify(value);
    files.set(path, text);
    return { path, sha256: await missionStateDataHash(value) };
  };
  const target = {
    position_x: 0,
    position_y: 0,
    action_position_x: 0,
    action_position_y: 0,
    polyline: [],
  };
  const data = { targets: [target] },
    level = { animations: [], patches: [] };
  const image = {
    path: "image.png",
    sha256: "a".repeat(64),
    width: 1,
    height: 1,
    offset: [0, 0] as [number, number],
    delay: 1,
  };
  const asset = {
    id: "body",
    role: "objects" as const,
    model: "body.glb",
    model_sha256: "b".repeat(64),
    resources: [],
    position: [10, 20, 30] as [number, number, number],
  };
  const contract: StateDeliveryContract = {
    version: 1,
    scope: "controlled-state-preview",
    native: {
      version: 1,
      mission: "test",
      mission_data_sha256: await missionStateDataHash(data),
      level_data_sha256: await missionStateDataHash(level),
      camera_elevation_deg: 35,
      scope: "map-art-and-listed-effects",
      background: image,
      origin: [0, 0],
      elements: [
        {
          id: "body",
          source: { kind: "mission-target", index: 0, sha256: await missionStateDataHash(target) },
          active: false,
          frames: [image],
          loop: false,
          display_position: [0, 0],
          sort_position: [0, 0],
          display_order: 0,
          creation_order: 0,
          polyline: [],
        },
      ],
    },
    families: [
      {
        id: "trap",
        element_ids: ["body"],
        background_ids: [],
        body_terminal_tick: 0,
        physical: { initial: [asset], applied: [asset] },
      },
    ],
  };
  const entry = {
    id: "trap",
    name: "Trap",
    map: "Croisement02",
    mission: "test",
    contract: await pin("mission-states/trap.json", contract),
    mission_data: await pin("mission-states/source.json", data),
    level_data: await pin("mission-states/level.json", level),
  };
  files.set("mission-states/index.json", JSON.stringify({ version: 1, entries: [entry] }));
  return { files, root: library(files), entry, contract, pin };
}
test("optional catalog absence differs from declared malformed entries", async () => {
  assert.deepEqual(await loadMissionStateCatalog(library(new Map()), "Croisement02", "test"), []);
  const f = await fixture();
  assert.equal((await loadMissionStateCatalog(f.root, "Croisement02", "test")).length, 1);
  assert.equal((await loadMissionStateCatalog(f.root, "Croisement02", "other")).length, 0);
  f.files.set("mission-states/index.json", '{"version":2,"entries":[]}');
  await assert.rejects(
    loadMissionStateCatalog(f.root, "Croisement02", "test"),
    /Invalid mission state catalog/,
  );
  assert.throws(
    () =>
      parseMissionStateCatalog({
        version: 1,
        entries: [{ ...f.entry, contract: { ...f.entry.contract, path: "../escape.json" } }],
      }),
    /Invalid/,
  );
});
test("published preview binds source hashes and explicit reusable asset placement", async () => {
  const f = await fixture();
  const result = await loadMissionStatePreview(f.root, f.entry);
  assert.equal(result.source.name, "test");
  assert.deepEqual(result.contract.families[0]!.physical.initial[0]!.position, [10, 20, 30]);
  f.files.set(f.entry.contract.path, "{}");
  await assert.rejects(loadMissionStatePreview(f.root, f.entry), /State resource changed/);
  delete f.contract.families[0]!.physical.initial[0]!.position;
  f.entry.contract = await f.pin(f.entry.contract.path, f.contract);
  await assert.rejects(loadMissionStatePreview(f.root, f.entry), /explicit local-origin placement/);
});
test("declared missing source and altered source semantics fail explicitly", async () => {
  const f = await fixture();
  f.files.delete(f.entry.mission_data.path);
  await assert.rejects(
    loadMissionStatePreview(f.root, f.entry),
    (error) => error instanceof DOMException && error.name === "NotFoundError",
  );
  f.entry.mission_data = await f.pin(f.entry.mission_data.path, { targets: [] });
  await assert.rejects(loadMissionStatePreview(f.root, f.entry), /source or camera changed/);
});

test("native loop catalog entries validate their own source contract without fabricated physical endpoints", async () => {
  const f = await fixture();
  const native = structuredClone(f.contract.native);
  native.elements[0]!.active = true;
  native.elements[0]!.loop = true;
  const loop = {
    version: 1,
    scope: "controlled-native-loop-preview",
    native,
    focus_element_id: "body",
  };
  const entry = {
    ...f.entry,
    kind: "native-loop" as const,
    contract: await f.pin("mission-states/loop.json", loop),
  };
  const result = await loadMissionStatePreview(f.root, entry);
  assert.equal(result.kind, "native-loop");
  assert.equal("families" in result.contract, false);
  loop.focus_element_id = "missing";
  entry.contract = await f.pin("mission-states/loop.json", loop);
  await assert.rejects(loadMissionStatePreview(f.root, entry), /looping focus/);
  assert.throws(
    () => parseMissionStateCatalog({ version: 1, entries: [{ ...entry, kind: "guess" }] }),
    /catalog/,
  );
});
