import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { SourceContractClockBinding } from "./source-contract-clock-binding.ts";
import { NativeStatePresentation } from "./native-state-presentation.ts";
import { PatchDisplay } from "./patch-display.ts";
import {
  validateSourceCoverVisibility,
  verifySourceCoverPlacements,
} from "../../shared/src/source-cover-visibility.ts";
import { missionStateDataHash } from "./mission-state-layer.ts";
import { sourceCoverFixture as fixture } from "./source-cover-visibility.fixture.ts";
test("placed roof appearance binding controls effect and overlay without resetting source time", async () => {
  const { c, source, cover, document, reader } = await fixture();
  verifySourceCoverPlacements([cover], document);
  validateSourceCoverVisibility([cover]);
  const clocks = new SourceContractClockBinding();
  await clocks.set(c.native, c.physical, source, [cover]);
  const samples: any[] = [];
  clocks.attachPhysical(
    cover.physical_target_id,
    {
      internallyPlaying: () => false,
      sampleExternalTick: (tick, visible) => samples.push({ tick, visible }),
    },
    clocks.physicalToken(cover.physical_target_id),
  );
  const art = new NativeStatePresentation();
  await art.set(c.native, source, reader);
  const display = new PatchDisplay(),
    roof = new THREE.Object3D();
  roof.userData.reveal_hide_when_applied = ["patch-002"];
  const root = new THREE.Group();
  root.add(roof);
  const sync = () => {
    display.apply(root);
    clocks.setCoverRevealed("patch-002", display.isRevealed("patch-002"));
    art.sampleExternalClocks(clocks.snapshot(), "animation-000");
  };
  clocks.setPhysicalPlaying(cover.physical_target_id, true);
  clocks.advance(1, 0.24);
  sync();
  assert.equal(roof.visible, true);
  assert.deepEqual(samples.at(-1), { tick: 6, visible: false });
  assert.equal(art.loopDrawSnapshot().draws.length, 0);
  const hidden = art.pixels().data.slice();
  display.set("patch-002", true);
  sync();
  assert.equal(roof.visible, false);
  assert.deepEqual(samples.at(-1), { tick: 6, visible: true });
  assert.equal(art.loopDrawSnapshot().draws.length, 1);
  assert.notDeepEqual(art.pixels().data, hidden);
  clocks.advance(2, 0.12);
  display.set("patch-002", false);
  sync();
  assert.equal(roof.visible, true);
  assert.deepEqual(samples.at(-1), { tick: 9, visible: false });
  assert.equal(clocks.snapshot().find((r) => r.id === "animation-000")!.active, true);
  assert.equal(clocks.snapshot().find((r) => r.id === "animation-000")!.playing, true);
  assert.equal(art.loopDrawSnapshot().draws.length, 0);
  display.set("patch-001", true);
  sync();
  assert.equal(samples.at(-1).visible, false);
  clocks.advance(3, 0.12);
  display.set("patch-002", true);
  sync();
  assert.deepEqual(samples.at(-1), { tick: 12, visible: true });
  const token = clocks.physicalToken(cover.physical_target_id);
  clocks.clear();
  assert.equal(clocks.resourceIsCurrent(token), false);
  assert.throws(() => clocks.setCoverRevealed("patch-002", true), /retired/);
  assert.equal(clocks.coverPatchIds().length, 0);
  art.clear();
  clocks.dispose();
  art.dispose();
  assert.throws(() => clocks.setCoverRevealed("patch-002", true), /retired/);
});
test("source, appearance placement and lifetime reject mismatch rather than showing through a roof", async () => {
  const { c, source, cover, document, reader } = await fixture();
  const clocks = new SourceContractClockBinding();
  const wrong = structuredClone(cover);
  wrong.source.sha256 = "0".repeat(64);
  await assert.rejects(clocks.set(c.native, c.physical, source, [wrong]), /patch or loop/);
  assert.equal(clocks.ready, false);
  const absent = structuredClone(cover);
  absent.source.index = 999;
  absent.placement.patch_id = "patch-999";
  await assert.rejects(clocks.set(c.native, c.physical, source, [absent]), /patch or loop/);
  const remapped = structuredClone(document);
  remapped.groups[0].patches[cover.placement.asset_id]["appearance-2"] = "patch-001";
  assert.throws(() => verifySourceCoverPlacements([cover], remapped), /displayed roof/);
  const replaced = structuredClone(document);
  replaced.assetSources[0].model_sha256 = "0".repeat(64);
  assert.throws(() => verifySourceCoverPlacements([cover], replaced), /displayed roof/);
  await clocks.set(c.native, c.physical, source, [cover]);
  clocks.setCoverRevealed("patch-002", true);
  await clocks.set(c.native, c.physical, source, [cover]);
  assert.equal(clocks.snapshot().find((r) => r.id === "animation-000")!.visible, false);
  clocks.dispose();
});

test("four synthetic source cover backgrounds follow real appearance bits, reset without resetting loop", async () => {
  const { source, document, c, reader } = await fixture();
  const { decodeNativeResource, compositeNativePixels } =
    await import("./native-state-presentation.ts");
  const clocks = new SourceContractClockBinding(),
    art = new NativeStatePresentation(),
    display = new PatchDisplay();
  await clocks.set(c.native, c.physical, source, c.cover_visibility);
  await art.set(c.native, source, reader);
  verifySourceCoverPlacements(art.coverArtworkBindings(), document);
  clocks.setPhysicalPlaying("hall000-flames-reflections", true);
  const backgrounds = [];
  for (const [a, b] of [
    [false, false],
    [true, false],
    [false, true],
    [true, true],
    [false, false],
  ]) {
    clocks.advance(backgrounds.length + 1, 0.12);
    display.set("patch-001", a!);
    display.set("patch-002", b!);
    clocks.setCoverRevealed("patch-002", display.isRevealed("patch-002"));
    for (const binding of art.coverArtworkBindings())
      art.setCoverArtworkState(
        binding.placement.patch_id,
        display.isRevealed(binding.placement.patch_id),
      );
    art.sampleExternalClocks(clocks.snapshot(), "animation-000");
    const snapshot = art.loopDrawSnapshot(),
      variant = c.native.cover_artwork.variants.find(
        (v: any) => v.applied.includes("patch-001") === a && v.applied.includes("patch-002") === b,
      );
    const expected = await decodeNativeResource(variant.image, reader);
    assert.deepEqual(snapshot.background, expected);
    backgrounds.push(expected.data.slice());
    for (const draw of snapshot.draws)
      compositeNativePixels(expected, draw.pixels, draw.x, draw.y, draw.shadow);
    assert.deepEqual(art.pixels(), expected);
    assert.equal(snapshot.draws.length, b ? 1 : 0);
    assert.equal(art.tick, backgrounds.length * 3);
    assert.equal(art.isPlaying, true);
  }
  assert.deepEqual(backgrounds[0], backgrounds[4]);
  assert.notDeepEqual(backgrounds[0], backgrounds[1]);
  assert.notDeepEqual(backgrounds[0], backgrounds[2]);
  const bad = structuredClone(c.native);
  bad.cover_artwork.bindings[0].source.sha256 = "0".repeat(64);
  await assert.rejects(art.set(bad, source, reader), /cover artwork source changed/);
  assert.equal(art.ready, false);
  clocks.dispose();
  art.dispose();
  assert.throws(() => art.setCoverArtworkState("patch-002", true), /Unknown/);
});
