import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import * as THREE from "three";
import type { ProjectionAssetEntry } from "@rle/shared";
import { AssetPreviewCache } from "./asset-preview-cache";

/** One context for the whole catalog, regardless of how many cards are visible. */
export class AssetPreviewRenderer {
  readonly cache = new AssetPreviewCache();
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
  let canvas: HTMLCanvasElement;
  let releaseLease: (() => void) | undefined;
  let observer: IntersectionObserver;
  let generation = 0;
  let visible = false;
  let draw: ((angle: number) => void) | undefined;

  function release() {
    generation++;
    draw = undefined;
    releaseLease?.();
    releaseLease = undefined;
  }
  async function load(root: FileSystemDirectoryHandle, entry: ProjectionAssetEntry) {
    const current = ++generation;
    try {
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
      draw = (angle) => {
        camera.position
          .copy(center)
          .add(
            new THREE.Vector3(Math.sin(angle) * 3, 1.9, Math.cos(angle) * 3).multiplyScalar(radius),
          );
        camera.lookAt(center);
        props.renderer.render(scene, camera, canvas);
      };
      draw(0.45);
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
      <Show when={status()}>
        <span class="preview-status">{status()}</span>
      </Show>
    </div>
  );
}
