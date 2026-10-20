import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import type { StateDeliveryContract } from "../../shared/src/state-delivery.ts";
import type { StateRuntimeBinding } from "../../shared/src/state-runtime-adapter.ts";
import type { StateClockBatch } from "../../shared/src/state-event-clocks.ts";
import { RecordedStateCaptureSource, type CaptureFile } from "./state-runtime-capture-source.ts";
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
function bundle() {
  const f = fixture(),
    p = f.capture(),
    q = f.capture(true);
  const encode = (v: unknown) => new TextEncoder().encode(JSON.stringify(v));
  const hash = (v: Uint8Array) => createHash("sha256").update(v).digest("hex");
  const { engine_sha256: _e, assets_sha256: _a, ...identity } = p.manifest;
  const limits = ["Synthetic codec fixture only; no live trigger or binary equivalence evidence."];
  const receipt = {
    ...identity,
    scope: "bound-state-metadata",
    bindings: f.bindings,
    limitations: limits,
    binary_mission_sha256: "d".repeat(64),
    binary_level_sha256: "e".repeat(64),
  };
  const receiptBytes = encode(receipt);
  const manifest = {
    ...identity,
    version: 1,
    kind: "recorded-state-observations",
    bindings: f.bindings,
    authority: {
      scope: receipt.scope,
      receipt: { file: "qualification.json", sha256: hash(receiptBytes) },
      limitations: limits,
      binary_mission_sha256: receipt.binary_mission_sha256,
      binary_level_sha256: receipt.binary_level_sha256,
    },
    assets: { file: "assets.json", sha256: hash(p.a) },
    snapshots: [
      { file: "first.json", sha256: hash(p.e) },
      { file: "last.json", sha256: hash(q.e) },
    ],
  };
  const content = new Map([
    ["runtime-capture.json", encode(manifest)],
    ["qualification.json", receiptBytes],
    ["assets.json", p.a],
    ["first.json", p.e],
    ["last.json", q.e],
  ]);
  const files = (): CaptureFile[] =>
    [...content].map(([name, b]) => ({ name, arrayBuffer: async () => b.slice().buffer }));
  return { ...f, manifest, receipt, content, files, encode };
}
test("recorded source delivers only explicit pinned snapshots through existing adapter", async () => {
  const f = bundle(),
    samples: StateClockBatch[] = [],
    s = new RecordedStateCaptureSource(f.contract, (b) => samples.push(b));
  await s.load(f.files());
  assert.equal(samples.length, 0);
  assert.equal(s.remaining, 2);
  await s.next();
  await s.next();
  assert.deepEqual(
    samples.map((b) => b.simulation_tick),
    [17, 20],
  );
  assert.deepEqual(
    samples.map((b) => b.samples[0].tick),
    [5, 0],
  );
  assert.equal(s.remaining, 0);
  await assert.rejects(s.next(), /No recorded/);
});
test("capture rejects mismatched identity and tampered qualification before publishing", async () => {
  for (const mutation of ["identity", "receipt", "binding", "missing"]) {
    const f = bundle(),
      samples: StateClockBatch[] = [],
      s = new RecordedStateCaptureSource(f.contract, (b) => samples.push(b));
    if (mutation === "identity") f.manifest.mission = "Other";
    if (mutation === "receipt")
      f.content.set("qualification.json", f.encode({ ...f.receipt, scope: "full-parity" }));
    if (mutation === "binding") f.manifest.bindings[0]!.runtime_index = 11;
    if (mutation === "missing") f.content.delete("first.json");
    f.content.set("runtime-capture.json", f.encode(f.manifest));
    await assert.rejects(s.load(f.files()));
    assert.equal(samples.length, 0);
    await assert.rejects(s.next(), /reload/);
  }
});
test("retired source cannot deliver a pending file read into the replacement viewport", async () => {
  const f = bundle(),
    samples: StateClockBatch[] = [],
    s = new RecordedStateCaptureSource(f.contract, (b) => samples.push(b));
  let release!: () => void;
  const wait = new Promise<void>((resolve) => {
    release = resolve;
  });
  const files = f.files().map((file) =>
    file.name === "first.json"
      ? {
          name: file.name,
          arrayBuffer: async () => {
            await wait;
            return file.arrayBuffer();
          },
        }
      : file,
  );
  await s.load(files);
  const pending = s.next();
  await assert.rejects(s.next(), /in progress/);
  s.invalidate();
  release();
  await assert.rejects(pending, /reload/);
  assert.equal(samples.length, 0);
});
test("retirement during manifest load prevents initialization", async () => {
  const f = bundle(),
    s = new RecordedStateCaptureSource(f.contract, () => assert.fail("retired delivery"));
  let release!: () => void;
  const wait = new Promise<void>((resolve) => {
    release = resolve;
  });
  const files = f.files().map((file) =>
    file.name === "runtime-capture.json"
      ? {
          name: file.name,
          arrayBuffer: async () => {
            await wait;
            return file.arrayBuffer();
          },
        }
      : file,
  );
  const pending = s.load(files);
  s.invalidate();
  release();
  await assert.rejects(pending, /reload/);
});
test("sink failure permanently invalidates recorded source", async () => {
  const f = bundle(),
    s = new RecordedStateCaptureSource(f.contract, () => {
      throw Error("viewport unavailable");
    });
  await s.load(f.files());
  await assert.rejects(s.next(), /viewport unavailable/);
  await assert.rejects(s.next(), /reload/);
});
test("snapshot corruption never reaches sink", async () => {
  const f = bundle(),
    s = new RecordedStateCaptureSource(f.contract, () => assert.fail("corrupt delivery"));
  f.content.set("first.json", f.encode({ control: { frame_counter: 999 } }));
  await s.load(f.files());
  await assert.rejects(s.next(), /hash/);
  await assert.rejects(s.next(), /reload/);
});
