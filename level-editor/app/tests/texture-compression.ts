import * as THREE from "three";
import { MapTextureCompressor } from "../src/texture-compression";
import { disposeObjectResources } from "../src/resources";

export async function checkTextureCompression() {
  const assert = (value: unknown, message: string) => {
    if (!value) throw new Error(message);
  };
  async function texture(width = 256, alpha = 1) {
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = 256;
    const ctx = canvas.getContext("2d")!;
    ctx.fillStyle = `rgba(180,90,30,${alpha})`;
    ctx.fillRect(0, 0, width, 256);
    const image = new Image();
    image.src = canvas.toDataURL();
    await image.decode();
    const value = new THREE.Texture(image);
    value.colorSpace = THREE.SRGBColorSpace;
    value.flipY = false;
    value.repeat.set(0.75, 0.5);
    value.offset.set(0.1, 0.2);
    value.needsUpdate = true;
    return value;
  }
  const root = new THREE.Group();
  const add = (map: THREE.Texture, alphaTest = 0) => {
    const material = new THREE.MeshBasicMaterial({ map, alphaTest });
    root.add(new THREE.Mesh(new THREE.PlaneGeometry(), material));
    return material;
  };
  const original = await texture();
  const first = add(original),
    shared = add(original);
  const second = add(await texture()),
    third = add(await texture());
  const foliage = add(await texture(), 0.5),
    unaligned = add(await texture(255)),
    ownership = add(await texture(256, 0.5));
  const untouched = [foliage.map, unaligned.map, ownership.map];
  const phases: number[] = [];
  const pool = new MapTextureCompressor((active) => phases.push(active));
  const renderer = new THREE.WebGLRenderer();
  try {
    if (!renderer.extensions.has("EXT_texture_compression_bptc"))
      throw new Error("BC7 regression test requires BPTC support");
    await pool.compress(root);
    assert(first.map instanceof THREE.CompressedTexture, "Opaque map was not compressed");
    assert(first.map === shared.map, "Shared texture was encoded separately");
    assert(
      second.map instanceof THREE.CompressedTexture && third.map instanceof THREE.CompressedTexture,
      "Queued textures were not encoded",
    );
    assert(
      foliage.map === untouched[0] &&
        unaligned.map === untouched[1] &&
        ownership.map === untouched[2],
      "Alpha/UV-sensitive texture changed",
    );
    assert(
      first.map!.repeat.equals(original.repeat) && first.map!.offset.equals(original.offset),
      "UV transforms changed",
    );
    assert(first.map!.colorSpace === THREE.SRGBColorSpace, "Texture color space changed");
    assert(
      Math.max(...phases) === Math.min(2, navigator.hardwareConcurrency || 2),
      "Encoding did not use its bounded worker pool",
    );
    renderer.initTexture(first.map!);
    first.map!.needsUpdate = true;
    renderer.initTexture(first.map!);
    assert(renderer.getContext().getError() === 0, "Compressed texture upload/reupload failed");
  } finally {
    pool.dispose();
    renderer.dispose();
    renderer.forceContextLoss();
    disposeObjectResources([root]);
  }
  const controller = new AbortController();
  const cancelled = new MapTextureCompressor(undefined, controller.signal);
  const pendingRoot = new THREE.Group();
  pendingRoot.add(
    new THREE.Mesh(
      new THREE.PlaneGeometry(),
      new THREE.MeshBasicMaterial({ map: await texture(1024) }),
    ),
  );
  try {
    const pending = cancelled.compress(pendingRoot);
    controller.abort();
    await pending.then(
      () => {
        throw new Error("Cancelled compression resolved");
      },
      (error) => assert(error.name === "AbortError", "Cancellation lost its reason"),
    );
  } finally {
    cancelled.dispose();
    disposeObjectResources([pendingRoot]);
  }
  return "PASS BC7 worker concurrency, sharing, alpha/UV fallback, sampler settings, reupload and cancellation";
}
