import test from "node:test";
import assert from "node:assert/strict";
import { Document } from "@gltf-transform/core";
import sharp from "sharp";
import { maskRecoveryMesh, maskRecoveryTextures } from "./mask-recovery-mesh.ts";

function fixture(indexed = true) {
  const model = new Document();
  const buffer = model.createBuffer();
  const positions = model
    .createAccessor()
    .setBuffer(buffer)
    .setType("VEC3")
    .setArray(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]));
  const primitive = model.createPrimitive().setAttribute("POSITION", positions);
  if (indexed)
    primitive.setIndices(
      model
        .createAccessor()
        .setBuffer(buffer)
        .setType("SCALAR")
        .setArray(new Uint16Array([2, 1, 0])),
    );
  const mesh = model.createMesh().addPrimitive(primitive);
  const child = model
    .createNode("mesh")
    .setTranslation([0, 2, 0])
    .setScale([2, 3, 1])
    .setMesh(mesh);
  const part = model.createNode("part").setTranslation([10, 0, 0]).addChild(child);
  const scene = model.createScene().addChild(part);
  model.getRoot().setDefaultScene(scene);
  return { model, primitive, part, scene };
}

test("mesh recovery applies descendant and parent transforms for indexed and raw triangles", () => {
  for (const indexed of [true, false]) {
    const { model } = fixture(indexed);
    const triangle = maskRecoveryMesh(model, "part", ([x, y, z]) => [x + 100, y, z])[0]!;
    const expected = [
      [110, 2, 0],
      [112, 2, 0],
      [110, 5, 0],
    ];
    assert.deepEqual(triangle, indexed ? expected.reverse() : expected);
  }
});

test("mesh recovery requires an unambiguous part in the selected scene", () => {
  const { model, scene } = fixture();
  model.createNode("part"); // An unreachable node is not a duplicate in this state.
  assert.equal(maskRecoveryMesh(model, "part", (p) => p).length, 1);
  assert.throws(() => maskRecoveryMesh(model, "missing", (p) => p), /one selected part/);
  scene.addChild(model.createNode("part"));
  assert.throws(() => maskRecoveryMesh(model, "part", (p) => p), /one selected part/);
});

test("culled mask authoring preserves mixed material sidedness without changing legacy coverage", () => {
  const { model, primitive } = fixture();
  primitive.setMaterial(model.createMaterial().setDoubleSided(false));
  const single = maskRecoveryMesh(model, "part", (p) => p);
  model
    .getRoot()
    .listMeshes()[0]!
    .addPrimitive(primitive.clone().setMaterial(model.createMaterial().setDoubleSided(true)));
  assert.deepEqual(
    maskRecoveryMesh(model, "part", (p) => p),
    [...single, ...single],
  );
  const result = maskRecoveryMesh(model, "part", (p) => p, undefined, undefined, {
    preserveMaterialSidedness: true,
  });
  assert.deepEqual(result, [single[0], single[0], [...single[0]!].reverse()]);
});

test("mesh recovery rejects unsupported coverage and malformed geometry", () => {
  for (const alpha of ["BLEND"] as const) {
    const { model, primitive } = fixture();
    primitive.setMaterial(model.createMaterial().setAlphaMode(alpha));
    assert.throws(() => maskRecoveryMesh(model, "part", (p) => p), /texture coverage/);
  }
  const { model, primitive } = fixture();
  primitive.getIndices()!.setArray(new Uint16Array([0, 1, 20]));
  assert.throws(() => maskRecoveryMesh(model, "part", (p) => p), /outside positions/);
  primitive.getIndices()!.setArray(new Uint16Array([0, 1]));
  assert.throws(() => maskRecoveryMesh(model, "part", (p) => p), /incomplete triangles/);
  primitive.getIndices()!.setArray(new Uint16Array([0, 1, 2]));
  assert.throws(() => maskRecoveryMesh(model, "part", () => [NaN, 0, 0]), /Invalid placed/);
  model.createAnimation();
  assert.throws(() => maskRecoveryMesh(model, "part", (p) => p), /static model state/);
});

test("cutout mesh recovery decodes alpha and keeps foliage ownership separate", async () => {
  const { model, primitive } = fixture(false);
  const texture = model.createTexture().setImage(
    await sharp({
      create: {
        width: 2,
        height: 2,
        channels: 4,
        background: { r: 255, g: 255, b: 255, alpha: 1 },
      },
    })
      .png()
      .toBuffer(),
  );
  const material = model
    .createMaterial()
    .setAlphaMode("MASK")
    .setBaseColorTexture(texture)
    .setAlphaCutoff(0.5);
  material.getBaseColorTextureInfo()!.setMagFilter(9728);
  primitive.setMaterial(material);
  const buffer = model.getRoot().listBuffers()[0]!;
  primitive.setAttribute(
    "TEXCOORD_0",
    model
      .createAccessor()
      .setBuffer(buffer)
      .setType("VEC2")
      .setArray(new Float32Array([0, 0, 1, 0, 0, 1])),
  );
  primitive.setAttribute(
    "COLOR_0",
    model.createAccessor().setBuffer(buffer).setType("VEC4").setArray(new Float32Array(12)),
  );
  assert.throws(() => maskRecoveryMesh(model, "part", (p) => p), /texture coverage/);
  const textures = await maskRecoveryTextures(model);
  assert.deepEqual(
    maskRecoveryMesh(model, "part", (p) => p, textures),
    [],
  );
  material.setExtras({
    foliage_physical_opacity: true,
    opacity_semantics: "physical-coverage",
    source_ownership_semantics: "separate-mask",
    source_ownership_channel: "vertex-color-r",
  });
  assert.deepEqual(
    maskRecoveryMesh(model, "part", (p) => p, textures),
    [
      [
        [10, 2, 0],
        [12, 2, 0],
        [10, 5, 0],
      ],
    ],
  );
  const covered = maskRecoveryMesh(model, "part", (p) => p, textures);
  material.setDoubleSided(true);
  // Only half the texture is covered; reversed faces must retain the clipped
  // geometry rather than restore the source triangle over transparent texels.
  const partial = new Map(textures);
  partial.set(texture, { width: 2, height: 2, alpha: new Uint8Array([255, 0, 255, 0]) });
  const clipped = maskRecoveryMesh(model, "part", (p) => p, partial);
  assert.ok(clipped.length > 0);
  assert.notDeepEqual(clipped, covered);
  assert.deepEqual(
    maskRecoveryMesh(model, "part", (p) => p, partial, undefined, {
      preserveMaterialSidedness: true,
    }),
    clipped.flatMap((triangle) => [triangle, [...triangle].reverse()]),
  );
  primitive.getAttribute("TEXCOORD_0")!.setArray(new Float32Array([-2, 3, -2, 3, -2, 3]));
  assert.throws(() => maskRecoveryMesh(model, "part", (p) => p, textures), /in-range/);
  material.getBaseColorTextureInfo()!.setWrapS(33071).setWrapT(33071);
  assert.deepEqual(
    maskRecoveryMesh(model, "part", (p) => p, textures),
    covered,
  );
  material.getBaseColorTextureInfo()!.setMagFilter(9729);
  assert.throws(() => maskRecoveryMesh(model, "part", (p) => p, textures), /nearest/);
  material.setAlphaMode("OPAQUE");
  assert.equal(maskRecoveryMesh(model, "part", (p) => p).length, 1);
});

test("review bounds discard only triangles outside the placed projection", () => {
  const { model, primitive } = fixture();
  primitive
    .getAttribute("POSITION")!
    .setArray(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 100, 0, 0, 101, 0, 0, 100, 1, 0]));
  primitive.getIndices()!.setArray(new Uint16Array([2, 1, 0, 3, 4, 5]));
  const place = ([x, y, z]: [number, number, number]): [number, number, number] => [
    x,
    y + 50,
    z + 50,
  ];
  const full = maskRecoveryMesh(model, "part", place);
  assert.equal(full.length, 2);
  const bounds = [{ left: 10, top: 2, right: 11, bottom: 3 }];
  assert.deepEqual(maskRecoveryMesh(model, "part", place, undefined, bounds), [full[0]]);
  assert.deepEqual(
    maskRecoveryMesh(model, "part", place, undefined, [
      { left: 10, top: 52, right: 11, bottom: 53 },
    ]),
    [],
  );
  assert.deepEqual(
    maskRecoveryMesh(model, "part", place, undefined, [
      ...bounds,
      { left: 210, top: 2, right: 211, bottom: 3 },
    ]),
    full,
  );
  assert.throws(
    () =>
      maskRecoveryMesh(model, "part", place, undefined, [{ left: 1, top: 0, right: 0, bottom: 1 }]),
    /Invalid mask recovery bounds/,
  );
});
