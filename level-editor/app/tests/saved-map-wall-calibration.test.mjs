import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { BoxGeometry, Mesh, MeshBasicMaterial } from "three";
import { wallSplineFixture } from "../../shared/test-fixtures/wall-spline.ts";
import { prepareWallGameplayAssets } from "../src/wall-gameplay-calibration.ts";
import { savedMapWallCalibration } from "./saved-map-wall-calibration.mjs";

const require = createRequire(new URL("../../pipeline/package.json", import.meta.url));
const { Document, NodeIO } = require("@gltf-transform/core");

test("saved wall calibration matches editor source frames and rejects changed model bytes", async () => {
  const library = await fs.mkdtemp(path.join(os.tmpdir(), "wall-calibration-"));
  const geometry = new BoxGeometry(100, 20, 40).translate(0, 0, 20);
  const material = new MeshBasicMaterial();
  try {
    const model = new Document();
    const buffer = model.createBuffer();
    const positions = model
      .createAccessor()
      .setType("VEC3")
      .setArray(geometry.getAttribute("position").array)
      .setBuffer(buffer);
    const indices = model
      .createAccessor()
      .setType("SCALAR")
      .setArray(geometry.index.array)
      .setBuffer(buffer);
    const primitive = model
      .createPrimitive()
      .setAttribute("POSITION", positions)
      .setIndices(indices);
    const mesh = model.createMesh().addPrimitive(primitive);
    const body = model.createNode("body").setMesh(mesh).setTranslation([25, 17, 4]);
    const group = model.createNode("wall").addChild(body);
    const wrapper = model
      .createNode("map")
      .setRotation([-Math.SQRT1_2, 0, 0, Math.SQRT1_2])
      .addChild(group);
    model.createScene("default").addChild(wrapper);
    const bytes = await new NodeIO().writeBinary(model);
    await fs.writeFile(path.join(library, "wall.glb"), bytes);
    const { document, assets } = wallSplineFixture();
    document.assetSources = [
      {
        id: "wall",
        role: "objects",
        model: "wall.glb",
        resources: [],
        model_sha256: createHash("sha256").update(bytes).digest("hex"),
      },
    ];
    document.splines[0].sourceAngle = 31;
    document.splines[0].sourceStraight = false;
    const source = new Mesh(geometry, material);
    source.position.set(25, 17, 4);
    const expected = prepareWallGameplayAssets(
      document,
      assets,
      new Map([["asset:wall:body", source]]),
    );
    assert.deepEqual(expected.warnings, []);
    const actual = await savedMapWallCalibration(document, assets, library);
    assert.deepEqual(actual.warnings, []);
    assert.deepEqual(actual.assets, expected.assets);
    assert.deepEqual(source.position.toArray(), [25, 17, 4]);
    await fs.appendFile(path.join(library, "wall.glb"), "changed");
    const changed = await savedMapWallCalibration(document, assets, library);
    assert.ok(changed.warnings.some((warning) => warning.includes("Asset changed: wall.glb")));
    assert.equal(changed.assets.get("wall"), assets.get("wall"));
  } finally {
    geometry.dispose();
    material.dispose();
    await fs.rm(library, { recursive: true, force: true });
  }
});
