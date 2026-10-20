import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { decodeRuntimeCapture, runtimeActorSlot } from "./state-runtime-observation.ts";
import type { StateRuntimeBinding } from "./state-runtime-adapter.ts";
const source = {
  mission: "Fixture",
  mission_data_sha256: "a".repeat(64),
  level_data_sha256: "b".repeat(64),
};
const binding: StateRuntimeBinding = {
  kind: "patch",
  id: "p",
  source: { kind: "mission-patch", index: 0, sha256: "c".repeat(64) },
  runtime_index: 0,
  phase_rows: { initial: 0, forward: 1 },
};
function packet(engine: unknown, assets: unknown) {
  const encode = (v: unknown) => new TextEncoder().encode(JSON.stringify(v));
  const e = encode(engine),
    a = encode(assets);
  return {
    e,
    a,
    manifest: {
      ...source,
      engine_sha256: createHash("sha256").update(e).digest("hex"),
      assets_sha256: createHash("sha256").update(a).digest("hex"),
    },
  };
}
function synthetic() {
  return {
    engine: {
      control: { frame_counter: 17 },
      world: {
        entities: [
          null,
          {
            Fx: {
              element: {
                active: true,
                sprite: {
                  current_row: 1,
                  current_frame: 0,
                  frame_count: 65535,
                  last_action: "PatchTransition",
                },
              },
            },
          },
        ],
      },
      script_domains: { interactables: { patches: [{ applied: false, in_transition: true }] } },
    },
    assets: { assets: { patch_entity_handles: [0x10000001] } },
  };
}
test("typed actor decoding rejects other domains and raw indices", () => {
  assert.equal(runtimeActorSlot(0x10000000), 0);
  assert.equal(runtimeActorSlot(0x1fffffff), 0xfffffff);
  for (const value of [0, 1, 0x30000001, -1, 1.5, null])
    assert.throws(() => runtimeActorSlot(value));
});
test("synthetic structural probe preserves sparse FX and reset sentinel (not live fixture proof)", async () => {
  const f = synthetic(),
    p = packet(f.engine, f.assets);
  const observed = await decodeRuntimeCapture(p.e, p.a, p.manifest, source, [binding]);
  assert.equal(observed.engine_frame, 17);
  assert.equal(observed.patches[0].sprite?.count, 65535);
  assert.equal(observed.patches[0].sprite?.row, 1);
});
test("source and payload guards reject before decoding", async () => {
  const f = synthetic(),
    p = packet(f.engine, f.assets);
  await assert.rejects(
    decodeRuntimeCapture(p.e, p.a, { ...p.manifest, mission: "Other" }, source, [binding]),
    /identity/,
  );
  const changed = new Uint8Array(p.e);
  changed[0] ^= 1;
  await assert.rejects(decodeRuntimeCapture(changed, p.a, p.manifest, source, [binding]), /hash/);
});
test("missing FX, wrong variant, invalid counter and old flat shape cannot become empty observations", async () => {
  const cases = [
    (f: ReturnType<typeof synthetic>) => {
      f.assets.assets.patch_entity_handles[0] = 0x30000001;
    },
    (f: ReturnType<typeof synthetic>) => {
      f.assets.assets.patch_entity_handles[0] = 0x10000000;
    },
    (f: ReturnType<typeof synthetic>) => {
      f.engine.world.entities[1] = { Target: {} } as never;
    },
    (f: ReturnType<typeof synthetic>) => {
      f.engine.world.entities[1]!.Fx.element.sprite.frame_count = -1;
    },
    (f: ReturnType<typeof synthetic>) => {
      delete (f.engine as Partial<typeof f.engine>).control;
    },
    (f: ReturnType<typeof synthetic>) => {
      f.assets.assets.patch_entity_handles = [];
    },
  ];
  for (const mutate of cases) {
    const f = synthetic();
    mutate(f);
    const p = packet(f.engine, f.assets);
    await assert.rejects(decodeRuntimeCapture(p.e, p.a, p.manifest, source, [binding]));
  }
});
test("duplicate binding is rejected but background and patch may observe one native FX", async () => {
  const f = synthetic(),
    p = packet(f.engine, f.assets);
  await assert.rejects(
    decodeRuntimeCapture(p.e, p.a, p.manifest, source, [binding, binding]),
    /Duplicate/,
  );
  const result = await decodeRuntimeCapture(p.e, p.a, p.manifest, source, [
    binding,
    { ...binding, kind: "background", id: "ground" },
  ]);
  assert.equal(result.patches.length, 1);
});
