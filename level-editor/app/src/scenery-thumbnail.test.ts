import test from "node:test";
import assert from "node:assert/strict";
import { loadSceneryThumbnail, loadSceneryBank } from "./scenery-thumbnail.ts";
import { sceneryThumbnailFixture } from "../tests/scenery-thumbnail-fixture.ts";

test("standalone thumbnails use verified profile frames and preserve pixel format", async () => {
  for (const legacy of [true, false]) {
    const { descriptor, files, read } = await sceneryThumbnailFixture(legacy);
    const result = await loadSceneryThumbnail(descriptor, read);
    assert.equal(result!.legacy, legacy);
    assert.deepEqual(result!.png, files.get("effects/fire.rhs.d/idle/0.png"));
  }
});

test("ordinary models and unpinned shared effects keep their existing previews", async () => {
  const { descriptor } = await sceneryThumbnailFixture();
  const read = async () => {
    throw new Error("Unexpected resource read");
  };
  delete descriptor.parts[0]!.gameplay_only;
  assert.equal(await loadSceneryThumbnail(descriptor, read), null);
  descriptor.parts[0]!.gameplay_only = true;
  delete descriptor.gameplay!.animations![0]!.resourceDirectory;
  assert.equal(await loadSceneryThumbnail(descriptor, read), null);
});

test("changed bytes or mismatched centers cannot produce a misleading thumbnail", async () => {
  const { descriptor, files, read } = await sceneryThumbnailFixture();
  descriptor.gameplay!.animations![0]!.center[0] = 999;
  await assert.rejects(loadSceneryThumbnail(descriptor, read), /changed sprite center/);
  descriptor.gameplay!.animations![0]!.center[0] = 1;
  files.set("effects/fire.rhs.d/idle/0.png", new Uint8Array());
  await assert.rejects(loadSceneryThumbnail(descriptor, read), /resource changed/);
});

test("preview starts with the same action and direction row as the native loader", async () => {
  const { descriptor, files, read } = await sceneryThumbnailFixture(true, true);
  const path = "effects/fire.rhs.d/manifest.json";
  const manifest = JSON.parse(new TextDecoder().decode(files.get(path)));
  const profile = manifest.profiles[0];
  const row = profile.rows[0];
  profile.rows = [
    { ...row, direction: 1, frames: [row.frames[0]] },
    { ...row, action_id: 3, direction: 0, frames: [row.frames[0]] },
    { ...row, direction: 0, frames: [row.frames[1]] },
  ];
  const bytes = new TextEncoder().encode(JSON.stringify(manifest));
  files.set(path, bytes);
  descriptor.resources!.find((pin) => pin.path === path)!.sha256 = [
    ...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
  ]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  const bank = await loadSceneryBank(descriptor, descriptor.gameplay!.animations![0]!, read);
  assert.deepEqual(
    bank.profile.rows.map((row) => [row.action, row.direction]),
    [
      [0, 0],
      [0, 1],
      [3, 0],
    ],
  );
  assert.deepEqual(
    (await loadSceneryThumbnail(descriptor, read))!.png,
    files.get("effects/fire.rhs.d/idle/1.png"),
  );
});
