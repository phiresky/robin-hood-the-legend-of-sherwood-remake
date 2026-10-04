import * as THREE from "three";
import { decodeSpritePixels } from "./entity-projection.ts";
import { loadSceneryBank } from "./scenery-thumbnail.ts";

/** Frame progression includes the initial sentinel tick and each frame's inclusive delay. */
export function sceneryFrameAtTick(delays: readonly number[], tick: number) {
  if (
    !delays.length ||
    delays.some((delay) => !Number.isInteger(delay) || delay < 0 || delay > 65535)
  )
    throw new Error("Invalid scenery frame delays");
  if (!Number.isSafeInteger(tick) || tick < 0) throw new Error("Invalid scenery preview tick");
  // The native unsigned counter wraps before it can exceed a maximum delay.
  const period = delays.includes(65535)
    ? Infinity
    : delays.reduce((sum, delay) => sum + delay + 1, 0);
  let remaining = Math.max(0, tick - 1) % period;
  for (const [index, delay] of delays.entries()) {
    if (delay === 65535 || remaining <= delay) return index;
    remaining -= delay + 1;
  }
  throw new Error("Scenery frame timing exceeded its period");
}

/** Animated preview resources are independent of the model and never enter baked artwork. */
export async function loadSceneryFrames(
  ...args: [...Parameters<typeof loadSceneryBank>, elevation: number]
) {
  const [descriptor, animation, read, elevation] = args;
  const { files, profile, legacy } = await loadSceneryBank(descriptor, animation, read);
  const textures = new Map<string, THREE.CanvasTexture<OffscreenCanvas>>();
  const frames: {
    geometry: THREE.PlaneGeometry;
    texture: THREE.CanvasTexture<OffscreenCanvas>;
    delay: number;
  }[] = [];
  const dispose = () => {
    for (const texture of textures.values()) texture.dispose();
    for (const frame of frames) frame.geometry.dispose();
  };
  try {
    for (const frame of profile.rows[0]!.frames) {
      let texture = textures.get(frame.path);
      if (!texture) {
        const bitmap = await createImageBitmap(
          new Blob([new Uint8Array(files[frame.path]!)], { type: "image/png" }),
        );
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        const context = canvas.getContext("2d");
        if (!context) {
          bitmap.close();
          throw new Error("Cannot decode scenery pixels");
        }
        context.drawImage(bitmap, 0, 0);
        bitmap.close();
        const pixels = context.getImageData(0, 0, canvas.width, canvas.height);
        decodeSpritePixels(pixels.data, legacy);
        context.putImageData(pixels, 0, 0);
        texture = new THREE.CanvasTexture(canvas);
        texture.colorSpace = THREE.SRGBColorSpace;
        texture.magFilter = THREE.NearestFilter;
        textures.set(frame.path, texture);
      }
      const canvas = texture.image;
      const cosine = Math.cos((elevation * Math.PI) / 180);
      const geometry = new THREE.PlaneGeometry(canvas.width, canvas.height / cosine);
      geometry.translate(
        frame.offsetX - profile.center_x + canvas.width / 2,
        (profile.center_y - frame.offsetY - canvas.height / 2) / cosine,
        0,
      );
      frames.push({ geometry, texture, delay: frame.delay });
    }
    return { frames, dispose };
  } catch (error) {
    dispose();
    throw error;
  }
}
