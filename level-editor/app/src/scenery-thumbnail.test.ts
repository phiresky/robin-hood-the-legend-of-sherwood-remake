import test from "node:test";
import assert from "node:assert/strict";
import { loadSceneryThumbnail } from "./scenery-thumbnail.ts";
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
