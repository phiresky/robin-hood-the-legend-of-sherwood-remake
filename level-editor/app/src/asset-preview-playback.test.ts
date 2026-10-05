import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { captureStateAppearance } from "./state-appearance-player.ts";
import { AssetPreviewPlayback, AssetPreviewPlaybackClock } from "./asset-preview-playback.ts";

function playback() {
  const asset = new THREE.Group();
  const node = new THREE.Object3D();
  node.name = "moving";
  asset.add(node);
  const clips = ["first", "second"].map(
    (name) =>
      new THREE.AnimationClip(name, -1, [
        new THREE.VectorKeyframeTrack(
          "moving.position",
          [0, 2 / 25, 5 / 25],
          [0, 0, 0, 2, 0, 0, 5, 0, 0],
          THREE.InterpolateDiscrete,
        ),
      ]),
  );
  return new AssetPreviewPlayback(captureStateAppearance(asset, clips));
}

test("preview defaults paused, clamps, restarts and seeks independently of mission timing", () => {
  const p = playback();
  assert.equal(p.playing, false);
  assert.equal(p.tick, 0);
  assert.equal(p.lastTick, 5);
  p.play();
  p.advance(1);
  assert.equal(p.tick, 5);
  assert.equal(p.playing, false);
  p.play();
  assert.equal(p.tick, 0);
  p.seek(2);
  assert.equal(p.playing, false);
  assert.equal(p.player.content.children[0]!.position.x, 2);
  p.setLoop(true);
  p.play();
  p.advance(4 / 25);
  assert.equal(p.tick, 0);
  p.select("second");
  assert.equal(p.playing, false);
  assert.equal(p.tick, 0);
  assert.throws(() => p.select("missing"), /Unknown/);
  p.dispose();
});

test("one clock advances independent visible cards and cancels after release or disposal", () => {
  let next = 0;
  const callbacks = new Map<number, (time: number) => void>();
  const clock = new AssetPreviewPlaybackClock(
    (cb) => {
      callbacks.set(++next, cb);
      return next;
    },
    (id) => {
      callbacks.delete(id);
    },
  );
  const frame = (time: number) => {
    assert.equal(callbacks.size, 1);
    const [id, cb] = callbacks.entries().next().value!;
    callbacks.delete(id);
    cb(time);
  };
  const a = playback(),
    b = playback();
  let changes = 0;
  const releaseA = clock.register(a, () => changes++);
  const releaseB = clock.register(b, () => changes++);
  a.play();
  b.play();
  clock.request();
  clock.request();
  frame(1000);
  frame(1080);
  assert.equal(a.tick, 2);
  assert.equal(b.tick, 2);
  assert.equal(changes, 2);
  releaseA();
  a.dispose();
  frame(1120);
  assert.equal(b.tick, 3);
  releaseB();
  assert.equal(callbacks.size, 0);
  b.dispose();
  const c = playback();
  clock.register(c, () => {});
  c.play();
  clock.request();
  clock.dispose();
  assert.equal(callbacks.size, 0);
  assert.throws(() => clock.request(), /disposed/);
  c.dispose();
});
