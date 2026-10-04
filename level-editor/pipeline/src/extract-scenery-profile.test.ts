import test from "node:test";
import assert from "node:assert/strict";
import sharp from "sharp";
import { extractSceneryProfile } from "./extract-scenery-profile.ts";

async function fixture() {
  const pixels = Buffer.from([0, 248, 0, 255, 255, 0, 0, 255, 1, 2, 3, 0, 0, 0, 255, 255]);
  const atlas = await sharp(pixels, { raw: { width: 4, height: 1, channels: 4 } })
    .webp({ lossless: true })
    .toBuffer();
  const frame = {
    file: "atlas.webp",
    rect: [0, 0, 2, 1],
    delay: 3,
    distance: 0,
    offset_x: 7,
    offset_y: -2,
    sound_id: 0,
  };
  const profile = {
    name: "Fire",
    width: 2,
    height: 1,
    center_x: 10,
    center_y: 20,
    rows: [
      {
        action_id: 0,
        action_done: 283,
        direction: 0,
        average_speed: 0,
        hotspot_x: 10,
        hotspot_y: 20,
        path: ".",
        frames: [frame, { ...frame, rect: [2, 0, 2, 1], delay: 5 }],
      },
    ],
  };
  const manifest = { atlas: "atlas.webp", profiles: [profile, { ...profile, name: "Unused" }] };
  return { manifest, profile, frame, read: async () => atlas, atlas };
}

test("atlas authoring retains frame pixels, timing, offsets, centers and color-key semantics", async () => {
  const { manifest, read, atlas } = await fixture();
  const result = await extractSceneryProfile(manifest, "Fire", read);
  const output = JSON.parse(new TextDecoder().decode(result.files["manifest.json"]));
  assert.equal(output.pixel_format, "legacy_color_keys");
  assert.equal(output.profiles.length, 1);
  assert.equal(result.profile.center_x, 10);
  assert.equal(result.profile.center_y, 20);
  const row = output.profiles[0].rows[0];
  assert.deepEqual(
    row.frames.map((f: { delay: number }) => f.delay),
    [3, 5],
  );
  assert.equal(row.frames[0].offset_x, 7);
  assert.equal(row.frames[0].offset_y, -2);
  for (let i = 0; i < 2; i++) {
    const actual = await sharp(result.files[`0-${i}.png`]).ensureAlpha().raw().toBuffer();
    const expected = await sharp(atlas)
      .extract({ left: i * 2, top: 0, width: 2, height: 1 })
      .ensureAlpha()
      .raw()
      .toBuffer();
    assert.deepEqual(actual, expected);
  }
  const rgba = await extractSceneryProfile({ ...manifest, pixel_format: "rgba" }, "Fire", read);
  assert.equal(
    JSON.parse(new TextDecoder().decode(rgba.files["manifest.json"])).pixel_format,
    "rgba",
  );
});

test("atlas authoring rejects ambiguous profiles, escaping paths, bad rectangles and invalid runtime metadata", async () => {
  const { manifest, profile, frame, read } = await fixture();
  await assert.rejects(extractSceneryProfile(manifest, "Missing", read), /Missing or duplicate/);
  await assert.rejects(
    extractSceneryProfile({ ...manifest, profiles: [profile, profile] }, "Fire", read),
    /duplicate/,
  );
  await assert.rejects(
    extractSceneryProfile({ ...manifest, atlas: "../atlas.webp" }, "Fire", read),
    /invalid sprite atlas/,
  );
  frame.rect = [3, 0, 2, 1];
  await assert.rejects(extractSceneryProfile(manifest, "Fire", read), /outside the image/);
  frame.rect = [0, 0, 2, 1];
  frame.delay = -1;
  await assert.rejects(
    extractSceneryProfile(manifest, "Fire", read),
    /invalid sprite frame metadata/,
  );
});

test("complete multi-profile PNG banks retain every frame byte and metadata", async () => {
  const { manifest, profile } = await fixture();
  const png = await sharp({ create: { width: 2, height: 1, channels: 4, background: "red" } })
    .png()
    .toBuffer();
  const frames = profile.rows[0]!.frames;
  frames[0]!.file = "0.png";
  frames[1]!.file = "1.png";
  const reads: string[] = [];
  const result = await extractSceneryProfile(
    { profiles: manifest.profiles },
    "Fire",
    async (name) => {
      reads.push(name);
      if (!name.startsWith("Fire/"))
        throw Object.assign(new Error("Missing frame"), { code: "ENOENT" });
      return png;
    },
  );
  assert.deepEqual(reads, ["0.png", "Fire/0.png", "1.png", "Fire/1.png"]);
  assert.deepEqual(result.files["0-0.png"], png);
  assert.deepEqual(result.files["0-1.png"], png);
  const output = JSON.parse(new TextDecoder().decode(result.files["manifest.json"]));
  assert.deepEqual(
    output.profiles[0].rows[0].frames.map((f: { delay: number }) => f.delay),
    [3, 5],
  );
  await assert.rejects(
    extractSceneryProfile({ profiles: [profile] }, "Fire", async () => new Uint8Array()),
    /invalid sprite PNG/,
  );
  const flat = await extractSceneryProfile(
    { profiles: manifest.profiles, pixel_format: "rgba" },
    "Fire",
    async (name) => {
      assert.ok(name === "0.png" || name === "1.png");
      return png;
    },
  );
  assert.deepEqual(flat.files["0-0.png"], png);
  await assert.rejects(
    extractSceneryProfile({ profiles: manifest.profiles }, "Fire", async () => {
      throw Object.assign(new Error("Unreadable frame"), { code: "EACCES" });
    }),
    /Unreadable frame/,
  );
});
