import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { Document, NodeIO } from "@gltf-transform/core";
import { savedMapBakeBounds } from "../../app/tests/saved-map-bounds.mjs";
import { IDENTITY_TRANSFORM } from "../../shared/src/level3d.ts";

test("saved-scene bounds use placed vertices when size is absent and verify model pins", async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "saved-map-bounds-"));
  try {
    const model = new Document();
    const positions = model
      .createAccessor()
      .setBuffer(model.createBuffer())
      .setType("VEC3")
      .setArray(new Float32Array([0, 0, 0, 10, 0, 0, 0, 0, 20]));
    const part = model
      .createNode("part")
      .setTranslation([2, 0, 5])
      .setMesh(
        model
          .createMesh()
          .addPrimitive(model.createPrimitive().setAttribute("POSITION", positions)),
      );
    model.getRoot().setDefaultScene(model.createScene("default").addChild(part));
    const bytes = await new NodeIO().writeBinary(model);
    await fs.writeFile(path.join(directory, "model.glb"), bytes);
    const reference = {
      id: "fixture",
      model: "model.glb",
      resources: [],
      model_sha256: createHash("sha256").update(bytes).digest("hex"),
    };
    const document = {
      camera: { kind: "oblique-orthographic", elevation_deg: 30 },
      objects: [
        {
          id: "placed",
          node: "asset:fixture:part",
          kind: "scenery",
          transform: { ...IDENTITY_TRANSFORM, dx: 100, dy: 200 },
        },
      ],
      groups: [],
      sceneAssets: [],
      assetSources: [reference],
    };
    assert.deepEqual(await savedMapBakeBounds(document, directory), [102, 202, 11, 12]);
    assert.deepEqual(
      await savedMapBakeBounds({ ...document, size: [300, 400] }, directory),
      [0, 0, 300, 400],
    );
    assert.deepEqual(
      await savedMapBakeBounds({ ...document, exportBounds: [1, 2, 3, 4] }, directory),
      [1, 2, 3, 4],
    );
    await assert.rejects(
      savedMapBakeBounds({ ...document, terrain: {} }, directory),
      /browser bake/,
    );
    reference.model_sha256 = "0".repeat(64);
    await assert.rejects(savedMapBakeBounds(document, directory), /Asset changed/);
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
});
