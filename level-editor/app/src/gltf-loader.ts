import { LoadingManager, TextureLoader } from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { MeshoptDecoder } from "meshoptimizer";

export function createGltfLoader(manager?: LoadingManager): GLTFLoader {
  const loader = new GLTFLoader(manager).setMeshoptDecoder(MeshoptDecoder);
  loader.register((parser) => {
    // HTML images let the browser evict decoded pixels after GPU upload, while
    // remaining usable for texture reuploads and additional renderers. Retained
    // ImageBitmaps pin a second, uncompressed copy of every atlas in CPU memory.
    parser.textureLoader = new TextureLoader(loader.manager)
      .setCrossOrigin(parser.options.crossOrigin)
      .setRequestHeader(parser.options.requestHeader);
    return { name: "RLE_reloadable_texture_images" };
  });
  return loader;
}
