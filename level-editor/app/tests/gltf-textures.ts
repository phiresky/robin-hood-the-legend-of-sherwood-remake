import { Texture } from "three";
import { createGltfLoader } from "../src/gltf-loader";

/** Embedded images remain uploadable after GLTFLoader revokes their blob URLs. */
export async function checkGltfTextures() {
  const png =
    "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAEUlEQVR4nGOsDr/MwMDAUA8AC7wCJh1FdDUAAAAASUVORK5CYII=";
  const gltf = await createGltfLoader().parseAsync(
    JSON.stringify({
      asset: { version: "2.0" },
      buffers: [{ uri: `data:application/octet-stream;base64,${png}`, byteLength: 74 }],
      bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 74 }],
      images: [{ bufferView: 0, mimeType: "image/png" }],
      textures: [{ source: 0 }],
      scenes: [{ nodes: [] }],
      scene: 0,
    }),
    "",
  );
  const texture: Texture = await gltf.parser.getDependency("texture", 0);
  if (!(texture.image instanceof HTMLImageElement))
    throw new Error("GLTF texture pins decoded bitmap pixels");
  // Test separate contexts, including RGB beneath zero alpha used for ownership.
  for (let i = 0; i < 2; i++) {
    const gl = document.createElement("canvas").getContext("webgl2")!;
    const gpuTexture = gl.createTexture();
    const framebuffer = gl.createFramebuffer();
    try {
      gl.bindTexture(gl.TEXTURE_2D, gpuTexture);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, texture.image);
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, gpuTexture, 0);
      const pixels = new Uint8Array(8);
      gl.readPixels(0, 0, 2, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      if (gl.getError() !== gl.NO_ERROR || pixels.join() !== "123,87,211,0,123,87,211,127") {
        throw new Error(`GLTF image upload changed ownership/color pixels: ${pixels}`);
      }
    } finally {
      gl.deleteFramebuffer(framebuffer);
      gl.deleteTexture(gpuTexture);
      gl.getExtension("WEBGL_lose_context")?.loseContext();
    }
  }
  texture.dispose();
}
