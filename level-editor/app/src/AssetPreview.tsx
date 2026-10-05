import { createEffect, createSignal, onCleanup, Show, For } from "solid-js";
import * as THREE from "three";
import type { ProjectionAssetEntry } from "@rle/shared";
import { AssetPreviewPlaybackClock, type AssetPreviewPlayback } from "./asset-preview-playback.ts";
import { AssetPreviewCache } from "./asset-preview-cache";
import { loadSceneryThumbnail } from "./scenery-thumbnail.ts";
import { libraryFile } from "./projection-library.ts";
import { decodeSpritePixels } from "./entity-projection.ts";

/** One context for the whole catalog, regardless of how many cards are visible. */
export class AssetPreviewRenderer {
  readonly cache = new AssetPreviewCache();
  readonly clock = new AssetPreviewPlaybackClock();
  private renderer?: THREE.WebGLRenderer;
  render(scene: THREE.Scene, camera: THREE.Camera, canvas: HTMLCanvasElement) {
    const renderer = (this.renderer ??= new THREE.WebGLRenderer({ antialias: true, alpha: true }));
    renderer.setSize(320, 220, false);
    renderer.render(scene, camera);
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Asset preview canvas is unavailable");
    context.clearRect(0, 0, canvas.width, canvas.height);
    context.drawImage(renderer.domElement, 0, 0);
  }
  dispose() {
    this.clock.dispose();
    this.cache.dispose();
    this.renderer?.dispose();
    this.renderer?.forceContextLoss();
    this.renderer = undefined;
  }
}

export default function AssetPreview(props: {
  entry: ProjectionAssetEntry;
  root: FileSystemDirectoryHandle;
  renderer: AssetPreviewRenderer;
}) {
  const [status, setStatus] = createSignal("Loading 3D preview…");
  const [playback, setPlayback] = createSignal<AssetPreviewPlayback>();
  const [frame, setFrame] = createSignal(0);
  const [playing, setPlaying] = createSignal(false);
  const [lastFrame, setLastFrame] = createSignal(0);
  const [loop, setLoop] = createSignal(false);
  let unsubscribePlayback: (() => void) | undefined;
  let angle = 0.45;
  const syncPlayback = () => {
    const value = playback();
    if (!value) return;
    setFrame(value.tick);
    setPlaying(value.playing);
    setLastFrame(value.lastTick);
    setLoop(value.loop);
    draw?.(angle);
  };
  const changePlayback = (change: (value: AssetPreviewPlayback) => void) => {
    const value = playback();
    if (!value) return;
    change(value);
    syncPlayback();
    props.renderer.clock.request();
  };
  let canvas: HTMLCanvasElement;
  let releaseLease: (() => void) | undefined;
  let observer: IntersectionObserver;
  let generation = 0;
  let visible = false;
  let draw: ((angle: number) => void) | undefined;

  function release() {
    generation++;
    draw = undefined;
    unsubscribePlayback?.();
    unsubscribePlayback = undefined;
    setPlayback(undefined);
    releaseLease?.();
    releaseLease = undefined;
  }
  async function load(root: FileSystemDirectoryHandle, entry: ProjectionAssetEntry) {
    const current = ++generation;
    try {
      const thumbnail =
        entry.editor &&
        (await loadSceneryThumbnail(
          entry.editor,
          async (name) => new Uint8Array(await (await libraryFile(root, name)).arrayBuffer()),
        ));
      if (!visible || current !== generation) return;
      if (thumbnail) {
        const bitmap = await createImageBitmap(
          new Blob([new Uint8Array(thumbnail.png)], { type: "image/png" }),
        );
        try {
          if (!visible || current !== generation) return;
          const image = document.createElement("canvas");
          image.width = bitmap.width;
          image.height = bitmap.height;
          const pixels = image.getContext("2d");
          const target = canvas.getContext("2d");
          if (!pixels || !target) throw new Error("Scenery thumbnail canvas is unavailable");
          pixels.drawImage(bitmap, 0, 0);
          const rgba = pixels.getImageData(0, 0, image.width, image.height);
          decodeSpritePixels(rgba.data, thumbnail.legacy);
          pixels.putImageData(rgba, 0, 0);
          const scale = Math.min(300 / image.width, 200 / image.height);
          target.clearRect(0, 0, canvas.width, canvas.height);
          target.imageSmoothingEnabled = false;
          target.drawImage(
            image,
            (canvas.width - image.width * scale) / 2,
            (canvas.height - image.height * scale) / 2,
            image.width * scale,
            image.height * scale,
          );
          setStatus("");
        } finally {
          bitmap.close();
        }
        return;
      }
      const loaded = await props.renderer.cache.acquire(root, entry);
      if (!visible || current !== generation) {
        loaded.release();
        return;
      }
      releaseLease = () => loaded.release();
      const asset = loaded.asset;
      const scene = new THREE.Scene();
      scene.add(asset, new THREE.HemisphereLight(0xffffff, 0x8c93aa, 2.5));
      const light = new THREE.DirectionalLight(0xffffff, 2);
      light.position.set(3, 8, 5);
      scene.add(light);
      const box = new THREE.Box3();
      asset.updateWorldMatrix(true, true);
      asset.traverseVisible((node) => {
        if (node instanceof THREE.Mesh) box.expandByObject(node);
      });
      if (box.isEmpty()) {
        let gameplayOnly = false;
        asset.traverse((node) => {
          gameplayOnly ||= node.userData.gameplay_only === true;
        });
        if (!gameplayOnly) throw new Error("Model has no visible geometry");
        setStatus(entry.name);
        return;
      }
      const center = box.getCenter(new THREE.Vector3());
      const radius = Math.max(box.getBoundingSphere(new THREE.Sphere()).radius, 0.01);
      const camera = new THREE.PerspectiveCamera(35, 320 / 220, radius / 100, radius * 20);
      draw = (nextAngle) => {
        angle = nextAngle;
        camera.position
          .copy(center)
          .add(
            new THREE.Vector3(Math.sin(angle) * 3, 1.9, Math.cos(angle) * 3).multiplyScalar(radius),
          );
        camera.lookAt(center);
        props.renderer.render(scene, camera, canvas);
      };
      draw(0.45);
      if (loaded.playback) {
        setPlayback(loaded.playback);
        unsubscribePlayback = props.renderer.clock.register(loaded.playback, syncPlayback);
        syncPlayback();
      }
      setStatus("");
    } catch (error) {
      if (current === generation) {
        release();
        setStatus(`Preview unavailable: ${String(error)}`);
      }
    }
  }
  createEffect(
    () => ({ root: props.root, entry: props.entry }),
    ({ root, entry }) => {
      if (!visible) return;
      release();
      setStatus("Loading 3D preview…");
      void load(root, entry);
    },
  );
  onCleanup(() => {
    visible = false;
    observer?.disconnect();
    release();
  });
  return (
    <div
      class="asset-preview"
      style={{ "aspect-ratio": playback() ? "auto" : undefined }}
      onPointerMove={(event) => {
        if (!draw) return;
        const rect = event.currentTarget.getBoundingClientRect();
        draw(((event.clientX - rect.left) / rect.width) * Math.PI * 2);
      }}
      onPointerLeave={() => draw?.(0.45)}
    >
      <canvas
        width="320"
        height="220"
        aria-label={`3D preview of ${props.entry.name}`}
        style={{ height: playback() ? "auto" : undefined, "aspect-ratio": "320 / 220" }}
        ref={(element) => {
          canvas = element;
          observer = new IntersectionObserver((entries) => {
            const next = entries[0]?.isIntersecting ?? false;
            if (next === visible) return;
            visible = next;
            if (visible) {
              setStatus("Loading 3D preview…");
              void load(props.root, props.entry);
            } else release();
          });
          observer.observe(element);
        }}
      />
      <Show when={playback()}>
        {(value) => (
          <div
            class="preview-playback"
            style={{
              position: "relative",
              background: "#202833ee",
              padding: "6px",
              display: "flex",
              "flex-wrap": "wrap",
              gap: "6px",
              "font-size": "12px",
            }}
            onClick={(event) => event.stopPropagation()}
            onPointerMove={(event) => event.stopPropagation()}
            onPointerDown={(event) => event.stopPropagation()}
          >
            <Show when={value().clips.length > 1}>
              <select
                aria-label="Preview animation"
                onChange={(event) => changePlayback((p) => p.select(event.currentTarget.value))}
              >
                <For each={value().clips}>
                  {(clip) => <option value={clip.name}>{clip.name}</option>}
                </For>
              </select>
            </Show>
            <button
              type="button"
              onClick={() => changePlayback((p) => (p.playing ? p.pause() : p.play()))}
            >
              {playing() ? "Pause" : "Play"}
            </button>
            <label>
              Preview behavior{" "}
              <select
                value={loop() ? "loop" : "once"}
                onChange={(event) =>
                  changePlayback((p) => p.setLoop(event.currentTarget.value === "loop"))
                }
              >
                <option value="once">Play once</option>
                <option value="loop">Loop</option>
              </select>
            </label>
            <label style={{ "min-width": "0", width: "100%" }}>
              Frame {frame()}{" "}
              <input
                aria-label="Preview frame"
                style={{ width: "100%", "min-width": "0", "box-sizing": "border-box" }}
                type="range"
                min="0"
                max={lastFrame()}
                step="1"
                value={frame()}
                onInput={(event) =>
                  changePlayback((p) => p.seek(Number(event.currentTarget.value)))
                }
              />
            </label>
          </div>
        )}
      </Show>
      <Show when={status()}>
        <span class="preview-status">{status()}</span>
      </Show>
    </div>
  );
}
