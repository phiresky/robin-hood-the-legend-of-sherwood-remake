import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import type { StateDeliveryContract } from "../../shared/src/state-delivery.ts";
import type { StateRuntimeBinding } from "../../shared/src/state-runtime-adapter.ts";
import type { StateClockBatch } from "../../shared/src/state-event-clocks.ts";
import { LiveStateCaptureSource } from "./state-runtime-live-source.ts";
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
      control: { frame_counter: applied ? 18 : 17 },
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
function setup() {
  const f = fixture(),
    hash = (v: Uint8Array) => createHash("sha256").update(v).digest("hex"),
    encode = (v: unknown) => new TextEncoder().encode(JSON.stringify(v));
  const identity = {
    mission: f.contract.native.mission,
    mission_data_sha256: f.contract.native.mission_data_sha256,
    level_data_sha256: f.contract.native.level_data_sha256,
  };
  const receipt = {
      ...identity,
      scope: "bound-state-metadata",
      bindings: f.bindings,
      limitations: ["Synthetic test only"],
      binary_mission_sha256: "d".repeat(64),
      binary_level_sha256: "e".repeat(64),
    },
    receiptBytes = encode(receipt);
  const m = {
    ...identity,
    version: 1,
    kind: "live-state-observations",
    session_id: "f".repeat(32),
    first_engine_frame: 17,
    bindings: f.bindings,
    assets: { file: "assets.json", sha256: hash(f.capture().a) },
    authority: {
      scope: receipt.scope,
      limitations: receipt.limitations,
      binary_mission_sha256: receipt.binary_mission_sha256,
      binary_level_sha256: receipt.binary_level_sha256,
      receipt: { file: "qualification.json", sha256: hash(receiptBytes) },
    },
  };
  const packet = (applied: boolean, sequence: number) => {
    const c = f.capture(applied);
    return {
      session_id: m.session_id,
      sequence,
      engine_frame: applied ? 18 : 17,
      engine_text: new TextDecoder().decode(c.e),
      engine_sha256: hash(c.e),
    };
  };
  const routes: Record<string, unknown> = {
    session: m,
    assets: JSON.parse(new TextDecoder().decode(f.capture().a)),
    qualification: receipt,
    "packet/0": packet(false, 0),
    "packet/1": packet(true, 1),
    "packet/2": { session_id: m.session_id, sequence: 2, closed: true },
  };
  const batches: StateClockBatch[] = [],
    source = new LiveStateCaptureSource(
      f.contract,
      (b) => batches.push(b),
      async (p) => new Response(JSON.stringify(routes[p])),
    );
  return { f, m, routes, batches, source };
}
test("live source delivers ordered producer frames then closes without inventing a clock", async () => {
  const t = setup();
  await t.source.connect();
  await t.source.next();
  await t.source.next();
  await t.source.next();
  assert.deepEqual(
    t.batches.map((b) => b.simulation_tick),
    [17, 18],
  );
  assert.equal(t.source.closed, true);
});
for (const [name, change] of Object.entries({
  restart: (p: any) => (p.session_id = "0".repeat(32)),
  sequence: (p: any) => (p.sequence = 2),
  gap: (p: any) => (p.engine_frame = 19),
  payload: (p: any) => (p.engine_sha256 = "0".repeat(64)),
}))
  test("live source rejects " + name + " and retires", async () => {
    const t = setup();
    await t.source.connect();
    change(t.routes["packet/0"]);
    await assert.rejects(t.source.next());
    await assert.rejects(t.source.next(), /reload/);
    assert.equal(t.batches.length, 0);
  });
test("live source refuses wrong source identity", async () => {
  const t = setup();
  t.m.mission = "Other";
  await assert.rejects(t.source.connect(), /identity/);
  assert.equal(t.batches.length, 0);
});
test("retirement during fetch cannot publish into a replacement viewport", async () => {
  const t = setup();
  let release!: () => void,
    wait = new Promise<void>((r) => (release = r));
  let requested = false;
  const source = new LiveStateCaptureSource(
    t.f.contract,
    (b) => t.batches.push(b),
    async (p) => {
      if (p.startsWith("packet/")) {
        requested = true;
        await wait;
      }
      return new Response(JSON.stringify(t.routes[p]));
    },
  );
  await source.connect();
  const pending = source.next();
  while (!requested) await new Promise((r) => setTimeout(r, 0));
  source.invalidate();
  release();
  await assert.rejects(pending, /reload/);
  assert.equal(t.batches.length, 0);
});
test("failed live sink invalidates further delivery", async () => {
  const t = setup();
  const source = new LiveStateCaptureSource(
    t.f.contract,
    () => {
      throw Error("sink failed");
    },
    async (p) => new Response(JSON.stringify(t.routes[p])),
  );
  await source.connect();
  await assert.rejects(source.next(), /sink failed/);
  await assert.rejects(source.next(), /reload/);
});
