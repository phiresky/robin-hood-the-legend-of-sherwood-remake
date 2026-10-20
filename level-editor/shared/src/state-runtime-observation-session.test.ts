import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { RuntimeObservationSession } from "./state-runtime-observation-session.ts";
import type { StateDeliveryContract } from "./state-delivery.ts";
import type { StateRuntimeBinding } from "./state-runtime-adapter.ts";
import type { StateClockBatch } from "./state-event-clocks.ts";
function fixture() {
  const source = { kind: "mission-patch" as const, index: 0, sha256: "c".repeat(64) };
  const native = {
    mission: "Synthetic",
    mission_data_sha256: "a".repeat(64),
    level_data_sha256: "b".repeat(64),
    elements: [],
    patch_states: [
      {
        id: "p",
        source,
        definitive: true,
        initial: [{ delay: 1 }],
        transition: [{ delay: 1 }, { delay: 1 }, { delay: 1 }],
        final: [],
      },
    ],
    background_states: [],
  };
  const contract = {
    native,
    families: [
      {
        id: "f",
        element_ids: [],
        patch_ids: ["p"],
        background_ids: [],
        body_terminal_tick: 5,
        physical: {},
      },
    ] as unknown as StateDeliveryContract["families"],
  } as unknown as StateDeliveryContract;
  const bindings: StateRuntimeBinding[] = [
    { kind: "patch", id: "p", source, runtime_index: 0, phase_rows: { initial: 0, forward: 1 } },
  ];
  function capture(applied = false) {
    const encode = (v: unknown) => new TextEncoder().encode(JSON.stringify(v));
    const e = encode({
      control: { frame_counter: applied ? 20 : 17 },
      world: {
        entities: [
          {
            Fx: {
              element: {
                active: !applied,
                sprite: {
                  current_row: 1,
                  current_frame: 2,
                  frame_count: 1,
                  last_action: "PatchTransition",
                },
              },
            },
          },
        ],
      },
      script_domains: { interactables: { patches: [{ applied, in_transition: !applied }] } },
    });
    const a = encode({ assets: { patch_entity_handles: [0x10000000] } });
    return {
      e,
      a,
      manifest: {
        mission: native.mission,
        mission_data_sha256: native.mission_data_sha256,
        level_data_sha256: native.level_data_sha256,
        engine_sha256: createHash("sha256").update(e).digest("hex"),
        assets_sha256: createHash("sha256").update(a).digest("hex"),
      },
    };
  }
  return { contract, bindings, capture };
}
test("synthetic session delegates observed clocks and empty final phase without inventing final row", async () => {
  const f = fixture(),
    batches: StateClockBatch[] = [],
    s = new RuntimeObservationSession(f.contract, f.bindings, (b) => batches.push(b));
  for (const applied of [false, true]) {
    const p = f.capture(applied);
    await s.deliver(p.e, p.a, p.manifest);
  }
  assert.deepEqual(
    batches.map((b) => b.samples[0].tick),
    [5, 0],
  );
  assert.deepEqual(
    batches.map((b) => b.sequence),
    [0, 1],
  );
});
test("sink failure invalidates rather than advancing a divergent subsequent delivery", async () => {
  const f = fixture(),
    s = new RuntimeObservationSession(f.contract, f.bindings, () => {
      throw new Error("sink unavailable");
    });
  const p = f.capture();
  await assert.rejects(s.deliver(p.e, p.a, p.manifest), /sink unavailable/);
  await assert.rejects(s.deliver(p.e, p.a, p.manifest), /reload/);
});
test("invalidate during hash and concurrent input cannot publish obsolete captures", async () => {
  const f = fixture(),
    received: StateClockBatch[] = [],
    s = new RuntimeObservationSession(f.contract, f.bindings, (b) => received.push(b));
  const p = f.capture(),
    pending = s.deliver(p.e, p.a, p.manifest);
  await assert.rejects(s.deliver(p.e, p.a, p.manifest), /in progress/);
  s.invalidate();
  await assert.rejects(pending, /reload/);
  assert.equal(received.length, 0);
});
