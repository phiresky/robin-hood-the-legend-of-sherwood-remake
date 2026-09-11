import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { TransformControls } from "three/examples/jsm/controls/TransformControls.js";
import {
  sceneToGame,
  gameToScene,
  gameTransformMatrix,
  groupCentroid,
  groupParts,
  obstacleCentroid,
  transformedObstacle,
  type GameTransform,
  type Level3D,
  type Level3DObject,
  type ProtoLevel,
  type Vec3,
} from "@rle/shared";
import type { Selection } from "./document-commands.ts";
import { disposeObjectResources } from "./resources.ts";

interface View {
  wrapper: THREE.Group;
  rot: THREE.Group;
  meshes: THREE.Mesh[];
}
interface CameraState {
  position: THREE.Vector3;
  quaternion: THREE.Quaternion;
  target: THREE.Vector3;
  frustum: number;
  zoom: number;
}
export interface ViewportBindings {
  document(): Level3D | null;
  selection(): Selection;
  level(): ProtoLevel | null;
  showObstacles(): boolean;
  showElevation(): boolean;
  onSelection(selection: Selection): void;
  commitTransform(transform: GameTransform): void;
}

/** Owns the entire scene projection and its browser/GPU lifetime. Document and
 * selection are borrowed from the session/UI, never copied into another model.
 * Editable clones share source resources; only source roots own their disposal. */
export class EditorViewport {
  readonly listeners = new AbortController();
  private renderer: THREE.WebGLRenderer | null = null;
  private container: HTMLDivElement | null = null;
  private camera: THREE.OrthographicCamera | null = null;
  private frustum = 1500;
  private orbit: OrbitControls | null = null;
  private gizmo: TransformControls | null = null;
  private readonly scene = new THREE.Scene();
  private readonly mapRoot = new THREE.Group();
  private readonly objectsRoot = new THREE.Group();
  private readonly overlayRoot = new THREE.Group();
  private readonly partViews = new Map<string, View>();
  private readonly groupViews = new Map<string, View>();
  private readonly sourceNodes = new Map<string, THREE.Object3D>();
  private readonly raycaster = new THREE.Raycaster();
  private readonly selectionBox = new THREE.Box3Helper(
    new THREE.Box3(),
    0xffcc40,
  );
  private readonly tinted = new Map<
    THREE.Mesh,
    THREE.Material | THREE.Material[]
  >();
  private dragging = false;
  private cancelPointerGesture: (() => void) | null = null;
  private flight: {
    from: CameraState;
    to: CameraState;
    start: number;
    ms: number;
  } | null = null;

  private readonly bindings: ViewportBindings;
  constructor(bindings: ViewportBindings) {
    this.bindings = bindings;
    this.scene.background = new THREE.Color(0x1c1c1c);
    this.mapRoot.quaternion.set(-Math.SQRT1_2, 0, 0, Math.SQRT1_2);
    this.scene.add(this.mapRoot);
    this.mapRoot.add(this.objectsRoot, this.overlayRoot);
    this.selectionBox.visible = false;
    this.scene.add(this.selectionBox);
  }
  private selectedPart() {
    const s = this.bindings.selection();
    return s?.kind === "part"
      ? (this.bindings.document()?.objects.find((o) => o.id === s.id) ?? null)
      : null;
  }
  private selectedGroup() {
    const s = this.bindings.selection();
    return s?.kind === "group"
      ? (this.bindings.document()?.groups.find((g) => g.id === s.id) ?? null)
      : null;
  }
  setGizmoVertical(vertical: boolean) {
    if (this.gizmo) this.gizmo.showY = vertical;
  }

  private sourceAsset: THREE.Object3D | null = null;
  private ground: THREE.Object3D | null = null;
  get groundNode() {
    return this.ground;
  }
  private observer: ResizeObserver | null = null;
  private animationFrame = 0;
  private controls: { dispose(): void }[] = [];
  private disposed = false;

  replaceMap(
    asset: THREE.Object3D,
    ground: THREE.Object3D | null,
    sources: ReadonlyMap<string, THREE.Object3D>,
  ) {
    if (this.disposed) throw new Error("Disposed viewport cannot adopt a map");
    this.retireMap();
    this.sourceAsset = asset;
    this.ground = ground;
    if (ground) this.mapRoot.add(ground);
    for (const [key, value] of sources) this.sourceNodes.set(key, value);
  }

  private ownControl<T extends { dispose(): void }>(control: T): T {
    this.controls.push(control);
    return control;
  }
  private observe(element: Element, resize: () => void) {
    this.observer?.disconnect();
    this.observer = new ResizeObserver(resize);
    this.observer.observe(element);
  }
  private animate(render: () => void) {
    const tick = () => {
      if (this.disposed) return;
      render();
      if (!this.disposed) this.animationFrame = requestAnimationFrame(tick);
    };
    tick();
  }
  private retireMap() {
    this.cancelPointerGesture?.();
    this.flight = null;
    this.select(null);
    disposeObjectResources([
      this.overlayRoot,
      ...(this.sourceAsset ? [this.sourceAsset] : []),
      ...(this.groundNode ? [this.groundNode] : []),
    ]);
    this.groundNode?.removeFromParent();
    this.sourceAsset = null;
    this.ground = null;
    this.objectsRoot.clear();
    this.overlayRoot.clear();
    this.partViews.clear();
    this.groupViews.clear();
    this.sourceNodes.clear();
  }
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.listeners.abort();
    if (this.animationFrame) cancelAnimationFrame(this.animationFrame);
    this.observer?.disconnect();
    this.retireMap();
    for (const control of this.controls.reverse()) control.dispose();
    this.controls = [];
    disposeObjectResources([this.selectionBox]);
    this.renderer?.dispose();
    this.renderer?.forceContextLoss();
    this.renderer?.domElement.remove();
    this.renderer = null;
    this.camera = null;
    this.orbit = null;
    this.gizmo = null;
    this.container = null;
    this.cancelPointerGesture = null;
    this.scene.clear();
  }
  setup(el: HTMLDivElement) {
    if (this.disposed || this.renderer)
      throw new Error("Viewport can only mount once");
    this.container = el;
    this.renderer = new THREE.WebGLRenderer({ antialias: true });
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    el.appendChild(this.renderer.domElement);
    this.camera = new THREE.OrthographicCamera(-1, 1, 1, -1, -100000, 100000);
    this.camera.position.set(0, 2000, 3000);
    this.orbit = this.ownControl(
      new OrbitControls(this.camera, this.renderer.domElement),
    );
    this.orbit.enableDamping = true;
    this.orbit.zoomToCursor = true;
    // left drag pans (or moves the selection, handler below), right drag
    // orbits around the point under the cursor (our own handler, OrbitControls
    // ignores the button), wheel zooms to the cursor
    this.orbit.mouseButtons = {
      LEFT: THREE.MOUSE.PAN,
      MIDDLE: THREE.MOUSE.DOLLY,
      RIGHT: null as unknown as THREE.MOUSE,
    };
    this.setupCursorOrbit(this.renderer.domElement);
    this.gizmo = this.ownControl(
      new TransformControls(this.camera, this.renderer.domElement),
    );
    this.gizmo.setMode("translate");
    this.gizmo.showY = false;
    this.scene.add(this.gizmo.getHelper());
    this.gizmo.addEventListener("dragging-changed", (e) => {
      this.dragging = !!(e as unknown as { value: boolean }).value;
      if (this.orbit) this.orbit.enabled = !this.dragging;
      if (!this.dragging) this.commitGizmo();
    });
    this.gizmo.addEventListener("objectChange", () =>
      this.refreshSelectionBox(),
    );
    const resize = () => {
      const w = el.clientWidth;
      const h = el.clientHeight;
      if (!this.renderer || !this.camera || w === 0 || h === 0) return;
      this.renderer.setSize(w, h, false);
      this.renderer.setPixelRatio(window.devicePixelRatio);
      this.applyFrustum();
    };
    this.observe(el, resize);
    resize();
    // click = pick (a click that did not orbit); alt-click picks a single part
    let downAt: [number, number] | null = null;
    this.renderer.domElement.addEventListener(
      "pointerdown",
      (e) => {
        if (e.button === 0) downAt = [e.clientX, e.clientY];
      },
      { signal: this.listeners.signal },
    );
    this.renderer.domElement.addEventListener(
      "pointerup",
      (e) => {
        if (!downAt || e.button !== 0) return;
        const moved = Math.hypot(e.clientX - downAt[0], e.clientY - downAt[1]);
        downAt = null;
        if (moved > 4 || this.dragging) return;
        this.pick(e, e.altKey);
      },
      { signal: this.listeners.signal },
    );
    this.animate(() => {
      if (!this.renderer || !this.camera) return;
      if (this.flight) this.stepFlight();
      else this.orbit?.update();
      this.renderer.render(this.scene, this.camera);
    });
  }

  private currentState(): CameraState {
    return {
      position: this.camera!.position.clone(),
      quaternion: this.camera!.quaternion.clone(),
      target: this.orbit!.target.clone(),
      frustum: this.frustum,
      zoom: this.camera!.zoom,
    };
  }

  private flyTo(to: CameraState, ms = 700) {
    if (!this.camera || !this.orbit) return;
    this.flight = {
      from: this.currentState(),
      to,
      start: performance.now(),
      ms,
    };
    this.orbit.enabled = false;
  }

  private stepFlight() {
    if (!this.flight || !this.camera || !this.orbit) return;
    const raw = Math.min(
      1,
      (performance.now() - this.flight.start) / this.flight.ms,
    );
    const t = raw < 0.5 ? 2 * raw * raw : 1 - Math.pow(-2 * raw + 2, 2) / 2; // ease in-out
    const { from, to } = this.flight;
    this.camera.position.lerpVectors(from.position, to.position, t);
    this.camera.quaternion.slerpQuaternions(from.quaternion, to.quaternion, t);
    this.orbit.target.lerpVectors(from.target, to.target, t);
    this.frustum = from.frustum + (to.frustum - from.frustum) * t;
    this.camera.zoom = from.zoom + (to.zoom - from.zoom) * t;
    this.applyFrustum();
    if (raw >= 1) {
      this.flight = null;
      this.camera.up.set(0, 1, 0);
      this.orbit.enabled = true;
      this.orbit.update();
    }
  }

  private lookState(
    position: THREE.Vector3,
    target: THREE.Vector3,
    frustumSize: number,
  ): CameraState {
    const probe = new THREE.OrthographicCamera();
    probe.position.copy(position);
    probe.up.set(0, 1, 0);
    probe.lookAt(target);
    return {
      position: position.clone(),
      quaternion: probe.quaternion.clone(),
      target: target.clone(),
      frustum: frustumSize,
      zoom: 1,
    };
  }

  private partOfHit(h: THREE.Intersection): Level3DObject | null {
    let node: THREE.Object3D | null = h.object;
    while (node && !this.partViews.has(node.name)) node = node.parent;
    return node
      ? (this.bindings.document()?.objects.find((o) => o.id === node!.name) ??
          null)
      : null;
  }

  private setupCursorOrbit(el: HTMLCanvasElement) {
    let active: {
      pivot: THREE.Vector3;
      startX: number;
      startY: number;
      position: THREE.Vector3;
      quaternion: THREE.Quaternion;
      target: THREE.Vector3;
      right: THREE.Vector3;
      polar: number;
    } | null = null;
    let moving: {
      view: View;
      plane: THREE.Plane;
      start: THREE.Vector3;
      startPos: THREE.Vector3;
    } | null = null;
    let capturedPointer: number | null = null;
    this.cancelPointerGesture = () => {
      active = null;
      moving = null;
      this.dragging = false;
      if (capturedPointer !== null && el.hasPointerCapture(capturedPointer))
        el.releasePointerCapture(capturedPointer);
      capturedPointer = null;
      if (this.orbit) this.orbit.enabled = true;
    };
    const up = new THREE.Vector3(0, 1, 0);
    // the right button is ours now, so OrbitControls no longer swallows the context menu
    el.addEventListener("contextmenu", (e) => e.preventDefault(), {
      signal: this.listeners.signal,
    });
    const setRay = (e: PointerEvent) => {
      const rect = el.getBoundingClientRect();
      const ndc = new THREE.Vector2(
        ((e.clientX - rect.left) / rect.width) * 2 - 1,
        -((e.clientY - rect.top) / rect.height) * 2 + 1,
      );
      this.raycaster.setFromCamera(ndc, this.camera!);
    };
    el.addEventListener(
      "pointerdown",
      (e) => {
        // the gizmo takes precedence when the cursor is on one of its handles
        if (
          !this.camera ||
          !this.orbit ||
          this.gizmo?.axis ||
          (e.button !== 0 && e.button !== 2)
        )
          return;
        setRay(e);
        const hits = this.raycaster.intersectObjects(
          [this.objectsRoot, ...(this.groundNode ? [this.groundNode] : [])],
          true,
        );
        if (e.button === 0) {
          // a left drag that starts on the selection moves it along the ground plane
          const s = this.bindings.selection();
          const hitPart = hits[0] ? this.partOfHit(hits[0]) : null;
          const view = this.selectedView();
          if (
            s &&
            hitPart &&
            view &&
            (s.kind === "part" ? hitPart.id === s.id : hitPart.group === s.id)
          ) {
            const plane = new THREE.Plane(up, -hits[0]!.point.y);
            moving = {
              view,
              plane,
              start: hits[0]!.point.clone(),
              startPos: view.wrapper.position.clone(),
            };
            this.orbit.enabled = false;
            this.dragging = true;
            el.setPointerCapture(e.pointerId);
            capturedPointer = e.pointerId;
          }
          return;
        }
        const pivot = hits[0]?.point.clone() ?? this.orbit.target.clone();
        const offset = this.camera.position.clone().sub(this.orbit.target);
        active = {
          pivot,
          startX: e.clientX,
          startY: e.clientY,
          position: this.camera.position.clone(),
          quaternion: this.camera.quaternion.clone(),
          target: this.orbit.target.clone(),
          right: new THREE.Vector3(1, 0, 0).applyQuaternion(
            this.camera.quaternion,
          ),
          polar: Math.acos(THREE.MathUtils.clamp(offset.normalize().y, -1, 1)),
        };
        this.orbit.enabled = false;
        this.dragging = true;
        el.setPointerCapture(e.pointerId);
        capturedPointer = e.pointerId;
      },
      { signal: this.listeners.signal },
    );
    el.addEventListener(
      "pointermove",
      (e) => {
        if (moving && this.camera) {
          setRay(e);
          const point = new THREE.Vector3();
          if (!this.raycaster.ray.intersectPlane(moving.plane, point)) return;
          // the delta in the wrapper's parent frame (a group's local frame for parts)
          const parent = moving.view.wrapper.parent!;
          const delta = parent
            .worldToLocal(point.clone())
            .sub(parent.worldToLocal(moving.start.clone()));
          moving.view.wrapper.position.copy(moving.startPos).add(delta);
          this.refreshSelectionBox();
          return;
        }
        if (!active || !this.camera || !this.orbit) return;
        const rect = el.getBoundingClientRect();
        const yaw = (-(e.clientX - active.startX) / rect.width) * Math.PI * 2;
        let pitch = (-(e.clientY - active.startY) / rect.height) * Math.PI;
        // keep the camera between straight down and just above the horizon
        pitch =
          THREE.MathUtils.clamp(
            active.polar + pitch,
            0.02,
            Math.PI / 2 - 0.02,
          ) - active.polar;
        const q = new THREE.Quaternion()
          .setFromAxisAngle(up, yaw)
          .multiply(
            new THREE.Quaternion().setFromAxisAngle(active.right, pitch),
          );
        this.camera.position
          .copy(active.position)
          .sub(active.pivot)
          .applyQuaternion(q)
          .add(active.pivot);
        this.camera.quaternion.copy(q).multiply(active.quaternion);
        this.camera.up.copy(up);
        this.orbit.target
          .copy(active.target)
          .sub(active.pivot)
          .applyQuaternion(q)
          .add(active.pivot);
      },
      { signal: this.listeners.signal },
    );
    const end = (e: PointerEvent) => {
      if (e.button === 0 && moving) {
        moving = null;
        this.commitGizmo();
      } else if (e.button === 2 && active) active = null;
      else return;
      this.dragging = false;
      if (capturedPointer !== null && el.hasPointerCapture(capturedPointer))
        el.releasePointerCapture(capturedPointer);
      capturedPointer = null;
      if (this.orbit) {
        this.orbit.enabled = true;
        this.orbit.update();
      }
    };
    el.addEventListener("pointerup", end, {
      signal: this.listeners.signal,
    });
    el.addEventListener("pointercancel", end, {
      signal: this.listeners.signal,
    });
  }

  private applyFrustum() {
    if (!this.camera || !this.container) return;
    const aspect =
      this.container.clientWidth / Math.max(1, this.container.clientHeight);
    this.camera.left = -this.frustum * aspect;
    this.camera.right = this.frustum * aspect;
    this.camera.top = this.frustum;
    this.camera.bottom = -this.frustum;
    this.camera.updateProjectionMatrix();
  }

  private contentBox(): THREE.Box3 {
    const box = new THREE.Box3();
    if (this.groundNode) box.expandByObject(this.groundNode);
    box.expandByObject(this.objectsRoot);
    return box;
  }

  frameContent(instant = false) {
    if (!this.camera || !this.orbit) return;
    const box = this.contentBox();
    if (box.isEmpty()) return;
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3()).length();
    const to = this.lookState(
      center.clone().add(new THREE.Vector3(0, size * 0.5, size * 0.6)),
      center,
      size * 0.35,
    );
    if (instant) this.applyState(to);
    else this.flyTo(to);
  }

  gameCamera(instant = false) {
    const d = this.bindings.document();
    if (!this.camera || !this.orbit) return;
    const box = this.contentBox();
    if (box.isEmpty()) return;
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3()).length();
    const t = ((d?.camera.elevation_deg ?? 35) * Math.PI) / 180;
    const forward = new THREE.Vector3(0, -Math.sin(t), -Math.cos(t));
    const to = this.lookState(
      center.clone().addScaledVector(forward, -size),
      center,
      d ? d.size[1] / 2 : size * 0.35,
    );
    if (instant) this.applyState(to);
    else this.flyTo(to);
  }

  private applyState(st: CameraState) {
    if (!this.camera || !this.orbit) return;
    this.flight = null;
    this.camera.position.copy(st.position);
    this.camera.quaternion.copy(st.quaternion);
    this.camera.up.set(0, 1, 0);
    this.orbit.target.copy(st.target);
    this.frustum = st.frustum;
    this.camera.zoom = st.zoom;
    this.applyFrustum();
    this.orbit.enabled = true;
    this.orbit.update();
  }

  private makeView(name: string): View {
    const wrapper = new THREE.Group();
    wrapper.name = name;
    const rot = new THREE.Group();
    rot.matrixAutoUpdate = false;
    wrapper.add(rot);
    return { wrapper, rot, meshes: [] };
  }

  private setAffine(v: View, m: number[]) {
    v.wrapper.position.set(m[12]!, m[13]!, m[14]!);
    const rest = new THREE.Matrix4().fromArray(m);
    rest.setPosition(0, 0, 0);
    v.rot.matrix.copy(rest);
    v.rot.matrixWorldNeedsUpdate = true;
  }

  syncViews(d: Level3D) {
    const aliveGroups = new Set<string>();
    for (const g of d.groups) {
      aliveGroups.add(g.id);
      let v = this.groupViews.get(g.id);
      if (!v) {
        v = this.makeView(g.id);
        this.objectsRoot.add(v.wrapper);
        this.groupViews.set(g.id, v);
      }
      this.setAffine(
        v,
        gameTransformMatrix(
          d.camera,
          g.transform,
          groupCentroid(groupParts(d, g.id)),
        ),
      );
      v.wrapper.visible = !g.hidden;
    }
    for (const [id, v] of this.groupViews) {
      if (aliveGroups.has(id)) continue;
      this.objectsRoot.remove(v.wrapper);
      this.groupViews.delete(id);
    }
    const aliveParts = new Set<string>();
    for (const o of d.objects) {
      aliveParts.add(o.id);
      let v = this.partViews.get(o.id);
      if (!v) {
        const src = this.sourceNodes.get(o.node);
        if (!src) throw new Error(`Missing source node ${o.node} for ${o.id}`);
        v = this.makeView(o.id);
        const node = src.clone(true);
        node.traverse((c) => {
          const m = c as THREE.Mesh;
          if (m.isMesh) v!.meshes.push(m);
        });
        v.rot.add(node);
        this.partViews.set(o.id, v);
      }
      const parent =
        (o.group ? this.groupViews.get(o.group)?.rot : undefined) ??
        this.objectsRoot;
      if (v.wrapper.parent !== parent) parent.add(v.wrapper);
      this.setAffine(
        v,
        gameTransformMatrix(
          d.camera,
          o.transform,
          obstacleCentroid(o.obstacle.points),
        ),
      );
      v.wrapper.visible = !o.hidden;
    }
    for (const [id, v] of this.partViews) {
      if (aliveParts.has(id)) continue;
      v.wrapper.parent?.remove(v.wrapper);
      this.partViews.delete(id);
    }
    const s = this.bindings.selection();
    if (s && !(s.kind === "group" ? aliveGroups : aliveParts).has(s.id))
      this.select(null);
    else this.refreshSelectionBox();
    if (this.bindings.showObstacles()) this.buildOverlays();
  }

  private selectedView(): View | null {
    const s = this.bindings.selection();
    if (!s) return null;
    return (
      (s.kind === "group" ? this.groupViews : this.partViews).get(s.id) ?? null
    );
  }

  private commitGizmo() {
    const d = this.bindings.document();
    const v = this.selectedView();
    const t = this.selectedGroup()?.transform ?? this.selectedPart()?.transform;
    if (!d || !v || !t) return;
    const g = this.selectedGroup();
    const p = this.selectedPart();
    const pivot = g
      ? groupCentroid(groupParts(d, g.id))
      : obstacleCentroid(p!.obstacle.points);
    const base = gameTransformMatrix(
      d.camera,
      { ...t, dx: 0, dy: 0, dz: 0 },
      pivot,
    );
    const pos = v.wrapper.position;
    const [dx, dy, dz] = sceneToGame(d.camera, [
      pos.x - base[12]!,
      pos.y - base[13]!,
      pos.z - base[14]!,
    ]).map((v) => Math.round(v * 10) / 10) as Vec3;
    if (dx === t.dx && dy === t.dy && dz === t.dz) return;
    this.bindings.commitTransform({ ...t, dx, dy, dz });
  }

  private pick(e: PointerEvent, partOnly: boolean) {
    if (!this.camera || !this.renderer) return;
    const rect = this.renderer.domElement.getBoundingClientRect();
    const ndc = new THREE.Vector2(
      ((e.clientX - rect.left) / rect.width) * 2 - 1,
      -((e.clientY - rect.top) / rect.height) * 2 + 1,
    );
    this.raycaster.setFromCamera(ndc, this.camera);
    const hits = this.raycaster.intersectObject(this.objectsRoot, true);
    for (const h of hits) {
      const part = this.partOfHit(h);
      if (!part) continue;
      // a click inside the selected building picks its part; alt always does
      const s = this.bindings.selection();
      if (
        part.group &&
        !partOnly &&
        !(s?.kind === "group" && s.id === part.group)
      )
        this.select({ kind: "group", id: part.group });
      else this.select({ kind: "part", id: part.id });
      return;
    }
    this.select(null);
  }

  select(s: Selection) {
    for (const [m, mat] of this.tinted) {
      for (const owned of Array.isArray(m.material) ? m.material : [m.material])
        owned.dispose();
      m.material = mat;
    }
    this.tinted.clear();
    this.bindings.onSelection(s);
    const d = this.bindings.document();
    const v = s
      ? (s.kind === "group" ? this.groupViews : this.partViews).get(s.id)
      : null;
    if (this.gizmo) {
      if (v) this.gizmo.attach(v.wrapper);
      else this.gizmo.detach();
    }
    if (s && d) {
      const parts =
        s.kind === "group"
          ? groupParts(d, s.id)
          : d.objects.filter((o) => o.id === s.id);
      for (const p of parts) {
        for (const m of this.partViews.get(p.id)?.meshes ?? []) {
          this.tinted.set(m, m.material);
          const tint = (original: THREE.Material) => {
            const mat = original.clone() as THREE.MeshBasicMaterial;
            if (mat.color) mat.color.set(0xffd27a);
            return mat;
          };
          m.material = Array.isArray(m.material)
            ? m.material.map(tint)
            : tint(m.material);
        }
      }
    }
    this.refreshSelectionBox();
  }

  private refreshSelectionBox() {
    const v = this.selectedView();
    if (!v) {
      this.selectionBox.visible = false;
      return;
    }
    v.wrapper.updateWorldMatrix(true, true);
    this.selectionBox.box.setFromObject(v.wrapper, true);
    this.selectionBox.visible = true;
  }

  buildOverlays() {
    disposeObjectResources([this.overlayRoot]);
    this.overlayRoot.clear();
    const d = this.bindings.document();
    if (!d) return;
    if (this.bindings.showObstacles()) {
      const pts: number[] = [];
      const hiddenGroups = new Set(
        d.groups.filter((g) => g.hidden).map((g) => g.id),
      );
      for (const o of d.objects) {
        if (o.hidden || (o.group && hiddenGroups.has(o.group))) continue;
        const ob = transformedObstacle(d, o);
        const n = ob.points.length;
        for (let i = 0; i < n; i++) {
          const a = ob.points[i]!;
          const b = ob.points[(i + 1) % n]!;
          const segs: [Vec3, Vec3][] = [
            [
              gameToScene(d.camera, a.x, a.y, a.z_top),
              gameToScene(d.camera, b.x, b.y, b.z_top),
            ],
            [
              gameToScene(d.camera, a.x, a.y, a.z_bottom),
              gameToScene(d.camera, a.x, a.y, a.z_top),
            ],
          ];
          for (const [p, q] of segs)
            pts.push(p[0], p[1], p[2], q[0], q[1], q[2]);
        }
      }
      const geo = new THREE.BufferGeometry();
      geo.setAttribute("position", new THREE.Float32BufferAttribute(pts, 3));
      this.overlayRoot.add(
        new THREE.LineSegments(
          geo,
          new THREE.LineBasicMaterial({
            color: 0x40e0ff,
            depthTest: false,
            transparent: true,
            opacity: 0.6,
          }),
        ),
      );
    }
    const lvl = this.bindings.level();
    if (this.bindings.showElevation() && lvl) {
      const pts: number[] = [];
      for (const e of lvl.elevation_lines) {
        const p = gameToScene(d.camera, e.point_a[0], e.point_a[1], 0);
        const q = gameToScene(d.camera, e.point_b[0], e.point_b[1], 0);
        pts.push(p[0], p[1], p[2] + 1, q[0], q[1], q[2] + 1);
      }
      const geo = new THREE.BufferGeometry();
      geo.setAttribute("position", new THREE.Float32BufferAttribute(pts, 3));
      this.overlayRoot.add(
        new THREE.LineSegments(
          geo,
          new THREE.LineBasicMaterial({
            color: 0xff70d0,
            depthTest: false,
            transparent: true,
            opacity: 0.8,
          }),
        ),
      );
    }
  }
}
