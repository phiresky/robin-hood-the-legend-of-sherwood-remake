import * as THREE from "three";
import { disposeObjectResources } from "./resources.ts";

/** Owns browser and GPU lifetimes, never the editable clones sharing asset resources. */
export class EditorViewport {
  readonly listeners = new AbortController();
  renderer: THREE.WebGLRenderer | null = null;
  private sourceAsset: THREE.Object3D | null = null;
  private ground: THREE.Object3D | null = null;
  get groundNode() {
    return this.ground;
  }
  private observer: ResizeObserver | null = null;
  private animationFrame = 0;
  private controls: { dispose(): void }[] = [];
  private disposed = false;

  installMap(
    asset: THREE.Object3D,
    ground: THREE.Object3D | null,
    parent: THREE.Object3D,
  ) {
    if (this.disposed || this.sourceAsset)
      throw new Error(
        "Viewport cannot adopt another map before retiring its current asset",
      );
    this.sourceAsset = asset;
    this.ground = ground;
    if (ground) parent.add(ground);
  }

  ownControl<T extends { dispose(): void }>(control: T): T {
    this.controls.push(control);
    return control;
  }
  observe(element: Element, resize: () => void) {
    this.observer?.disconnect();
    this.observer = new ResizeObserver(resize);
    this.observer.observe(element);
  }
  animate(render: () => void) {
    const tick = () => {
      if (this.disposed) return;
      render();
      if (!this.disposed) this.animationFrame = requestAnimationFrame(tick);
    };
    tick();
  }
  retireMap(overlay: THREE.Object3D) {
    disposeObjectResources([
      overlay,
      ...(this.sourceAsset ? [this.sourceAsset] : []),
      ...(this.groundNode ? [this.groundNode] : []),
    ]);
    this.groundNode?.removeFromParent();
    this.sourceAsset = null;
    this.ground = null;
  }
  dispose(
    overlay: THREE.Object3D,
    selectionBox: THREE.Object3D,
    clearSelection: () => void,
  ) {
    if (this.disposed) return;
    this.disposed = true;
    this.listeners.abort();
    cancelAnimationFrame(this.animationFrame);
    this.observer?.disconnect();
    clearSelection();
    for (const control of this.controls.reverse()) control.dispose();
    this.controls = [];
    // One disposal traversal deduplicates resources shared by these owning roots.
    disposeObjectResources([
      overlay,
      selectionBox,
      ...(this.sourceAsset ? [this.sourceAsset] : []),
      ...(this.groundNode ? [this.groundNode] : []),
    ]);
    this.sourceAsset = null;
    this.ground = null;
    this.renderer?.dispose();
    this.renderer?.forceContextLoss();
    this.renderer?.domElement.remove();
    this.renderer = null;
  }
}
