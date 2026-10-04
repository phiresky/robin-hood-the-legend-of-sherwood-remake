/** Decoded atlas images are shared across poses while a mission preview is built. */
export class SpriteAtlasImages {
  private images = new Map<string, Promise<ImageBitmap>>();
  get(directory: FileSystemDirectoryHandle, key: string, filename: string) {
    if (!filename || /[\\/]/.test(filename) || filename === "." || filename === "..")
      throw new Error("Invalid sprite atlas path");
    const identity = key + "/" + filename;
    let image = this.images.get(identity);
    if (!image) {
      image = directory
        .getFileHandle(filename)
        .then(async (file) => createImageBitmap(await file.getFile()));
      this.images.set(identity, image);
    }
    return image;
  }
  dispose() {
    for (const image of this.images.values())
      void image.then(
        (bitmap) => bitmap.close(),
        () => {},
      );
    this.images.clear();
  }
}

export { spriteAtlasRect } from "../../shared/src/sprite-atlas-rect.ts";
