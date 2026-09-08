import sharp from "sharp";
import { Document, NodeIO } from "@gltf-transform/core";
import { ALL_EXTENSIONS, KHRMaterialsUnlit } from "@gltf-transform/extensions";
import { groundToScene, type MapCamera } from "@rle/shared";
import type { Geometry } from "./volume-geometry";
import type { Textured, Fill } from "./volume-fill";
const ZUP_TO_YUP: [number, number, number, number] = [
  -Math.SQRT1_2,
  0,
  0,
  Math.SQRT1_2,
];
export async function encode(
  tex: { width: number; height: number; rgba: Buffer },
  fill: Fill,
): Promise<{ data: Buffer; mime: string }> {
  const img = sharp(tex.rgba, {
    raw: { width: tex.width, height: tex.height, channels: 4 },
    limitInputPixels: false,
  });
  return fill === "none"
    ? {
        data: await img.png({ compressionLevel: 6 }).toBuffer(),
        mime: "image/png",
      }
    : {
        data: await img.removeAlpha().jpeg({ quality: 88 }).toBuffer(),
        mime: "image/jpeg",
      };
}

export async function exportGlb(
  file: string,
  g: Geometry,
  tex: Textured,
  cam: MapCamera,
  size: [number, number],
  fill: Fill,
) {
  const doc = new Document();
  const buffer = doc.createBuffer();
  const unlit = doc.createExtension(KHRMaterialsUnlit);
  const scene = doc.createScene("scene");
  const root = doc.createNode("map").setRotation(ZUP_TO_YUP);
  scene.addChild(root);

  const mkMat = (name: string, img: { data: Buffer; mime: string }) => {
    const texture = doc
      .createTexture(name)
      .setImage(img.data)
      .setMimeType(img.mime);
    const m = doc
      .createMaterial(name)
      .setBaseColorTexture(texture)
      .setDoubleSided(true)
      .setExtension("KHR_materials_unlit", unlit.createUnlit());
    if (fill === "none") m.setAlphaMode("MASK").setAlphaCutoff(0.5);
    return m;
  };
  const atlasMat = mkMat("atlas", await encode(tex.atlas, fill));
  const groundMat = mkMat("ground", await encode(tex.ground, fill));

  // one node per obstacle, grouped into buildings and terraces, all on the atlas material
  const buildings = doc.createNode("buildings");
  const terraces = doc.createNode("terraces");
  root.addChild(buildings);
  root.addChild(terraces);
  const facesOf = new Map<number, number[]>();
  g.faces.forEach((face, f) => {
    const list = facesOf.get(face.obstacle) ?? [];
    list.push(f);
    facesOf.set(face.obstacle, list);
  });
  for (const [obstacle, faceIds] of [...facesOf.entries()].sort(
    (a, b) => a[0] - b[0],
  )) {
    const remap = new Map<number, number>();
    const pos: number[] = [];
    const uvs: number[] = [];
    const idx: number[] = [];
    for (const f of faceIds) {
      for (const t of g.faces[f]!.tris) {
        for (let k = 0; k < 3; k++) {
          const v = g.tris[t * 3 + k]!;
          let local = remap.get(v);
          if (local === undefined) {
            local = remap.size;
            remap.set(v, local);
            pos.push(
              g.positions[v * 3]!,
              g.positions[v * 3 + 1]!,
              g.positions[v * 3 + 2]!,
            );
            uvs.push(tex.uvs[v * 2]!, tex.uvs[v * 2 + 1]!);
          }
          idx.push(local);
        }
      }
    }
    if (idx.length === 0) continue;
    const isTerrace = g.terraceIds.has(obstacle);
    const name = `${isTerrace ? "terrace" : "building"}-${String(obstacle).padStart(3, "0")}`;
    const position = doc
      .createAccessor(`${name}-position`)
      .setType("VEC3")
      .setArray(new Float32Array(pos))
      .setBuffer(buffer);
    const uv = doc
      .createAccessor(`${name}-uv`)
      .setType("VEC2")
      .setArray(new Float32Array(uvs))
      .setBuffer(buffer);
    const indices = doc
      .createAccessor(`${name}-indices`)
      .setType("SCALAR")
      .setArray(
        remap.size > 65535 ? new Uint32Array(idx) : new Uint16Array(idx),
      )
      .setBuffer(buffer);
    const mesh = doc
      .createMesh(name)
      .addPrimitive(
        doc
          .createPrimitive()
          .setAttribute("POSITION", position)
          .setAttribute("TEXCOORD_0", uv)
          .setIndices(indices)
          .setMaterial(atlasMat),
      );
    (isTerrace ? terraces : buildings).addChild(
      doc.createNode(name).setMesh(mesh),
    );
  }

  const [w, h] = size;
  const c = [
    groundToScene(cam, 0, 0),
    groundToScene(cam, w, 0),
    groundToScene(cam, w, h),
    groundToScene(cam, 0, h),
  ];
  const gpos = doc
    .createAccessor("ground-position")
    .setType("VEC3")
    .setArray(new Float32Array(c.flat()))
    .setBuffer(buffer);
  const guv = doc
    .createAccessor("ground-uv")
    .setType("VEC2")
    .setArray(new Float32Array([0, 0, 1, 0, 1, 1, 0, 1]))
    .setBuffer(buffer);
  const gidx = doc
    .createAccessor("ground-indices")
    .setType("SCALAR")
    .setArray(new Uint16Array([0, 2, 1, 0, 3, 2]))
    .setBuffer(buffer);
  const ground = doc
    .createMesh("ground")
    .addPrimitive(
      doc
        .createPrimitive()
        .setAttribute("POSITION", gpos)
        .setAttribute("TEXCOORD_0", guv)
        .setIndices(gidx)
        .setMaterial(groundMat),
    );
  root.addChild(doc.createNode("ground").setMesh(ground));

  const io = new NodeIO().registerExtensions(ALL_EXTENSIONS);
  await io.write(file, doc);
}

// ── renders ──────────────────────────────────────────────────────────
