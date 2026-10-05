import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import sharp from "sharp";
import { auxiliaryReferences } from "./refinement/auxiliary-references.ts";

const sha = (bytes: Buffer) => crypto.createHash("sha256").update(bytes).digest("hex");
test("Material examples carry distinct provenance and reject stale image bytes", async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "texture-material-"));
  try {
    const input = await sharp({ create: { width: 8, height: 8, channels: 4, background: "gray" } }).png().toBuffer();
    const example = await sharp({ create: { width: 16, height: 12, channels: 4, background: "green" } }).png().toBuffer();
    const file = path.join(directory, "references.json");
    const reference = { file: "example.png", sha256: sha(example), source: "material", asset_id: "permitted-tree", role: "Leaf and bark texture character" };
    await fs.writeFile(path.join(directory, "example.png"), example);
    await fs.writeFile(file, JSON.stringify({ input_sha256: sha(input), lighting_sha256: sha(input), references: [reference] }));
    const loaded = await auxiliaryReferences(file, input, input);
    assert.deepEqual(loaded.images, [example]);
    assert.deepEqual(loaded.evidence?.references, [reference]);
    assert.match(loaded.instructions, /Image 3.*permitted-tree/);
    assert.match(loaded.instructions, /do not copy its shape/);
    assert.match(loaded.instructions, /protected source pixels/);
    await fs.writeFile(path.join(directory, "example.png"), input);
    await assert.rejects(auxiliaryReferences(file, input, input), /hash changed/);
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
});

test("Auxiliary crop evidence binds exact approved pixels and rejects altered artwork", async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "texture-auxiliary-"));
  try {
    const input = await sharp({
      create: { width: 8, height: 8, channels: 4, background: { r: 123, g: 45, b: 67, alpha: 1 } },
    })
      .png()
      .toBuffer();
    const crop = { left: 2, top: 3, width: 4, height: 2 };
    const reference = await sharp(input)
      .extract(crop)
      .resize(12, 6, { kernel: "nearest" })
      .png()
      .toBuffer();
    const manifest = {
      input_sha256: sha(input),
      lighting_sha256: sha(input),
      references: [
        { file: "crop.png", sha256: sha(reference), source: "lighting", crop, scale: 3 },
      ],
    };
    const file = path.join(directory, "references.json");
    await fs.writeFile(path.join(directory, "crop.png"), reference);
    await fs.writeFile(file, JSON.stringify(manifest));
    const loaded = await auxiliaryReferences(file, input, input);
    assert.equal(loaded.images.length, 1);
    assert.equal(loaded.evidence?.references[0]?.sha256, sha(reference));
    assert.equal(loaded.evidence?.manifest_sha256, sha(await fs.readFile(file)));
    assert.match(loaded.instructions, /Image 3.*3x.*second.*\(2,3,4,2\)/);
    await assert.rejects(auxiliaryReferences(file, Buffer.from("changed"), input), /bind/);
    await assert.rejects(auxiliaryReferences(file, input, null), /lighting/);
    const altered = await sharp({
      create: { width: 12, height: 6, channels: 4, background: "red" },
    })
      .png()
      .toBuffer();
    await fs.writeFile(path.join(directory, "crop.png"), altered);
    await assert.rejects(auxiliaryReferences(file, input, input), /hash changed/);
    manifest.references[0]!.sha256 = sha(altered);
    await fs.writeFile(file, JSON.stringify(manifest));
    await assert.rejects(auxiliaryReferences(file, input, input), /exact magnified approved crop/);
    assert.deepEqual(await auxiliaryReferences(null, input, null), {
      images: [],
      evidence: null,
      instructions: "",
    });
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
});

test("An ordinary region guide accompanies four references and remains canvas-bound", async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "texture-region-guide-"));
  try {
    const input = await sharp({ create: { width: 8, height: 8, channels: 4, background: "gray" } }).png().toBuffer();
    const guide = await sharp({ create: { width: 8, height: 8, channels: 4, background: "yellow" } }).png().toBuffer();
    await fs.writeFile(path.join(directory, "input.png"), input);
    await fs.writeFile(path.join(directory, "guide.png"), guide);
    const material = { source: "material", asset_id: "native-ground", role: "floor", file: "input.png", sha256: sha(input) };
    const region = { source: "region-guide", role: "Yellow is editable", file: "guide.png", sha256: sha(guide) };
    const manifest = { input_sha256: sha(input), lighting_sha256: sha(input), references: [material, material, material, material, region] };
    const file = path.join(directory, "references.json");
    await fs.writeFile(file, JSON.stringify(manifest));
    const loaded = await auxiliaryReferences(file, input, input);
    assert.equal(loaded.images.length, 5);
    assert.match(loaded.instructions, /Image 7.*ordinary image, not a provider edit mask/);
    assert.match(loaded.instructions, /never copy its diagnostic colors/);
    const wrongSize = await sharp(guide).resize(7, 8).png().toBuffer();
    await fs.writeFile(path.join(directory, "guide.png"), wrongSize);
    await assert.rejects(auxiliaryReferences(file, input, input), /hash changed/);
    region.sha256 = sha(wrongSize);
    await fs.writeFile(file, JSON.stringify(manifest));
    await assert.rejects(auxiliaryReferences(file, input, input), /exact input canvas/);
    manifest.references.push(region);
    await fs.writeFile(file, JSON.stringify(manifest));
    await assert.rejects(auxiliaryReferences(file, input, input), /at most one/);
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
});
