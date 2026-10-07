import test from "node:test";
import assert from "node:assert/strict";
import { SourceAnimationClocks, type SourceClockIdentity } from "./native-animation-clocks.ts";
import { nativePresentationFrame } from "./native-state-presentation.ts";

const context = { levelSha256: "a".repeat(64), missionSha256: "b".repeat(64) };
const ambient: SourceClockIdentity = {
  kind: "map-animation",
  index: 7,
  sourceSha256: "c".repeat(64),
};
const sign: SourceClockIdentity = {
  kind: "mission-target",
  index: 8,
  sourceSha256: "d".repeat(64),
};
const row = {
  loop: true,
  frames: [0, 1, 3].map((delay, index) => ({
    delay,
    path: `frame-${index}.png`,
    sha256: "e".repeat(64),
    width: 1,
    height: 1,
    offset: [0, 0] as [number, number],
  })),
};
function setup() {
  const clocks = new SourceAnimationClocks();
  clocks.reset(context);
  const a = clocks.define(ambient),
    s = clocks.define(sign);
  clocks.setState(a, { playing: true });
  clocks.setState(s, { playing: true });
  return { clocks, a, s };
}

test("one25Hz boundary retains fractional remainder across varied render rates", () => {
  const { clocks, a } = setup();
  assert.equal(clocks.advance(0, 0.01), 0);
  assert.equal(clocks.advance(1, 0.01), 0);
  assert.equal(clocks.advance(2, 0.019), 0);
  assert.equal(clocks.advance(3, 0.001), 1);
  assert.equal(clocks.read(a).tick, 1);
  assert.ok(clocks.timing().remainderTicks < 1e-9);
  for (let i = 4; i < 64; i++) clocks.advance(i, 1 / 60);
  assert.equal(clocks.read(a).tick, 26);
  assert.ok(clocks.timing().remainderTicks < 1e-8);
});
test("repeated render consumers cannot double advance a simulation frame", () => {
  const { clocks, a } = setup();
  clocks.advance(10, 0.04);
  assert.throws(() => clocks.advance(10, 0.04), /Duplicate/);
  assert.throws(() => clocks.advance(9, 0.04), /reversed/);
  assert.equal(clocks.read(a).tick, 1);
});
test("same source identity shares one cursor and definition never restarts it", () => {
  const { clocks, a } = setup();
  clocks.advance(0, 0.4);
  const late = clocks.define({ ...ambient });
  assert.equal(a, late);
  assert.equal(clocks.read(late).tick, 10);
  assert.throws(() => clocks.define({ ...ambient, sourceSha256: "f".repeat(64) }), /changed/);
});
test("seeking or pausing a sign does not change ambient cursor or fraction", () => {
  const { clocks, a, s } = setup();
  clocks.advance(0, 0.03);
  clocks.seek(s, 50);
  clocks.setState(s, { playing: false });
  clocks.advance(1, 0.01);
  assert.equal(clocks.read(a).tick, 1);
  assert.equal(clocks.read(s).tick, 50);
  clocks.setState(s, { playing: true });
  clocks.advance(2, 0.04);
  assert.equal(clocks.read(a).tick, 2);
  assert.equal(clocks.read(s).tick, 51);
});
test("inactive source pauses its own cursor while global tick boundaries continue", () => {
  const { clocks, a, s } = setup();
  clocks.setState(a, { active: false });
  clocks.advance(0, 0.03);
  clocks.setState(a, { active: true });
  clocks.advance(1, 0.01);
  assert.equal(clocks.read(a).tick, 1);
  assert.equal(clocks.read(s).tick, 1);
});
test("source frame selection uses delay+1 and is identical across representations", () => {
  const { clocks, a } = setup();
  const scenery = clocks.define(ambient),
    native = clocks.define(ambient);
  const expected = [0, 1, 1, 2, 2, 2, 2, 0];
  for (let i = 0; i < expected.length; i++) {
    if (i) clocks.advance(i, 0.04);
    assert.equal(nativePresentationFrame(row, clocks.read(a).tick), expected[i]);
    assert.equal(
      nativePresentationFrame(row, clocks.read(scenery).tick),
      nativePresentationFrame(row, clocks.read(native).tick),
    );
  }
});
test("late async asset load samples current phase instead of restarting at load completion", async () => {
  const { clocks, a } = setup();
  let resolve!: () => void;
  const gate = new Promise<void>((r) => {
    resolve = r;
  });
  const loaded = (async () => {
    await gate;
    assert.equal(clocks.isCurrent(a), true);
    return clocks.read(clocks.define(ambient)).tick;
  })();
  clocks.advance(0, 0.56);
  resolve();
  assert.equal(await loaded, 14);
  assert.equal(clocks.read(a).tick, 14);
});
test("mission reset retires old asynchronous handles even for identical source bytes", async () => {
  const { clocks, a } = setup();
  let resolve!: () => void;
  const gate = new Promise<void>((r) => {
    resolve = r;
  });
  const loaded = (async () => {
    await gate;
    return clocks.isCurrent(a) ? clocks.read(a).tick : null;
  })();
  clocks.reset(context);
  const replacement = clocks.define(ambient);
  resolve();
  assert.equal(await loaded, null);
  assert.notEqual(replacement.epoch, a.epoch);
  assert.equal(clocks.read(replacement).tick, 0);
  assert.throws(() => clocks.seek(a, 3), /Retired/);
});
test("release then redefine invalidates same-epoch resource callbacks", () => {
  const { clocks, a } = setup();
  clocks.release(a);
  const replacement = clocks.define(ambient);
  assert.equal(replacement.epoch, a.epoch);
  assert.notEqual(replacement.generation, a.generation);
  assert.equal(clocks.isCurrent(a), false);
  assert.equal(clocks.isCurrent(replacement), true);
});
test("invalid input and overflow leave all cursors and sequence unchanged", () => {
  const { clocks, a, s } = setup();
  clocks.seek(s, Number.MAX_SAFE_INTEGER);
  assert.throws(() => clocks.advance(0, 0.04), /integer/);
  assert.equal(clocks.read(a).tick, 0);
  assert.equal(clocks.timing().sequence, -1);
  assert.throws(() => clocks.advance(0, NaN), /elapsed/);
  assert.throws(() => clocks.advance(0, -1), /elapsed/);
  clocks.setState(s, { playing: false });
  assert.equal(clocks.advance(0, 0.04), 1);
});
test("snapshots are immutable and forged/cross-store handles cannot mutate clocks", () => {
  const { clocks, a } = setup(),
    other = setup();
  const snapshot = clocks.read(a);
  assert.equal(Object.isFrozen(snapshot), true);
  assert.throws(() => clocks.read({ ...a }), /foreign/);
  assert.throws(() => other.clocks.read(a), /foreign/);
  clocks.dispose();
  clocks.dispose();
  assert.equal(clocks.isCurrent(a), false);
  assert.throws(() => clocks.advance(0, 0.04), /disposed/);
});
