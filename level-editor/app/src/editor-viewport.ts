import { TerrainControls, type TerrainEditMode } from "./terrain-controls.ts";
import { MissionLayer } from "./mission-layer.ts";
import { CHARACTER_DRAG_TYPE } from "./mission-character-catalog.ts";
import { TerrainLayer } from "./terrain-layer.ts";
import { followTerrainEdit, followTerrainTransform } from "./terrain-follow.ts";
import { AssetOutlineRenderer } from "./asset-outline.ts";
import { stableOpaqueSort } from "./render-order.ts";
import {
  bakeScene,
  contentBakeBounds,
  renderMapBake,
  renderMapBakeAsync,
  yieldBakeFrame,
  type BakeProgress,
  maskOcclusionObjects,
} from "./map-bake-render.ts";
import { compileMap, type BakeBounds, type CompiledMap } from "./map-compile.ts";
import {
  bindBakeAppearances,
  planAppearanceRegions,
  bakeAppearanceRegions,
  bakeAppearanceRegionsAsync,
} from "./map-appearance-bake.ts";
import { SunLighting } from "./sun-lighting.ts";
import { SplineLayer, type SplineEditMode } from "./spline-layer.ts";
import type { ExternalAssetSource } from "@rle/shared";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { TransformControls } from "three/examples/jsm/controls/TransformControls.js";
import {
  sceneToGame,
  sceneToMap,
  groundToScene,
  gameToScene,
  gameTransformMatrix,
  groupCentroid,
  groupParts,
  partPivot,
  transformedObstacle,
  type GameTransform,
  type Level3D,
  type Level3DObject,
  type ProtoLevel,
  type Vec3,
} from "@rle/shared";
import type { SceneEntities } from "./population-view.ts";
import type { Selection } from "./document-commands.ts";
import { disposeObjectResources } from "./resources.ts";
import { TextureDisplay } from "./texture-display.ts";
import { PatchDisplay, applyPlacementPatches } from "./patch-display.ts";
import { setViewportRay, visibleSurface } from "./viewport-picking.ts";

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
  onSelection(selection: Selection, revealInList: boolean): void;
  onError?(message: string): void;
  commitTransform(transform: GameTransform): void;
}

/** Owns the entire scene projection and its browser/GPU lifetime. Document and
 * selection are borrowed from the session/UI, never copied into another model.
 * Editable clones share source resources; only source roots own their disposal. */
export class EditorViewport {
  private prepareMapBake(document: Level3D) {
    if (this.disposed || this.bindings.document() !== document)
      throw new Error("The map changed before compilation started. Export the current map again.");
    // Reapply committed transforms so an in-progress numeric preview cannot leak into export.
    this.syncViews(document);
    const root = bakeScene([
      this.objectsRoot,
      ...(this.ground ? [this.ground] : []),
      ...this.splines.bakeObjects(),
      this.terrain.root,
    ]);
    const bounds =
      document.exportBounds ??
      (document.size
        ? ([0, 0, ...document.size] as [number, number, number, number])
        : (() => {
            const bounds = contentBakeBounds(root, document.camera, true);
            return [bounds[0], bounds[1], bounds[2] + 1, bounds[3] + 1] as typeof bounds;
          })());
    return { root, bounds };
  }
  bakeMap(
    document: Level3D,
    assets?: ReadonlyMap<string, import("@rle/shared").ProjectionAssetDescriptor>,
  ) {
    const { root, bounds } = this.prepareMapBake(document);
    const compiled = compileMap(document, bounds, assets, { bestEffort: true });
    const transitions = compiled.descriptor.asset_geometry?.movement_transitions ?? [];
    bindBakeAppearances(root, document, assets ?? new Map(), transitions, (message) => {
      if (!compiled.warnings.includes(message)) compiled.warnings.push(message);
    });
    const plans = planAppearanceRegions(
      root,
      document.camera,
      compiled.bounds,
      transitions,
      !!document.lighting?.enabled,
    );
    const render = () =>
      renderMapBake(
        root,
        document.camera,
        compiled.bounds,
        document.lighting,
        this.ground,
        maskOcclusionObjects(document, assets),
      );
    const pixels = render();
    const appearance = bakeAppearanceRegions(root, plans, compiled.bounds[2], pixels, render);
    return { compiled, pixels, appearance };
  }
  async bakeMapAsync(
    document: Level3D,
    assets?: ReadonlyMap<string, import("@rle/shared").ProjectionAssetDescriptor>,
    progress: (progress: BakeProgress) => void = () => {},
    compiler?: (bounds: BakeBounds) => Promise<CompiledMap>,
  ) {
    const checkCurrent = () => {
      if (this.disposed || this.bindings.document() !== document)
        throw new Error("The map changed during compilation. Export the current map again.");
    };
    progress({ stage: "Preparing map geometry", completed: 0, total: 0 });
    await yieldBakeFrame();
    checkCurrent();
    const { root, bounds } = this.prepareMapBake(document);
    progress({ stage: "Compiling gameplay", completed: 0, total: 0 });
    await yieldBakeFrame();
    checkCurrent();
    const compiled = compiler
      ? await compiler(bounds)
      : compileMap(document, bounds, assets, { bestEffort: true });
    checkCurrent();
    const transitions = compiled.descriptor.asset_geometry?.movement_transitions ?? [];
    bindBakeAppearances(root, document, assets ?? new Map(), transitions, (message) => {
      if (!compiled.warnings.includes(message)) compiled.warnings.push(message);
    });
    const plans = planAppearanceRegions(
      root,
      document.camera,
      compiled.bounds,
      transitions,
      !!document.lighting?.enabled,
    );
    const total = 1 + plans.reduce((sum, plan) => sum + 2 ** plan.patches.length - 1, 0);
    const excluded = maskOcclusionObjects(document, assets);
    let completed = 0;
    const render = async () => {
      const pixels = await renderMapBakeAsync(
        root,
        document.camera,
        compiled.bounds,
        document.lighting,
        this.ground,
        excluded,
        (tile) =>
          progress({
            stage: `${tile.stage}${completed ? ` (appearance ${completed}/${total - 1})` : ""}`,
            completed: completed + tile.completed / tile.total,
            total,
          }),
        checkCurrent,
      );
      completed++;
      return pixels;
    };
    const pixels = await render();
    const appearance = await bakeAppearanceRegionsAsync(
      root,
      plans,
      compiled.bounds[2],
      pixels,
      render,
    );
    checkCurrent();
    return { compiled, pixels, appearance };
  }
  private readonly patchDisplay = new PatchDisplay();
  setPatchRevealed(patch: string, revealed: boolean) {
    this.patchDisplay.set(patch, revealed);
    this.patchDisplay.apply(this.objectsRoot);
  }
  patchPreviews(selection?: Selection) {
    const patches = new Set<string>();
    const labels = new Map<string, string>();
    this.sourceAsset?.traverse((object) => {
      for (const patch of object.userData.reveal?.patches ?? []) labels.set(patch.id, patch.name);
    });
    this.objectsRoot.traverse((object) => {
      for (const patch of object.userData.reveal?.patches ?? []) labels.set(patch.id, patch.name);
    });
    const root =
      selection === undefined
        ? this.objectsRoot
        : selection
          ? (selection.kind === "group" ? this.groupViews : this.partViews).get(selection.id)
              ?.wrapper
          : null;
    root?.traverse((object) => {
      const id = object.userData.reveal_material_patch;
      if (typeof id === "string") patches.add(id);
      for (const key of ["reveal_hide_when_applied", "reveal_show_when_applied"])
        for (const trigger of object.userData[key] ?? [])
          if (typeof trigger === "string") patches.add(trigger);
      for (const patch of object.userData.reveal?.patches ?? []) labels.set(patch.id, patch.name);
    });
    return [...patches].map((id) => ({
      id,
      name: labels.get(id) ?? id,
      revealed: this.patchDisplay.isRevealed(id),
    }));
  }
  private readonly textureDisplay = new TextureDisplay();
  setTextureDisplay(smooth: boolean, synthesized: boolean) {
    this.textureDisplay.smooth = smooth;
    this.textureDisplay.synthesized.value = synthesized;
    this.refreshTextureDisplay();
  }
  private refreshTextureDisplay(root?: THREE.Object3D) {
    const anisotropy = this.renderer?.capabilities.getMaxAnisotropy() ?? 1;
    if (root) {
      this.textureDisplay.apply(root, anisotropy);
      return;
    }
    for (const sceneRoot of [this.sourceAsset, this.ground, this.objectsRoot])
      if (sceneRoot) this.textureDisplay.apply(sceneRoot, anisotropy);
  }
  readonly listeners = new AbortController();
  private renderer: THREE.WebGLRenderer | null = null;
  private container: HTMLDivElement | null = null;
  private camera: THREE.OrthographicCamera | null = null;
  private frustum = 1500;
  private perspective = 0;
  private rotationSnap = false;
  private spriteOrientationLock = true;
  private readonly projectionBounds = new THREE.Sphere(new THREE.Vector3(), 10000);
  private readonly framingBounds = new THREE.Box3();
  private readonly clippingBounds = new THREE.Box3();
  private clippingBoundsDirty = true;
  private framingPoints: THREE.Vector3[] = [];
  private framingKey = "";
  private framingDistance = 0;
  private readonly perspectiveCamera = new THREE.PerspectiveCamera(45, 1, 0.1, 200000);
  private entities: SceneEntities | null = null;

  setPerspective(value: number) {
    this.perspective = THREE.MathUtils.clamp(value, 0, 65);
    if (this.camera) this.activeCamera();
  }
  setRotationSnap(enabled: boolean) {
    this.rotationSnap = enabled;
    if (this.camera) this.activeCamera();
  }
  /** Face a compass direction while retaining the current working location and scale. */
  setCardinalView(direction: "N" | "E" | "S" | "W") {
    this.orientCamera({ N: 0, E: -Math.PI / 2, S: Math.PI, W: Math.PI / 2 }[direction]);
  }
  rotateViewQuarterTurn(turns = 1) {
    this.orientCamera(this.cameraAzimuth() + (turns * Math.PI) / 2);
  }
  topView() {
    this.orientCamera(this.cameraAzimuth(), true);
  }
  private cameraAzimuth() {
    if (!this.camera) return 0;
    // Screen-right remains well defined even when looking vertically down.
    const right = new THREE.Vector3(1, 0, 0).applyQuaternion(this.camera.quaternion);
    return Math.atan2(-right.z, right.x);
  }
  private orientCamera(azimuth: number, top = false) {
    if (!this.camera || !this.orbit || !Number.isFinite(azimuth)) return;
    const offset = this.camera.position.clone().sub(this.orbit.target);
    const radius = Math.max(1, offset.length());
    // OrbitControls uses this same tiny pole margin to keep top-view yaw defined.
    const polar = top
      ? 1e-6
      : Math.max(1e-6, Math.acos(THREE.MathUtils.clamp(offset.y / radius, -1, 1)));
    const position = this.orbit.target
      .clone()
      .add(new THREE.Vector3().setFromSphericalCoords(radius, polar, azimuth));
    const destination = this.lookState(position, this.orbit.target, this.frustum);
    destination.zoom = this.camera.zoom;
    this.flyTo(destination);
  }
  private assetDisplayMode: "visible" | "outline" | "hidden" = "visible";
  private readonly assetOutline = new AssetOutlineRenderer();
  setAssetDisplayMode(mode: "visible" | "outline" | "hidden") {
    this.assetDisplayMode = mode;
    if (this.renderer) this.renderer.shadowMap.needsUpdate = true;
    if (mode === "hidden") this.select(null);
  }
  setSpriteOrientationLock(enabled: boolean) {
    this.spriteOrientationLock = enabled;
  }
  replaceEntities(entities: SceneEntities | null) {
    this.entities?.dispose();
    this.entities = entities;
    if (entities) this.scene.add(entities.root);
  }
  setPopulationPlaying(value: boolean) {
    this.entities?.setPlaying?.(value);
  }
  setPopulationRoutesVisible(value: boolean) {
    this.entities?.setRoutesVisible?.(value);
  }
  setEntitiesVisible(visible: boolean) {
    if (this.entities) this.entities.root.visible = visible;
  }
  private updateDepthRange(camera: THREE.OrthographicCamera | THREE.PerspectiveCamera) {
    if (this.clippingBoundsDirty) {
      this.clippingBounds.copy(this.contentBox());
      this.clippingBoundsDirty = false;
    }
    // Lens framing intentionally stays stable while editing. Clipping must follow
    // the live geometry, including previews and content retained outside bounds.
    const bounds = this.clippingBounds.isEmpty() ? this.framingBounds : this.clippingBounds;
    const forward = camera.getWorldDirection(new THREE.Vector3());
    let nearest = Infinity;
    let farthest = -Infinity;
    if (!bounds.isEmpty()) {
      for (const x of [bounds.min.x, bounds.max.x])
        for (const y of [bounds.min.y, bounds.max.y])
          for (const z of [bounds.min.z, bounds.max.z]) {
            const depth = new THREE.Vector3(x, y, z).sub(camera.position).dot(forward);
            nearest = Math.min(nearest, depth);
            farthest = Math.max(farthest, depth);
          }
      // Include sprites standing above the map and small editing overlays.
      nearest -= 128;
      farthest += 128;
    } else {
      const center = this.projectionBounds.center.clone().sub(camera.position).dot(forward);
      const radius = Math.max(10, this.projectionBounds.radius * 1.1);
      nearest = center - radius;
      farthest = center + radius;
    }
    camera.near = camera instanceof THREE.PerspectiveCamera ? Math.max(0.1, nearest) : nearest;
    camera.far = Math.max(camera.near + 1, farthest);
    camera.updateProjectionMatrix();
  }
  /** Keep the map's average projected scale while introducing foreshortening with a
   * virtual lens. Controls retain their map-unit pan and zoom. */
  private activeCamera(): THREE.OrthographicCamera | THREE.PerspectiveCamera {
    const camera = this.camera!;
    if (!camera) throw new Error("Viewport camera is not mounted");
    if (this.rotationSnap) {
      const back = new THREE.Vector3(0, 0, 1).applyQuaternion(camera.quaternion);
      const azimuth = Math.atan2(back.x, back.z);
      const step = Math.PI / 8;
      const delta = Math.round(azimuth / step) * step - azimuth;
      if (Math.abs(delta) > 1e-10) {
        const rotation = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), delta);
        camera.position.sub(this.orbit!.target).applyQuaternion(rotation).add(this.orbit!.target);
        camera.quaternion.premultiply(rotation);
      }
    }
    this.orbit!.screenSpacePanning = this.perspective === 0;
    if (this.perspective === 0) {
      if (this.gizmo) this.gizmo.camera = camera;
      this.updateDepthRange(camera);
      camera.updateMatrixWorld();
      return camera;
    }
    const lens = this.perspectiveCamera;
    const target = this.orbit!.target;
    const forward = camera.getWorldDirection(new THREE.Vector3());
    const halfHeight = this.frustum / camera.zoom;
    const aspect = this.container!.clientWidth / Math.max(1, this.container!.clientHeight);
    let distance = halfHeight / Math.tan(THREE.MathUtils.degToRad(this.perspective / 2));
    const targetDepth = target.clone().sub(camera.position).dot(forward);
    // Preserve the map's average projected scale, not just the target plane. Nearby
    // walls otherwise grow dramatically as the lens moves closer at wide angles.
    // Pan and orbit are rigid camera movements. Refit only when lens/view
    // dimensions change, otherwise rotating toward a wider map silhouette
    // silently dollies the camera and moves the point under the cursor.
    // Store the fitted distance at unit zoom: wheel zoom must dolly freely,
    // rather than refitting the entire map and converging on its front surface.
    const framingKey = [this.frustum, aspect, this.perspective].join(",");
    if (framingKey === this.framingKey) {
      distance = this.framingDistance / camera.zoom;
    } else if (!this.framingBounds.isEmpty()) {
      const inverse = camera.quaternion.clone().invert();
      const samples: { weight: number; depth: number }[] = [];
      let totalWeight = 0;
      let nearestDepth = -Infinity;
      const points = this.framingPoints.length
        ? this.framingPoints
        : [this.framingBounds.min.x, this.framingBounds.max.x].flatMap((x) =>
            [this.framingBounds.min.y, this.framingBounds.max.y].flatMap((y) =>
              [this.framingBounds.min.z, this.framingBounds.max.z].map(
                (z) => new THREE.Vector3(x, y, z),
              ),
            ),
          );
      for (const source of points) {
        const point = source.clone().sub(camera.position).applyQuaternion(inverse);
        const weight = (point.x / aspect) ** 2 + point.y ** 2;
        if (weight === 0) continue;
        const depth = point.z + targetDepth;
        samples.push({ weight, depth });
        totalWeight += weight;
        nearestDepth = Math.max(nearestDepth, depth);
      }
      if (totalWeight > 0) {
        const baseDistance = distance;
        // Match RMS screen radius. The hard maximum over vertices causes a
        // visible kink whenever a different silhouette point controls the fit.
        // This monotonic equation instead varies smoothly with the lens angle.
        let low = nearestDepth;
        let high = nearestDepth + baseDistance;
        for (let iteration = 0; iteration < 40; iteration++) {
          const candidate = (low + high) / 2;
          let weight = 0;
          for (const sample of samples)
            weight += sample.weight * (baseDistance / (candidate - sample.depth)) ** 2;
          if (weight > totalWeight) low = candidate;
          else high = candidate;
        }
        distance = (low + high) / 2;
      }
    }
    this.framingKey = framingKey;
    this.framingDistance = distance * camera.zoom;
    lens.position.copy(camera.position).addScaledVector(forward, targetDepth - distance);
    lens.quaternion.copy(camera.quaternion);
    lens.fov = this.perspective;
    lens.aspect = aspect;
    // A distant, narrow-angle lens needs a distant near plane too: retaining
    // a 0.1 near plane loses depth precision between adjacent map surfaces.
    this.updateDepthRange(lens);
    lens.updateMatrixWorld();
    if (this.gizmo) this.gizmo.camera = lens;
    return lens;
  }
  private orbit: OrbitControls | null = null;
  private gizmo: TransformControls | null = null;
  // A separate frame rotates translation axes without changing asset orientation.
  private readonly gizmoFrame = new THREE.Object3D();
  private coordinateRotation = 45;
  private readonly scene = new THREE.Scene();
  private readonly mapRoot = new THREE.Group();
  private readonly objectsRoot = new THREE.Group();
  private readonly overlayRoot = new THREE.Group();
  private readonly terrain = new TerrainLayer();
  private readonly terrainControls = new TerrainControls((region) => this.previewTerrain(region));
  private terrainMode: TerrainEditMode | null = null;
  private terrainSelectionHandler: ((id: string) => void) | null = null;
  setTerrainSelectionHandler(handler: ((id: string) => void) | null) {
    this.terrainSelectionHandler = handler;
  }
  private terrainPreview: import("@rle/shared").TerrainGrid | null = null;
  private gizmoVertical = false;
  setTerrainEdit(mode: TerrainEditMode | null) {
    if (!mode && !this.terrainMode) {
      this.terrainControls.setMode(null);
      return;
    }
    if (mode && this.splineMode) this.setSplineEdit(null);
    if (this.terrainPreview) this.previewTerrain(null);
    this.terrainControls.setMode(mode);
    if (mode && !this.terrainMode) this.select(null);
    this.terrainMode = mode;
    this.terrainPreview = null;
    if (this.gizmo) this.gizmo.showY = this.gizmoVertical;
    this.syncSelection(mode ? null : this.bindings.selection());
  }
  previewTerrain(grid: import("@rle/shared").TerrainGrid | null) {
    if (this.disposed) return;
    const document = this.bindings.document();
    if (!document) return;
    this.terrainPreview = grid;
    this.terrainControls.show(grid ?? this.terrainMode?.grid ?? null);
    this.syncViews(
      grid ? followTerrainEdit(document, { ...document, terrain: grid }) : document,
      false,
    );
  }
  private readonly splines = new SplineLayer();
  private readonly missionMarkers = new MissionLayer();
  private cancelMissionDrag: (() => void) | null = null;
  private missionPaletteDrag: { key: string; id: string } | null = null;
  private missionEdit: {
    selected: string;
    select(id: string): void;
    preview(position: Vec3): void;
    move(position: Vec3): void;
    cancel(): void;
    add(key: string, position: Vec3, id: string): void;
    previewAdd(key: string, position: Vec3, id: string): void;
  } | null = null;

  setMissionEdit(mode: typeof this.missionEdit) {
    if (!mode) {
      this.cancelMissionDrag?.();
      this.endMissionPaletteDrag();
    }
    this.missionEdit = mode;
    if (mode) this.select(null);
    const document = this.bindings.document();
    if (document) this.missionMarkers.sync(document, mode?.selected);
  }
  setMissionVisible(visible: boolean) {
    if (!visible) {
      this.cancelMissionDrag?.();
      this.endMissionPaletteDrag();
    }
    this.missionMarkers.setVisible(visible);
  }
  startMissionPaletteDrag(key: string) {
    this.endMissionPaletteDrag();
    if (this.missionEdit) this.missionPaletteDrag = { key, id: `character-${crypto.randomUUID()}` };
  }
  private hideMissionPalettePreview() {
    const document = this.bindings.document();
    if (document) this.missionMarkers.sync(document, this.missionEdit?.selected);
  }
  endMissionPaletteDrag() {
    if (!this.missionPaletteDrag) return;
    this.missionPaletteDrag = null;
    this.hideMissionPalettePreview();
  }
  setMissionSpriteLibrary(
    root: FileSystemDirectoryHandle | null,
    profiles: readonly import("./mission-character-catalog.ts").MissionCharacterProfile[],
    onStatus: (loading: boolean, warnings: string[]) => void,
  ) {
    this.missionMarkers.setLibrary(root, profiles, onStatus);
  }
  private readonly sunlight = new SunLighting();
  private splineMode: SplineEditMode | null = null;
  private readonly partViews = new Map<string, View>();
  private readonly groupViews = new Map<string, View>();
  private readonly sourceNodes = new Map<string, THREE.Object3D>();
  private readonly raycaster = new THREE.Raycaster();
  private readonly selectionBox = new THREE.Box3Helper(new THREE.Box3(), 0xffcc40);
  private readonly tinted = new Map<THREE.Mesh, THREE.Material | THREE.Material[]>();
  private dragging = false;
  private cancelPointerGesture: (() => void) | null = null;
  private flight: {
    from: CameraState;
    to: CameraState;
    start: number;
    ms: number;
  } | null = null;

  private readonly exportFrame = new THREE.LineLoop(
    new THREE.BufferGeometry(),
    new THREE.LineDashedMaterial({ color: 0xe2cb8e, dashSize: 32, gapSize: 16, depthTest: false }),
  );
  private readonly workspaceFrame = new THREE.LineLoop(
    new THREE.BufferGeometry(),
    new THREE.LineBasicMaterial({ color: 0x91cfa0, depthTest: false, depthWrite: false }),
  );
  private readonly workspaceGrid = new THREE.GridHelper(10000, 100, 0x52655a, 0x34423b);
  private readonly bindings: ViewportBindings;
  constructor(bindings: ViewportBindings) {
    this.bindings = bindings;
    this.scene.background = new THREE.Color(0x1c1c1c);
    this.mapRoot.quaternion.set(-Math.SQRT1_2, 0, 0, Math.SQRT1_2);
    this.scene.add(this.mapRoot);
    this.workspaceGrid.visible = false;
    this.scene.add(this.workspaceGrid);
    this.exportFrame.visible = false;
    this.exportFrame.renderOrder = 1000;
    this.mapRoot.add(this.exportFrame);
    this.workspaceFrame.visible = false;
    this.workspaceFrame.renderOrder = 999;
    this.mapRoot.add(this.workspaceFrame);
    this.mapRoot.add(this.missionMarkers.root);
    this.scene.add(this.missionMarkers.spritesRoot);
    this.missionMarkers.setVisible(true);
    this.mapRoot.add(
      this.terrain.root,
      this.terrainControls.root,
      this.objectsRoot,
      this.overlayRoot,
      this.splines.root,
      this.sunlight.root,
    );
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
  setCoordinateRotation(degrees: number) {
    if (!Number.isFinite(degrees)) return;
    this.coordinateRotation = degrees;
    this.gizmoFrame.rotation.set(0, THREE.MathUtils.degToRad(degrees), 0);
  }
  private syncGizmoFrame(view = this.selectedView()) {
    // Direct object drags move the wrapper, so its gizmo must follow each frame.
    // Only freeze synchronization while TransformControls itself owns the drag.
    if (view && !this.gizmo?.dragging) view.wrapper.getWorldPosition(this.gizmoFrame.position);
  }
  setGizmoVertical(vertical: boolean) {
    this.gizmoVertical = vertical;
    if (this.gizmo) this.gizmo.showY = vertical;
  }

  private sourceAsset: THREE.Object3D | null = null;
  private readonly externalAssetHashes = new Map<string, string>();
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
    references: ExternalAssetSource[] = [],
  ) {
    if (this.disposed) throw new Error("Disposed viewport cannot adopt a map");
    this.retireMap();
    this.patchDisplay.clear();
    this.sourceAsset = asset;
    this.ground = ground;
    if (ground) this.mapRoot.add(ground);
    this.sunlight.setGround(ground);
    for (const [key, value] of sources) this.sourceNodes.set(key, value);
    for (const ref of references)
      this.externalAssetHashes.set(ref.id, ref.descriptor_sha256 + ref.model_sha256);
    this.refreshTextureDisplay();
  }

  /** Register immutable standalone geometry before document insertion.
   * Returns false when the caller should dispose a redundant prepared asset.
   */
  adoptAsset(
    reference: ExternalAssetSource,
    asset: THREE.Object3D,
    sources: ReadonlyMap<string, THREE.Object3D>,
    additionalReferences: readonly ExternalAssetSource[] = [],
  ): boolean {
    if (this.disposed || !this.sourceAsset) throw new Error("No active map for asset insertion");
    const references = [reference, ...additionalReferences];
    if (new Set(references.map((ref) => ref.id)).size !== references.length)
      throw new Error("Duplicate asset registration");
    const added = new Set<string>();
    for (const ref of references) {
      const hash = ref.descriptor_sha256 + ref.model_sha256;
      const existing = this.externalAssetHashes.get(ref.id);
      if (existing !== undefined && existing !== hash)
        throw new Error(
          "This asset changed during the editing session; reload the map before importing its new revision",
        );
      if (existing === undefined) added.add(ref.id);
    }
    if (!added.size) return false;
    for (const key of sources.keys()) {
      const owner = references.find((ref) => key.startsWith(`asset:${ref.id}:`));
      if (!owner) throw new Error(`Unregistered asset node: ${key}`);
      if (!added.has(owner.id) && !this.sourceNodes.has(key))
        throw new Error(`Missing reused asset node: ${key}`);
    }
    const fresh = [...sources].filter(([key]) =>
      [...added].some((id) => key.startsWith(`asset:${id}:`)),
    );
    for (const id of added)
      if (!fresh.some(([key]) => key.startsWith(`asset:${id}:`)))
        throw new Error(`Asset registration has no new model nodes: ${id}`);
    for (const [key] of fresh)
      if (this.sourceNodes.has(key)) throw new Error(`Asset node collision: ${key}`);
    this.sourceAsset.add(asset);
    for (const [key, node] of fresh) this.sourceNodes.set(key, node);
    for (const ref of references)
      this.externalAssetHashes.set(ref.id, ref.descriptor_sha256 + ref.model_sha256);
    this.refreshTextureDisplay(asset);
    return true;
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
  clearMap() {
    this.retireMap();
    this.patchDisplay.clear();
  }

  /** Capture synchronously so saving during later edits keeps the matching preview. */
  captureThumbnail(): HTMLCanvasElement {
    if (!this.renderer || !this.camera) throw new Error("Viewport is not ready");
    const canvas = document.createElement("canvas");
    canvas.width = 480;
    canvas.height = 300;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Cannot create thumbnail canvas");
    const helpers = [
      this.selectionBox,
      this.overlayRoot,
      this.splines.controls,
      this.terrainControls.root,
      this.exportFrame,
      this.workspaceFrame,
      this.workspaceGrid,
      this.gizmo?.getHelper(),
    ].filter((node) => node !== undefined);
    const visibility = helpers.map((node) => node.visible);
    const renderSize = this.renderer.getSize(new THREE.Vector2());
    const pixelRatio = this.renderer.getPixelRatio();
    const camera = this.activeCamera().clone();
    if (camera instanceof THREE.OrthographicCamera && this.framingPoints.length) {
      camera.updateMatrixWorld();
      const bounds = new THREE.Box2();
      const point = new THREE.Vector3();
      const projected = new THREE.Vector2();
      for (const source of this.framingPoints) {
        point.copy(source).applyMatrix4(camera.matrixWorldInverse);
        bounds.expandByPoint(projected.set(point.x, point.y));
      }
      const center = bounds.getCenter(new THREE.Vector2());
      const size = bounds.getSize(new THREE.Vector2());
      const halfHeight = Math.max(size.y, (size.x * canvas.height) / canvas.width, 1) * 0.53;
      const halfWidth = (halfHeight * canvas.width) / canvas.height;
      camera.left = center.x - halfWidth;
      camera.right = center.x + halfWidth;
      camera.top = center.y + halfHeight;
      camera.bottom = center.y - halfHeight;
      camera.zoom = 1;
      camera.updateProjectionMatrix();
    } else if (camera instanceof THREE.PerspectiveCamera) {
      camera.aspect = canvas.width / canvas.height;
      camera.updateProjectionMatrix();
    }
    const materials = [...this.tinted].map(([mesh, original]) => {
      const tinted = mesh.material;
      mesh.material = original;
      return { mesh, tinted };
    });
    try {
      for (const node of helpers) node.visible = false;
      this.renderer.shadowMap.needsUpdate = true;
      this.renderer.setPixelRatio(1);
      this.renderer.setSize(canvas.width, canvas.height, false);
      this.renderer.render(this.scene, camera);
      const source = this.renderer.domElement;
      const scale = Math.min(canvas.width / source.width, canvas.height / source.height);
      context.fillStyle = "#1c1c1c";
      context.fillRect(0, 0, canvas.width, canvas.height);
      context.drawImage(
        source,
        (canvas.width - source.width * scale) / 2,
        (canvas.height - source.height * scale) / 2,
        source.width * scale,
        source.height * scale,
      );
      return canvas;
    } finally {
      this.renderer.shadowMap.needsUpdate = true;
      this.renderer.setPixelRatio(pixelRatio);
      this.renderer.setSize(renderSize.x, renderSize.y, false);
      for (const { mesh, tinted } of materials) mesh.material = tinted;
      helpers.forEach((node, index) => {
        node.visible = visibility[index]!;
      });
    }
  }

  private retireMap() {
    this.endMissionPaletteDrag();
    this.cancelMissionDrag?.();
    this.missionEdit = null;
    this.missionMarkers.clear();
    this.splineMode = null;
    this.splineTerrainPreview = false;
    this.exportFrame.visible = false;
    this.workspaceFrame.visible = false;
    this.sunlight.setGround(null);
    this.sunlight.root.visible = false;
    this.splines.clear();
    this.setTerrainEdit(null);
    this.terrain.clear();
    this.cancelPointerGesture?.();
    this.replaceEntities(null);
    this.flight = null;
    this.select(null);
    // Retire borrowed material tints before releasing the old map, even if
    // reactive selection publication is pending or its owner is being disposed.
    this.syncSelection(null);
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
    this.externalAssetHashes.clear();
    this.framingPoints = [];
    this.framingBounds.makeEmpty();
    this.clippingBounds.makeEmpty();
    this.clippingBoundsDirty = true;
    this.framingKey = "";
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
    disposeObjectResources([
      this.selectionBox,
      this.workspaceGrid,
      this.exportFrame,
      this.workspaceFrame,
    ]);
    this.sunlight.dispose();
    this.terrainControls.dispose();
    this.assetOutline.dispose();
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
    if (this.disposed || this.renderer) throw new Error("Viewport can only mount once");
    this.container = el;
    this.renderer = new THREE.WebGLRenderer({ antialias: true, reversedDepthBuffer: true });
    this.renderer.setOpaqueSort(stableOpaqueSort);
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    this.renderer.shadowMap.autoUpdate = false;
    this.refreshTextureDisplay();
    el.appendChild(this.renderer.domElement);
    this.camera = new THREE.OrthographicCamera(-1, 1, 1, -1, -100000, 100000);
    const elevation = THREE.MathUtils.degToRad(
      this.bindings.document()?.camera.elevation_deg ?? 35,
    );
    this.camera.position.set(0, Math.sin(elevation), Math.cos(elevation)).multiplyScalar(3600);
    this.orbit = this.ownControl(new OrbitControls(this.camera, this.renderer.domElement));
    this.orbit.enableDamping = true;
    this.orbit.zoomToCursor = true;
    // left drag pans (or moves the selection, handler below), right drag
    // orbits around the point under the cursor (our own handler, OrbitControls
    // ignores the button), wheel zooms to the cursor
    this.orbit.mouseButtons = {
      LEFT: THREE.MOUSE.PAN,
      MIDDLE: THREE.MOUSE.DOLLY,
      RIGHT: null,
    };
    this.terrainControls.setup(
      this.renderer.domElement,
      (x, y) => {
        const rect = this.renderer!.domElement.getBoundingClientRect();
        this.scene.updateMatrixWorld(true);
        // Grid controls compare neighboring screen rays to size their handles.
        // Each sample must remain independent while the next one is computed.
        const ray = new THREE.Raycaster();
        ray.layers.mask = this.raycaster.layers.mask;
        ray.params = structuredClone(this.raycaster.params);
        setViewportRay(
          ray,
          new THREE.Vector2(
            ((x - rect.left) / rect.width) * 2 - 1,
            (-(y - rect.top) / rect.height) * 2 + 1,
          ),
          this.activeCamera(),
        );
        return ray;
      },
      () => {
        const enabled = this.orbit!.enabled;
        this.orbit!.enabled = false;
        return () => {
          if (this.orbit) this.orbit.enabled = enabled;
        };
      },
      this.listeners.signal,
      (point) => {
        const projected = point.clone().project(this.activeCamera());
        if (
          !Number.isFinite(projected.x) ||
          !Number.isFinite(projected.y) ||
          projected.z < -1 ||
          projected.z > 1
        )
          return null;
        const rect = this.renderer!.domElement.getBoundingClientRect();
        return {
          x: rect.left + ((projected.x + 1) * rect.width) / 2,
          y: rect.top + ((1 - projected.y) * rect.height) / 2,
        };
      },
    );
    this.setupMissionInteraction(this.renderer.domElement);
    this.setupCursorOrbit(this.renderer.domElement);
    this.setupSplineInteraction(this.renderer.domElement);
    this.gizmo = this.ownControl(new TransformControls(this.camera, this.renderer.domElement));
    this.gizmo.setMode("translate");
    this.gizmo.setSpace("local");
    this.setCoordinateRotation(this.coordinateRotation);
    this.scene.add(this.gizmoFrame);
    this.gizmo.showY = false;
    this.scene.add(this.gizmo.getHelper());
    this.gizmo.addEventListener("dragging-changed", (e) => {
      this.dragging = (e as unknown as { value: boolean }).value;
      if (this.orbit) this.orbit.enabled = !this.dragging;
      if (!this.dragging) this.commitGizmo();
    });
    this.gizmo.addEventListener("objectChange", () => {
      const view = this.selectedView();
      if (view?.wrapper.parent) {
        view.wrapper.position.copy(
          view.wrapper.parent.worldToLocal(this.gizmoFrame.position.clone()),
        );
        this.previewTerrainFollowing(view);
      }
      this.refreshSelectionBox();
      if (this.renderer) this.renderer.shadowMap.needsUpdate = true;
    });

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
        if (this.missionEdit) {
          this.assetDropPosition(e.clientX, e.clientY);
          const id = this.missionMarkers.hit(this.raycaster);
          if (id) this.missionEdit.select(id);
          return;
        }
        if (!this.splineMode) this.pick(e, e.altKey);
      },
      { signal: this.listeners.signal },
    );
    this.animate(() => {
      if (!this.renderer || !this.camera) return;
      if (this.flight) this.stepFlight();
      else this.orbit?.update();
      const camera = this.activeCamera();
      this.workspaceGrid.visible = this.bindings.document()?.size === null;
      if (this.workspaceGrid.visible && this.orbit) {
        const spacing =
          100 * 2 ** Math.floor(Math.log2(Math.max(1, this.frustum / this.camera.zoom) / 500));
        this.workspaceGrid.scale.setScalar(spacing / 100);
        this.workspaceGrid.position.set(
          Math.round(this.orbit.target.x / spacing) * spacing,
          -0.1,
          Math.round(this.orbit.target.z / spacing) * spacing,
        );
      }
      this.entities?.update(camera, this.spriteOrientationLock);
      this.missionMarkers.update(camera, this.spriteOrientationLock);
      if (this.assetDisplayMode === "outline") {
        this.assetOutline.render(this.renderer, this.scene, camera, [
          this.objectsRoot,
          ...this.splines.assetObjects(),
        ]);
      } else if (this.assetDisplayMode === "hidden") {
        const objects = [this.objectsRoot, ...this.splines.assetObjects()];
        const visible = objects.map((object) => object.visible);
        objects.forEach((object) => {
          object.visible = false;
        });
        try {
          this.renderer.render(this.scene, camera);
        } finally {
          objects.forEach((object, index) => {
            object.visible = visible[index]!;
          });
        }
      } else this.renderer.render(this.scene, camera);
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
    const from = this.currentState();
    // Consume pending control inertia without moving the visible start pose.
    // Interrupted flights must begin exactly where the previous frame left off.
    const damping = this.orbit.enableDamping;
    this.orbit.enableDamping = false;
    this.orbit.update();
    this.orbit.enableDamping = damping;
    this.camera.position.copy(from.position);
    this.camera.quaternion.copy(from.quaternion);
    this.orbit.target.copy(from.target);
    this.camera.zoom = from.zoom;
    this.camera.updateProjectionMatrix();
    this.camera.updateMatrixWorld();
    this.flight = {
      from,
      to,
      start: performance.now(),
      ms,
    };
    this.orbit.enabled = false;
  }

  private stepFlight() {
    if (!this.flight || !this.camera || !this.orbit) return;
    const raw = Math.min(1, (performance.now() - this.flight.start) / this.flight.ms);
    const t = raw < 0.5 ? 2 * raw * raw : 1 - Math.pow(-2 * raw + 2, 2) / 2; // ease in-out
    const { from, to } = this.flight;
    this.camera.quaternion.slerpQuaternions(from.quaternion, to.quaternion, t);
    this.orbit.target.lerpVectors(from.target, to.target, t);
    // Interpolate the orbit's orientation and radius, not a chord through the map.
    // Local +Z points back from the focus, keeping it centered throughout the turn.
    const radius = THREE.MathUtils.lerp(
      from.position.distanceTo(from.target),
      to.position.distanceTo(to.target),
      t,
    );
    this.camera.position
      .set(0, 0, radius)
      .applyQuaternion(this.camera.quaternion)
      .add(this.orbit.target);
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
    while (node && this.partViews.get(node.name)?.wrapper !== node) node = node.parent;
    return node
      ? (this.bindings.document()?.objects.find((o) => o.id === node.name) ?? null)
      : null;
  }

  private setupMissionInteraction(el: HTMLCanvasElement) {
    const dropPosition = (event: DragEvent) => {
      const ground = this.assetDropPosition(event.clientX, event.clientY);
      const document = this.bindings.document();
      const surface = this.raycaster.intersectObject(this.objectsRoot, true).find(visibleSurface);
      if (ground && document && surface) {
        const [x, y, z] = gameToScene(document.camera, ...ground);
        const distance = new THREE.Vector3(x, z, -y)
          .sub(this.raycaster.ray.origin)
          .dot(this.raycaster.ray.direction);
        if (distance < surface.distance) return ground;
      }
      return surface && document
        ? sceneToGame(document.camera, [surface.point.x, -surface.point.z, surface.point.y])
        : ground;
    };
    el.addEventListener(
      "dragleave",
      () => {
        if (this.missionPaletteDrag) this.hideMissionPalettePreview();
      },
      { signal: this.listeners.signal },
    );
    window.addEventListener(
      "keydown",
      (event) => {
        if (event.key === "Escape") this.endMissionPaletteDrag();
      },
      { signal: this.listeners.signal },
    );
    window.addEventListener("blur", () => this.endMissionPaletteDrag(), {
      signal: this.listeners.signal,
    });
    el.addEventListener(
      "dragover",
      (event) => {
        if (!this.missionEdit || !event.dataTransfer?.types.includes(CHARACTER_DRAG_TYPE)) return;
        event.preventDefault();
        event.stopPropagation();
        event.dataTransfer.dropEffect = "copy";
        const drag = this.missionPaletteDrag;
        const position = dropPosition(event);
        if (drag && position) this.missionEdit.previewAdd(drag.key, position, drag.id);
        else this.hideMissionPalettePreview();
      },
      { signal: this.listeners.signal },
    );
    el.addEventListener(
      "drop",
      (event) => {
        if (!this.missionEdit || !event.dataTransfer?.types.includes(CHARACTER_DRAG_TYPE)) return;
        event.preventDefault();
        event.stopPropagation();
        const position = dropPosition(event);
        const drag = this.missionPaletteDrag;
        if (position && drag && event.dataTransfer.getData(CHARACTER_DRAG_TYPE) === drag.key)
          this.missionEdit.add(drag.key, position, drag.id);
        this.endMissionPaletteDrag();
      },
      { signal: this.listeners.signal },
    );
    let drag: {
      pointer: number;
      x: number;
      y: number;
      plane: THREE.Plane;
      start: THREE.Vector3;
      origin: THREE.Vector3;
      position: Vec3 | null;
      mode: NonNullable<EditorViewport["missionEdit"]>;
    } | null = null;
    const finish = (commit: boolean) => {
      const current = drag;
      if (!current) return;
      drag = null;
      this.dragging = false;
      if (el.hasPointerCapture(current.pointer)) el.releasePointerCapture(current.pointer);
      if (this.orbit) this.orbit.enabled = true;
      if (commit && current.position) current.mode.move(current.position);
      else if (current.position) current.mode.cancel();
    };
    this.cancelMissionDrag = () => finish(false);
    el.addEventListener(
      "pointerdown",
      (event) => {
        const mode = this.missionEdit;
        const document = this.bindings.document();
        if (event.button !== 0 || !mode || !document || drag) return;
        this.assetDropPosition(event.clientX, event.clientY);
        const id = this.missionMarkers.hit(this.raycaster);
        const entry = [
          ...(document.mission?.spawnPoints ?? []),
          ...(document.mission?.soldiers ?? []),
        ].find((entry) => entry.id === id);
        if (!entry) return;
        const [x, y, z] = gameToScene(document.camera, ...entry.position);
        const origin = new THREE.Vector3(x, z, -y);
        const plane = new THREE.Plane(new THREE.Vector3(0, 1, 0), -z);
        const start = this.raycaster.ray.intersectPlane(plane, new THREE.Vector3());
        if (!start) return;
        mode.select(entry.id);
        drag = {
          pointer: event.pointerId,
          x: event.clientX,
          y: event.clientY,
          plane,
          start,
          origin,
          position: null,
          mode,
        };
        this.dragging = true;
        if (this.orbit) this.orbit.enabled = false;
        el.setPointerCapture(event.pointerId);
        event.preventDefault();
        event.stopImmediatePropagation();
      },
      { capture: true, signal: this.listeners.signal },
    );
    el.addEventListener(
      "pointermove",
      (event) => {
        if (!drag || event.pointerId !== drag.pointer) return;
        event.stopImmediatePropagation();
        if (!drag.position && Math.hypot(event.clientX - drag.x, event.clientY - drag.y) <= 4)
          return;
        const document = this.bindings.document();
        if (!document) return;
        this.assetDropPosition(event.clientX, event.clientY);
        const point = this.raycaster.ray.intersectPlane(drag.plane, new THREE.Vector3());
        if (!point) return;
        point.sub(drag.start).add(drag.origin);
        drag.position = sceneToGame(document.camera, [point.x, -point.z, point.y]);
        drag.mode.preview(drag.position);
      },
      { capture: true, signal: this.listeners.signal },
    );
    for (const name of ["pointerup", "pointercancel", "lostpointercapture"] as const)
      el.addEventListener(
        name,
        (event) => {
          if (!drag || event.pointerId !== drag.pointer) return;
          event.stopImmediatePropagation();
          finish(name === "pointerup");
        },
        { capture: true, signal: this.listeners.signal },
      );
    window.addEventListener(
      "keydown",
      (event) => {
        if (drag && event.key === "Escape") {
          event.preventDefault();
          finish(false);
        }
      },
      { signal: this.listeners.signal },
    );
    window.addEventListener("blur", () => finish(false), { signal: this.listeners.signal });
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
      azimuth: number;
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
      this.scene.updateMatrixWorld(true);
      setViewportRay(this.raycaster, ndc, this.activeCamera());
    };
    el.addEventListener(
      "pointerdown",
      (e) => {
        // the gizmo takes precedence when the cursor is on one of its handles
        if (
          ((this.missionEdit || this.terrainMode) && e.button === 0) ||
          !this.camera ||
          !this.orbit ||
          this.gizmo?.axis ||
          (e.button !== 0 && e.button !== 2)
        )
          return;
        setRay(e);
        const hits = this.raycaster
          .intersectObjects(
            [
              ...(this.assetDisplayMode === "hidden" ? [] : [this.objectsRoot]),
              this.terrain.root,
              ...(this.groundNode ? [this.groundNode] : []),
            ],
            true,
          )
          .filter(visibleSurface);
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
          right: new THREE.Vector3(1, 0, 0).applyQuaternion(this.camera.quaternion),
          polar: Math.acos(THREE.MathUtils.clamp(offset.normalize().y, -1, 1)),
          azimuth: Math.atan2(offset.x, offset.z),
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
          this.previewTerrainFollowing(moving.view);
          this.refreshSelectionBox();
          return;
        }
        if (!active || !this.camera || !this.orbit) return;
        const rect = el.getBoundingClientRect();
        let yaw = (-(e.clientX - active.startX) / rect.width) * Math.PI * 2;
        if (this.rotationSnap) {
          const step = Math.PI / 8;
          yaw = Math.round((active.azimuth + yaw) / step) * step - active.azimuth;
        }
        let pitch = (-(e.clientY - active.startY) / rect.height) * Math.PI;
        // keep the camera between straight down and just above the horizon
        pitch =
          THREE.MathUtils.clamp(active.polar + pitch, 0.02, Math.PI / 2 - 0.02) - active.polar;
        const q = new THREE.Quaternion()
          .setFromAxisAngle(up, yaw)
          .multiply(new THREE.Quaternion().setFromAxisAngle(active.right, pitch));
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
    const aspect = this.container.clientWidth / Math.max(1, this.container.clientHeight);
    this.camera.left = -this.frustum * aspect;
    this.camera.right = this.frustum * aspect;
    this.camera.top = this.frustum;
    this.camera.bottom = -this.frustum;
    this.camera.updateProjectionMatrix();
  }

  private contentBox(): THREE.Box3 {
    // The map's Z-up asset frame is rotated into the viewport's Y-up frame.
    // Box3 updates descendants, but needs the ancestor transform refreshed first.
    this.mapRoot.updateWorldMatrix(true, true);
    const box = new THREE.Box3();
    if (this.groundNode) box.expandByObject(this.groundNode);
    box.expandByObject(this.objectsRoot);
    box.expandByObject(this.splines.root);
    box.expandByObject(this.terrain.root);
    // Initial camera framing is a viewport preference, never an authored boundary.
    if (box.isEmpty() && this.bindings.document()?.size === null)
      box.set(new THREE.Vector3(-500, 0, -500), new THREE.Vector3(500, 0, 500));
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
      d?.size ? d.size[1] / 2 : size * 0.35,
    );
    if (instant) this.applyState(to);
    else this.flyTo(to);
  }

  private applyState(st: CameraState) {
    if (!this.camera || !this.orbit) return;
    this.framingKey = "";
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
    this.clippingBoundsDirty = true;
    v.wrapper.position.set(m[12]!, m[13]!, m[14]);
    const rest = new THREE.Matrix4().fromArray(m);
    rest.setPosition(0, 0, 0);
    v.rot.matrix.copy(rest);
    v.rot.matrixWorldNeedsUpdate = true;
  }

  fitExportBounds(): [number, number, number, number] {
    const document = this.bindings.document();
    if (!document) throw new Error("Open a map before setting an export frame");
    this.scene.updateMatrixWorld(true);
    const box = this.contentBox();
    if (box.isEmpty()) throw new Error("Add content before fitting an export frame");
    const projected = new THREE.Box2();
    for (const x of [box.min.x, box.max.x])
      for (const y of [box.min.y, box.max.y])
        for (const z of [box.min.z, box.max.z]) {
          const point = sceneToMap(document.camera, [x, -z, y]);
          projected.expandByPoint(new THREE.Vector2(...point));
        }
    const x = Math.floor(projected.min.x),
      y = Math.floor(projected.min.y);
    return [
      x,
      y,
      Math.max(1, Math.ceil(projected.max.x) - x + 1),
      Math.max(1, Math.ceil(projected.max.y) - y + 1),
    ];
  }

  syncViews(d: Level3D, rebuildFraming = true) {
    this.clippingBoundsDirty = true;
    this.missionMarkers.sync(d, this.missionEdit?.selected);
    this.terrain.sync(d);
    this.workspaceFrame.visible = !!d.size;
    if (d.size) {
      const [width, height] = d.size;
      this.workspaceFrame.geometry.dispose();
      this.workspaceFrame.geometry = new THREE.BufferGeometry().setFromPoints(
        [
          [0, 0],
          [width, 0],
          [width, height],
          [0, height],
        ].map(([x, y]) => new THREE.Vector3(...groundToScene(d.camera, x!, y!))),
      );
    }
    this.exportFrame.visible = !!d.exportBounds;
    if (d.exportBounds) {
      const [x, y, w, h] = d.exportBounds;
      this.exportFrame.geometry.dispose();
      this.exportFrame.geometry = new THREE.BufferGeometry().setFromPoints(
        [
          [x, y],
          [x + w, y],
          [x + w, y + h],
          [x, y + h],
        ].map(([px, py]) => new THREE.Vector3(...groundToScene(d.camera, px!, py!))),
      );
      this.exportFrame.computeLineDistances();
    }
    const syncSplines = () => this.splines.sync(d.splines ?? [], d.camera, this.sourceNodes, d);
    if (rebuildFraming) syncSplines();
    else this.updateSplinePreview(syncSplines);
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
        gameTransformMatrix(d.camera, g.transform, groupCentroid(groupParts(d, g.id))),
      );
      v.wrapper.visible = !g.hidden;
    }
    for (const [id, v] of this.groupViews) {
      if (aliveGroups.has(id)) continue;
      this.objectsRoot.remove(v.wrapper);
      this.groupViews.delete(id);
    }
    const aliveParts = new Set<string>();
    const availableNodes = new Set(this.sourceNodes.keys());
    for (const o of d.objects) {
      aliveParts.add(o.id);
      let v = this.partViews.get(o.id);
      if (!v) {
        const src = this.sourceNodes.get(o.node);
        if (!src) throw new Error(`Missing source node ${o.node} for ${o.id}`);
        v = this.makeView(o.id);
        v.wrapper.userData.map_bake_object_id = o.id;
        const node = src.clone(true);
        applyPlacementPatches(node, d, o, availableNodes);
        node.traverse((c) => {
          const m = c as THREE.Mesh;
          if (m.isMesh) v!.meshes.push(m);
        });
        v.rot.add(node);
        this.partViews.set(o.id, v);
      }
      const parent = (o.group ? this.groupViews.get(o.group)?.rot : undefined) ?? this.objectsRoot;
      if (v.wrapper.parent !== parent) parent.add(v.wrapper);
      this.setAffine(v, gameTransformMatrix(d.camera, o.transform, partPivot(o)));
      v.wrapper.visible = !o.hidden;
    }
    for (const [id, v] of this.partViews) {
      if (aliveParts.has(id)) continue;
      v.wrapper.parent?.remove(v.wrapper);
      this.partViews.delete(id);
    }
    if (!rebuildFraming) {
      // Transform edits only need to move existing wrappers. Rebuilding the
      // projection point cache below walks every vertex in every imported mesh;
      // doing that for each 15° button press makes a large map appear frozen.
      this.refreshSelectionBox();
      if (this.bindings.showObstacles()) this.buildOverlays(d);
      return;
    }
    this.patchDisplay.apply(this.objectsRoot);
    const s = this.bindings.selection();
    if (s && !(s.kind === "group" ? aliveGroups : aliveParts).has(s.id)) this.select(null);
    else this.refreshSelectionBox();
    const bounds = this.contentBox();
    this.refreshSunLighting(bounds);
    this.framingBounds.copy(bounds);
    this.framingPoints = [];
    this.framingKey = "";
    // Perspective extrema lie at triangle vertices. Empty bounding-box corners
    // must not influence lens compensation as the viewing angle changes.
    for (const root of [
      this.objectsRoot,
      this.terrain.root,
      this.splines.root,
      ...(this.groundNode ? [this.groundNode] : []),
    ]) {
      root.updateWorldMatrix(true, true);
      root.traverseVisible((node) => {
        if (!(node instanceof THREE.Mesh)) return;
        const positions = node.geometry.getAttribute("position");
        if (!positions) return;
        for (let i = 0; i < positions.count; i++)
          this.framingPoints.push(
            new THREE.Vector3().fromBufferAttribute(positions, i).applyMatrix4(node.matrixWorld),
          );
      });
    }
    if (!bounds.isEmpty()) bounds.getBoundingSphere(this.projectionBounds);
    if (this.bindings.showObstacles()) this.buildOverlays(d);
  }

  private selectedView(): View | null {
    const s = this.bindings.selection();
    if (!s) return null;
    return (s.kind === "group" ? this.groupViews : this.partViews).get(s.id) ?? null;
  }

  private viewTransform(): GameTransform | null {
    const d = this.bindings.document();
    const v = this.selectedView();
    const t = this.selectedGroup()?.transform ?? this.selectedPart()?.transform;
    if (!d || !v || !t) return null;
    const g = this.selectedGroup();
    const p = this.selectedPart();
    const pivot = g ? groupCentroid(groupParts(d, g.id)) : partPivot(p!);
    const base = gameTransformMatrix(d.camera, { ...t, dx: 0, dy: 0, dz: 0 }, pivot);
    const pos = v.wrapper.position;
    const [dx, dy, dz] = sceneToGame(d.camera, [
      pos.x - base[12]!,
      pos.y - base[13]!,
      pos.z - base[14]!,
    ]);
    return { ...t, dx, dy, dz };
  }

  private previewTerrainFollowing(view: View) {
    const document = this.bindings.document();
    const selection = this.bindings.selection();
    const transform = this.viewTransform();
    if (!document || !selection || !transform) return;
    const next = followTerrainTransform(document, selection, transform);
    if (next.dz === transform.dz) return;
    const pivot =
      selection.kind === "group"
        ? groupCentroid(groupParts(document, selection.id))
        : partPivot(this.selectedPart()!);
    this.setAffine(view, gameTransformMatrix(document.camera, next, pivot));
  }

  private commitGizmo() {
    const next = this.viewTransform();
    const previous = this.selectedGroup()?.transform ?? this.selectedPart()?.transform;
    if (!next || !previous) return;
    next.dx = Math.round(next.dx * 10) / 10;
    next.dy = Math.round(next.dy * 10) / 10;
    next.dz = Math.round(next.dz * 10) / 10;
    if (next.dx === previous.dx && next.dy === previous.dy && next.dz === previous.dz) return;
    this.bindings.commitTransform(next);
  }

  private refreshSunLighting(bounds = this.contentBox()) {
    const settings = this.bindings.document()?.lighting;
    this.sunlight.sync(settings, [this.objectsRoot, this.splines.root], bounds);
    if (this.renderer) {
      this.renderer.shadowMap.enabled = !!settings?.enabled;
      this.renderer.shadowMap.needsUpdate = true;
    }
  }

  private splinePreviewError: string | null = null;
  private splineTerrainPreview = false;
  private updateSplinePreview(update: () => void) {
    try {
      update();
      this.clippingBoundsDirty = true;
      this.splinePreviewError = null;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      // Pointer moves can retry the same invalid draft many times per second.
      if (message !== this.splinePreviewError) {
        this.splinePreviewError = message;
        const description = `Could not preview path: ${message}`;
        if (this.bindings.onError) this.bindings.onError(description);
        else console.error(description);
      }
    }
  }

  previewSpline(path: import("@rle/shared").LevelSpline | null) {
    this.updateSplinePreview(() => {
      const document = this.bindings.document();
      if (document && (path?.kind === "river" || this.splineTerrainPreview)) {
        const splines = path
          ? [...(document.splines ?? []).filter((spline) => spline.id !== path.id), path]
          : document.splines;
        this.syncViews(
          path ? followTerrainEdit(document, { ...document, splines }) : document,
          false,
        );
        this.splineTerrainPreview = path !== null;
      }
      if (path) this.splines.showPreview(path);
      else this.splines.setMode(this.splineMode);
      this.refreshSunLighting();
    });
  }

  setSplineEdit(mode: SplineEditMode | null) {
    if (this.splineTerrainPreview) this.previewSpline(null);
    if (mode && this.terrainMode) {
      this.terrainMode.deselect?.();
      this.setTerrainEdit(null);
    }
    if (mode && !this.splineMode) this.select(null);
    this.splineMode = mode;
    this.updateSplinePreview(() => {
      this.splines.setMode(mode);
      this.refreshSunLighting();
    });
  }

  private setupSplineInteraction(canvas: HTMLCanvasElement) {
    let gesture: {
      mode: SplineEditMode;
      index: number | null;
      point: Vec3;
      pointer: number;
      x: number;
      y: number;
      moved: boolean;
    } | null = null;
    const consume = (event: PointerEvent) => {
      event.preventDefault();
      event.stopImmediatePropagation();
    };
    canvas.addEventListener(
      "pointerdown",
      (event) => {
        const mode = this.splineMode;
        if (!mode || event.button !== 0 || gesture) return;
        const point = this.assetDropPosition(event.clientX, event.clientY);
        if (!point) return;
        const index = this.splines.hitHandle(this.raycaster);
        if (!mode.drawing && index === null) {
          const section = this.splines.hitSection(this.raycaster);
          if (section !== null) {
            consume(event);
            mode.selectSection?.(section);
          }
          return;
        }
        if (index !== null) point[2] = mode.path.points[index]![2];
        gesture = {
          mode,
          index,
          point,
          pointer: event.pointerId,
          x: event.clientX,
          y: event.clientY,
          moved: false,
        };
        // Empty-space gestures remain available to camera panning; only a click adds a point.
        if (index !== null) {
          consume(event);
          mode.selectPoint(index);
          canvas.setPointerCapture(event.pointerId);
          if (this.orbit) this.orbit.enabled = false;
        }
      },
      { capture: true, signal: this.listeners.signal },
    );
    canvas.addEventListener(
      "pointermove",
      (event) => {
        if (!gesture || gesture.pointer !== event.pointerId) return;
        gesture.moved ||= Math.hypot(event.clientX - gesture.x, event.clientY - gesture.y) > 4;
        if (gesture.index === null) return;
        consume(event);
        const point = this.assetDropPosition(event.clientX, event.clientY);
        if (!point) return;
        if (gesture.index !== null) point[2] = gesture.mode.path.points[gesture.index]![2];
        gesture.point = point;
        if (gesture.index !== null) {
          const points = gesture.mode.path.points.map((p, i) => (i === gesture!.index ? point : p));
          this.previewSpline({ ...gesture.mode.path, points });
        }
      },
      { capture: true, signal: this.listeners.signal },
    );
    const finish = (event: PointerEvent) => {
      if (!gesture || gesture.pointer !== event.pointerId) return;
      const active = gesture;
      gesture = null;
      active.moved ||= Math.hypot(event.clientX - active.x, event.clientY - active.y) > 4;
      if (active.index !== null) {
        consume(event);
        if (canvas.hasPointerCapture(event.pointerId))
          canvas.releasePointerCapture(event.pointerId);
        if (this.orbit) this.orbit.enabled = true;
      }
      this.updateSplinePreview(() => {
        if (event.type === "pointerup" && this.splineMode?.path.id === active.mode.path.id) {
          if (active.index === null) {
            if (!active.moved && this.splineMode.drawing) active.mode.append(active.point);
          } else active.mode.move(active.index, active.point);
        }
        this.previewSpline(null);
      });
    };
    canvas.addEventListener("pointerup", finish, { capture: true, signal: this.listeners.signal });
    canvas.addEventListener("pointercancel", finish, {
      capture: true,
      signal: this.listeners.signal,
    });
  }

  /** Locate the drop on visible terrain, falling back to the map ground plane. */
  assetDropPosition(clientX: number, clientY: number): Vec3 | null {
    const document = this.bindings.document();
    if (!document || !this.camera || !this.renderer) return null;
    const rect = this.renderer.domElement.getBoundingClientRect();
    const ndc = new THREE.Vector2(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      (-(clientY - rect.top) / rect.height) * 2 + 1,
    );
    this.scene.updateMatrixWorld(true);
    setViewportRay(this.raycaster, ndc, this.activeCamera());
    const terrainHit = this.raycaster
      .intersectObject(this.terrain.root, true)
      .find((hit) => hit.object.userData.terrainSurface && visibleSurface(hit));
    const hit =
      terrainHit ??
      (this.groundNode
        ? this.raycaster.intersectObject(this.groundNode, true).find(visibleSurface)
        : undefined);
    let point = hit?.point ?? null;
    if (!point) {
      const ray = this.raycaster.ray;
      if (Math.abs(ray.direction.y) < 1e-8) return null;
      const distance = -ray.origin.y / ray.direction.y;
      // Orthographic views use a signed near plane: lower-screen rays can
      // begin below the ground while the ground remains inside the view volume.
      if (distance < 0 && !(this.activeCamera() instanceof THREE.OrthographicCamera)) return null;
      point = ray.at(distance, new THREE.Vector3());
    }
    return sceneToGame(document.camera, [point.x, -point.z, point.y]);
  }

  private pick(e: PointerEvent, partOnly: boolean) {
    if (!this.camera || !this.renderer) return;
    const rect = this.renderer.domElement.getBoundingClientRect();
    const ndc = new THREE.Vector2(
      ((e.clientX - rect.left) / rect.width) * 2 - 1,
      -((e.clientY - rect.top) / rect.height) * 2 + 1,
    );
    this.scene.updateMatrixWorld(true);
    setViewportRay(this.raycaster, ndc, this.activeCamera());
    const hits =
      this.assetDisplayMode === "hidden" || this.terrainMode
        ? []
        : this.raycaster.intersectObject(this.objectsRoot, true).filter(visibleSurface);
    for (const h of hits) {
      const part = this.partOfHit(h);
      if (!part) continue;
      if (part.group && !partOnly) this.select({ kind: "group", id: part.group });
      else this.select({ kind: "part", id: part.id });
      return;
    }
    const groundHit = this.raycaster
      .intersectObject(this.terrain.root, true)
      .find((hit) => hit.object.userData.terrainSurface && visibleSurface(hit));
    if (groundHit && this.terrainSelectionHandler) {
      this.select(null);
      const cell =
        groundHit.object.userData.terrainCells?.[groundHit.faceIndex ?? -1] ??
        groundHit.object.userData.terrainCell;
      if (typeof cell === "string") this.terrainSelectionHandler(cell);
      return;
    }
    this.select(null);
  }

  /** Request a state change; the UI's selection effect owns visual publication. */
  select(s: Selection, revealInList = true) {
    this.bindings.onSelection(s, revealInList);
  }

  /** Project the published selection into all three visual representations together. */
  syncSelection(s: Selection) {
    if (s && this.terrainMode) {
      const mode = this.terrainMode;
      this.terrainMode = null;
      this.terrainControls.setMode(null);
      this.terrainPreview = null;
      if (this.gizmo) this.gizmo.showY = this.gizmoVertical;
      mode.deselect?.();
    }
    for (const [m, mat] of this.tinted) {
      for (const owned of Array.isArray(m.material) ? m.material : [m.material]) owned.dispose();
      m.material = mat;
    }
    this.tinted.clear();
    const d = this.bindings.document();
    const v = s ? (s.kind === "group" ? this.groupViews : this.partViews).get(s.id) : null;
    if (this.gizmo) {
      if (v) {
        this.syncGizmoFrame(v);
        this.gizmo.attach(this.gizmoFrame);
      } else this.gizmo.detach();
    }
    if (s && d) {
      const parts =
        s.kind === "group" ? groupParts(d, s.id) : d.objects.filter((o) => o.id === s.id);
      for (const p of parts) {
        for (const m of this.partViews.get(p.id)?.meshes ?? []) {
          this.tinted.set(m, m.material);
          const tint = (original: THREE.Material) => {
            const mat = original.clone() as THREE.MeshBasicMaterial;
            this.textureDisplay.material(mat);
            if (mat.color) mat.color.set(0xffd27a);
            return mat;
          };
          m.material = Array.isArray(m.material) ? m.material.map(tint) : tint(m.material);
        }
      }
    }
    this.refreshSelectionBox(v);
  }

  private refreshSelectionBox(v = this.selectedView()) {
    this.clippingBoundsDirty = true;
    this.syncGizmoFrame(v);
    if (!v) {
      this.selectionBox.visible = false;
      return;
    }
    v.wrapper.updateWorldMatrix(true, true);
    this.selectionBox.box.setFromObject(v.wrapper, true);
    this.selectionBox.visible = true;
  }

  buildOverlays(d = this.bindings.document()) {
    disposeObjectResources([this.overlayRoot]);
    this.overlayRoot.clear();
    if (!d) return;
    if (this.bindings.showObstacles()) {
      const pts: number[] = [];
      const hiddenGroups = new Set(d.groups.filter((g) => g.hidden).map((g) => g.id));
      for (const o of d.objects) {
        if (o.hidden || (o.group && hiddenGroups.has(o.group)) || o.kind === "scenery") continue;
        const ob = transformedObstacle(d, o);
        const n = ob.points.length;
        for (let i = 0; i < n; i++) {
          const a = ob.points[i]!;
          const b = ob.points[(i + 1) % n]!;
          const segs: [Vec3, Vec3][] = [
            [gameToScene(d.camera, a.x, a.y, a.z_top), gameToScene(d.camera, b.x, b.y, b.z_top)],
            [gameToScene(d.camera, a.x, a.y, a.z_bottom), gameToScene(d.camera, a.x, a.y, a.z_top)],
          ];
          for (const [p, q] of segs) pts.push(p[0], p[1], p[2], q[0], q[1], q[2]);
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
