import type { Document, Node, Texture } from "@gltf-transform/core";
import sharp from "sharp";
import { maskAlphaCoverage, type MaskAlphaImage } from "./mask-alpha-coverage.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import type { MaskTriangle } from "../../shared/src/compile-mask-geometry.ts";
import type { MaskCoverageRectangle } from "./recover-mask-bitmap.ts";

/** Decode physical alpha only; opaque atlas alpha may instead encode provenance. */
export async function maskRecoveryTextures(model: Document): Promise<Map<Texture, MaskAlphaImage>> {
  const textures = new Map<Texture, MaskAlphaImage>();
  for (const material of model.getRoot().listMaterials()) {
    if (material.getAlphaMode() !== "MASK") continue;
    const texture = material.getBaseColorTexture();
    if (!texture || textures.has(texture)) continue;
    const image = texture.getImage();
    if (!image) throw new Error(`Missing mask alpha image: ${texture.getName()}`);
    const { data, info } = await sharp(Buffer.from(image))
      .toColourspace("srgb")
      .ensureAlpha()
      .raw()
      .toBuffer({ resolveWithObject: true });
    if (info.channels !== 4) throw new Error("Mask alpha image needs RGBA channels");
    textures.set(texture, {
      width: info.width,
      height: info.height,
      alpha: Uint8Array.from({ length: info.width * info.height }, (_, i) => data[i * 4 + 3]!),
    });
  }
  return textures;
}

/** Read a selected part's static surfaces, including descendant mesh
 * transforms. The caller converts model-world glTF coordinates into placed game
 * coordinates. Cutouts use explicitly decoded nearest-sampled base-level alpha. */
export function maskRecoveryMesh(
  model: Document,
  part: string,
  place: (point: Vec3) => Vec3,
  textures?: ReadonlyMap<Texture, MaskAlphaImage>,
  projectedBounds?: readonly MaskCoverageRectangle[],
  options: { preserveMaterialSidedness?: boolean } = {},
): MaskTriangle[] {
  if (
    projectedBounds?.some(
      (b) =>
        ![b.left, b.top, b.right, b.bottom].every(Number.isFinite) ||
        b.left >= b.right ||
        b.top >= b.bottom,
    )
  )
    throw new Error("Invalid mask recovery bounds");
  const scene = model.getRoot().getDefaultScene();
  if (!scene) throw new Error("Mask recovery model has no selected scene");
  const matches: Node[] = [];
  scene.traverse((node) => {
    if (node.getName() === part) matches.push(node);
  });
  if (matches.length !== 1) throw new Error(`Mask recovery needs one selected part: ${part}`);
  if (model.getRoot().listAnimations().length)
    throw new Error("Mask recovery requires a static model state");
  const triangles: MaskTriangle[] = [];
  let sourceTriangles = 0;
  matches[0]!.traverse((node) => {
    if (node.getSkin()) throw new Error(`Mask recovery does not support skinned part ${part}`);
    const matrix = node.getWorldMatrix();
    for (const primitive of node.getMesh()?.listPrimitives() ?? []) {
      if (primitive.getMode() !== 4 || primitive.listTargets().length)
        throw new Error(`Mask recovery requires static triangles: ${part}`);
      const material = primitive.getMaterial();
      // A culled mask can represent mixed material sidedness by retaining an
      // opposite winding only for faces whose material renders both sides.
      // Do this after alpha clipping so the reverse face has identical holes.
      const append = (triangle: MaskTriangle) => {
        triangles.push(triangle);
        if (options.preserveMaterialSidedness && material?.getDoubleSided())
          triangles.push([triangle[2], triangle[1], triangle[0]]);
      };
      const mode = material?.getAlphaMode() ?? "OPAQUE";
      if (mode === "BLEND") throw new Error(`Mask recovery needs texture coverage for ${part}`);
      const texture = mode === "MASK" ? material!.getBaseColorTexture() : null;
      const info = texture ? material!.getBaseColorTextureInfo()! : null;
      const alphaImage = texture ? textures?.get(texture) : undefined;
      if (texture && !alphaImage)
        throw new Error(`Mask recovery needs texture coverage for ${part}`);
      if (info && (info.getMagFilter() !== 9728 || info.listExtensions().length))
        throw new Error(
          `Mask recovery requires nearest alpha sampling without texture transforms: ${part}`,
        );
      const uv = info ? primitive.getAttribute(`TEXCOORD_${info.getTexCoord()}`) : null;
      if (info && (!uv || uv.getElementSize() !== 2))
        throw new Error(`Mask recovery has invalid alpha texture coordinates: ${part}`);
      const colors = mode === "MASK" ? primitive.getAttribute("COLOR_0") : null;
      const foliage = mode === "MASK" && material!.getExtras().foliage_physical_opacity === true;
      if (
        foliage &&
        (!colors ||
          material!.getExtras().opacity_semantics !== "physical-coverage" ||
          material!.getExtras().source_ownership_semantics !== "separate-mask" ||
          material!.getExtras().source_ownership_channel !== "vertex-color-r")
      )
        throw new Error(`Mask recovery needs explicit foliage opacity semantics: ${part}`);
      if (colors && ![3, 4].includes(colors.getElementSize()))
        throw new Error(`Mask recovery has invalid vertex alpha: ${part}`);
      const positions = primitive.getAttribute("POSITION");
      if (!positions || positions.getElementSize() !== 3)
        throw new Error(`Mask recovery has invalid positions: ${part}`);
      const indices = primitive.getIndices();
      if (indices && indices.getElementSize() !== 1)
        throw new Error(`Mask recovery has invalid indices: ${part}`);
      const count = indices?.getCount() ?? positions.getCount();
      if (count % 3) throw new Error(`Mask recovery has incomplete triangles: ${part}`);
      sourceTriangles += count / 3;
      const cache = new Map<number, Vec3>();
      const point = (offset: number): Vec3 => {
        const index = indices?.getScalar(offset) ?? offset;
        if (!Number.isInteger(index) || index < 0 || index >= positions.getCount())
          throw new Error(`Mask recovery index outside positions: ${part}`);
        const cached = cache.get(index);
        if (cached) return cached;
        const [x, y, z] = positions.getElement(index, [0, 0, 0]);
        const world: Vec3 = [
          matrix[0] * x + matrix[4] * y + matrix[8] * z + matrix[12],
          matrix[1] * x + matrix[5] * y + matrix[9] * z + matrix[13],
          matrix[2] * x + matrix[6] * y + matrix[10] * z + matrix[14],
        ];
        if (!world.every(Number.isFinite)) throw new Error(`Invalid mesh point: ${part}`);
        const placed = place(world);
        if (placed.length !== 3 || !placed.every(Number.isFinite))
          throw new Error(`Invalid placed mesh point: ${part}`);
        cache.set(index, placed);
        return placed;
      };
      for (let offset = 0; offset < count; offset += 3) {
        const triangle: MaskTriangle = [point(offset), point(offset + 1), point(offset + 2)];
        if (projectedBounds) {
          const left = Math.min(...triangle.map((p) => p[0]));
          const right = Math.max(...triangle.map((p) => p[0]));
          const top = Math.min(...triangle.map((p) => p[1] - p[2]));
          const bottom = Math.max(...triangle.map((p) => p[1] - p[2]));
          if (
            !projectedBounds.some(
              (b) => left < b.right && right > b.left && top < b.bottom && bottom > b.top,
            )
          )
            continue;
        }
        if (mode === "OPAQUE") {
          append(triangle);
          continue;
        }
        const indicesForTriangle = [0, 1, 2].map(
          (i) => indices?.getScalar(offset + i) ?? offset + i,
        );
        const coordinates = indicesForTriangle.map((index): [number, number] => {
          if (!uv) return [0, 0];
          if (index >= uv.getCount())
            throw new Error(`Mask alpha UV index outside coordinates: ${part}`);
          return uv.getElement(index, [0, 0]) as [number, number];
        });
        // A constant coordinate on a repeat seam samples the first texel.
        for (const axis of [0, 1] as const)
          if (
            coordinates.every((p) => p[axis] === 1) &&
            (axis === 0 ? info?.getWrapS() : info?.getWrapT()) === 10497
          )
            for (const coordinate of coordinates) coordinate[axis] = 0;
        const vertexAlpha = indicesForTriangle.map((index) => {
          if (colors && index >= colors.getCount())
            throw new Error(`Mask vertex alpha index outside colors: ${part}`);
          return (
            material!.getBaseColorFactor()[3] *
            (!foliage && colors?.getElementSize() === 4
              ? colors.getElement(index, [0, 0, 0, 0])[3]!
              : 1)
          );
        });
        for (const covered of maskAlphaCoverage(
          triangle,
          coordinates,
          vertexAlpha,
          material!.getAlphaCutoff(),
          alphaImage,
          [info?.getWrapS() === 33071, info?.getWrapT() === 33071],
        ))
          append(covered);
      }
    }
  });
  if (!sourceTriangles) throw new Error(`Mask recovery part has no surfaces: ${part}`);
  return triangles;
}
