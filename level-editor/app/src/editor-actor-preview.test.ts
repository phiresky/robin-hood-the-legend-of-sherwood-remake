import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { EditorActorPreview, type EditorActorBinding } from "./editor-actor-preview.ts";
import type { NativeStatePresentation } from "./native-state-presentation.ts";
import type { MissionSpritePreview } from "./mission.ts";
const root = resolve(import.meta.dirname, "../../..");
const read = (path: string) => JSON.parse(readFileSync(resolve(root, path), "utf8"));
const contract = read(
  "level-editor/library/mission-states/croisement02/contracts/signposts.json",
).native;
const source = {
  name: "S03_FoB_MP",
  data: read("level-editor/library/mission-states/croisement02/source/S03_FoB_MP.rhm.json"),
  level: read("level-editor/library/mission-states/croisement02/source/Croisement02.rhp.json"),
  camera: { kind: "oblique-orthographic" as const, elevation_deg: 35 },
};
const native = { loopSourceBinding: () => contract } as NativeStatePresentation;
const binding: EditorActorBinding = {
  identity: "soldiers:0",
  preview: { kind: "editable", editorId: "import-soldier-0" },
  active: true,
  layer: 0,
  drawHidden: false,
  outlineColor: 0xf800,
};
const angle = (35 * Math.PI) / 180;
const sprite: MissionSpritePreview = {
  localIndex: 900,
  position: [100.75, 20 / Math.cos(angle), 220 / Math.sin(angle)],
  direction: 7,
  visible: true,
  bounds: { left: -10.5, top: 30.25, width: 30, height: 40 },
  frame: {
    resourceId: 1,
    filename: "fixture",
    profile: "fixture",
    action: 3,
    direction: 7,
    frame: 0,
    legacy: true,
    pixels: { width: 30, height: 40, data: new Uint8Array(30 * 40 * 4) },
  },
};
const frames = () => ({
  editable: [{ editorId: "import-soldier-0", sprite: structuredClone(sprite) }],
  source: [],
});
test("current editor position projects foot independently from depth ordering", async () => {
  const preview = await EditorActorPreview.bind(source, native, 4, [binding]);
  const snapshot = preview.snapshot(17, frames()),
    actor = snapshot.actors[0]!;
  assert.deepEqual(actor.maskQuery.mapPosition, [100.75, 200]);
  assert.deepEqual(actor.maskQuery.screenOrigin, [90, 169]);
  assert.equal(actor.displayOrder, 220);
  assert.equal(actor.sprite.direction, 7);
  assert.equal(snapshot.tick, 17);
  assert.equal(snapshot.epoch, 4);
  const moved = frames();
  moved.editable[0]!.sprite.position[2] = 221 / Math.sin(angle);
  assert.equal(preview.snapshot(17, moved).actors[0]!.displayOrder, 221);
  assert.equal(preview.snapshot(17, frames()).actors[0]!.displayOrder, 220);
  preview.dispose();
});
test("missing or ambiguous resources and invented identity aliases cannot survive retirement", async () => {
  const preview = await EditorActorPreview.bind(source, native, 5, [binding]);
  assert.throws(() => preview.snapshot(0, { editable: [], source: [] }), /missing or ambiguous/);
  const duplicate = frames();
  duplicate.editable.push(duplicate.editable[0]!);
  assert.throws(() => preview.snapshot(0, duplicate), /missing or ambiguous/);
  assert.throws(() => preview.setMaskMembership([]), /membership/);
  await assert.rejects(
    () =>
      EditorActorPreview.bind(source, native, 5, [
        { ...binding, preview: { kind: "editable", editorId: "arbitrary-mesh-0" } },
      ]),
    /verified import binding/,
  );
  preview.dispose();
  assert.throws(() => preview.snapshot(0, frames()), /disposed/);
});
