// List static source triangles under a map-space pixel, before GPU depth/material rules.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createRequire } from "node:module";
import { readStoredMap } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { applyAffineMatrix, gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { sceneToMap } from "../shared/src/scene.ts";

const sharp = createRequire(new URL("../pipeline/package.json", import.meta.url))("sharp");

const [file, library, xText, yText] = process.argv.slice(2);
assert.ok(file && library && xText && yText, "Supply saved scene, library, map X and map Y");
const pixel = [Number(xText) + 0.5, Number(yText) + 0.5];
assert.ok(pixel.every(Number.isFinite));
const document = await readStoredMap(file, library);
assert.ok(!document.terrain && !document.splines?.length, "Inspect static asset scenes only");
const candidates = [];
const images = new Map();
const objects = document.objects.filter(
  (part) =>
    !part.hidden && !document.groups.some((group) => group.id === part.group && group.hidden),
);
for (const reference of document.assetSources ?? []) {
  const placed = objects.filter((part) => part.node.startsWith(`asset:${reference.id}:`));
  if (!placed.length) continue;
  const model = await loadSceneModel(library, reference);
  const root = model.getRoot().getDefaultScene();
  assert.ok(root);
  async function visit(node, placement, owner) {
    const extras = node.getExtras();
    if (extras.reveal_material_state === "revealed" || extras.reveal_show_when_applied) return;
    if (extras.default_hidden && !extras.reveal_material_patch && !extras.reveal_hide_when_applied)
      return;
    const world = node.getWorldMatrix();
    for (const primitive of node.getMesh()?.listPrimitives() ?? []) {
      assert.equal(primitive.getMode(), 4);
      const positions = primitive.getAttribute("POSITION");
      assert.ok(positions);
      const vertices = [];
      const projected = [];
      for (let i = 0; i < positions.getCount(); i++) {
        let point = gltfToScene(applyAffineMatrix(world, positions.getElement(i, [0, 0, 0])));
        if (placement) point = applyAffineMatrix(placement, point);
        vertices.push(point);
        projected.push(sceneToMap(document.camera, point));
      }
      const indices = primitive.getIndices()?.getArray() ?? Array.from(vertices.keys());
      for (let i = 0; i < indices.length; i += 3) {
        const ids = Array.from(indices.slice(i, i + 3));
        const [a, b, c] = ids.map((index) => projected[index]);
        const determinant = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if (Math.abs(determinant) < 1e-10) continue;
        const u =
          ((pixel[0] - a[0]) * (c[1] - a[1]) - (pixel[1] - a[1]) * (c[0] - a[0])) / determinant;
        const v =
          ((b[0] - a[0]) * (pixel[1] - a[1]) - (b[1] - a[1]) * (pixel[0] - a[0])) / determinant;
        if (u < 0 || v < 0 || u + v > 1) continue;
        const weights = [1 - u - v, u, v];
        const interpolate = (values) =>
          values[0].map((_, axis) =>
            values.reduce((sum, point, j) => sum + point[axis] * weights[j], 0),
          );
        const point = interpolate(ids.map((index) => vertices[index]));
        const material = primitive.getMaterial();
        const texture = material?.getBaseColorTexture();
        const texCoord = material?.getBaseColorTextureInfo()?.getTexCoord() ?? 0;
        const coords = primitive.getAttribute(`TEXCOORD_${texCoord}`);
        const uv = coords
          ? interpolate(ids.map((index) => coords.getElement(index, [0, 0])))
          : null;
        let sample = null;
        if (texture?.getImage() && uv) {
          let decoded = images.get(texture);
          if (!decoded) {
            decoded = await sharp(Buffer.from(texture.getImage()))
              .ensureAlpha()
              .raw()
              .toBuffer({ resolveWithObject: true });
            images.set(texture, decoded);
          }
          if (uv.every((value) => value >= 0 && value < 1)) {
            const offset =
              (Math.floor(uv[1] * decoded.info.height) * decoded.info.width +
                Math.floor(uv[0] * decoded.info.width)) *
              decoded.info.channels;
            sample = Array.from(decoded.data.subarray(offset, offset + decoded.info.channels));
          }
        }
        candidates.push({
          owner,
          node: node.getName(),
          triangle: i / 3,
          ground: sceneToGame(document.camera, point),
          determinant,
          uv,
          sample,
          material: material?.getName(),
          materialExtras: material?.getExtras(),
          color: material?.getBaseColorFactor(),
          metallic: material?.getMetallicFactor(),
          materialExtensions: material
            ?.listExtensions()
            .map((extension) => extension.extensionName),
          alphaMode: material?.getAlphaMode(),
          doubleSided: material?.getDoubleSided(),
          textureInfoExtensions: material
            ?.getBaseColorTextureInfo()
            ?.listExtensions()
            .map((extension) => extension.extensionName),
        });
      }
    }
    for (const child of node.listChildren()) await visit(child, placement, owner);
  }
  for (const part of placed) {
    const name = part.node.slice(`asset:${reference.id}:`.length);
    const matches = [];
    root.traverse((node) => {
      if (node.getName() === name) matches.push(node);
    });
    assert.equal(matches.length, 1, `Ambiguous pixel source: ${part.node}`);
    await visit(matches[0], partMatrix(document.camera, document, part), part.id);
  }
  images.clear();
}
candidates.sort((a, b) => b.ground[2] - a.ground[2]);
await fs.mkdir("work/needs-review/map-compile", { recursive: true });
const output = await fs.mkdtemp("work/needs-review/map-compile/pixel-inspection-");
const report = {
  scope: "projected-placed-asset-triangle-candidates-not-ground-or-GPU-visibility-proof",
  file,
  pixel,
  notes:
    "Samples use untransformed in-range UVs and nearest filtering; material transforms, alpha tests, culling, render order and lighting require separate review.",
  candidates,
};
await fs.writeFile(`${output}/report.json`, JSON.stringify(report));
console.log(output);
console.log(JSON.stringify(candidates.slice(0, 8)));
