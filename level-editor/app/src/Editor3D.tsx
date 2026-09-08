// The 3D level editor viewport and panels. Loads the volume reconstruction
// of a map (library/scenes/<map>-volumes.scene.glb, one node per obstacle)
// together with the game's level data, keeps a Level3D document (parts =
// obstacles with an editor transform, grouped into buildings), and lets
// you select, move, turn, duplicate, hide and delete buildings or single
// parts. Two cameras: the game's own (oblique orthographic, looking north)
// and a free orbit around the point under the cursor. Saves
// <map>.level3d.json next to the GLB; pipeline/src/bake.ts turns that back
// into game files.
import { For, Show, createEffect, createSignal, onCleanup } from "solid-js";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { TransformControls } from "three/examples/jsm/controls/TransformControls.js";
import {
  parseLevel3D,
  parseSceneDoc,
  documentProvenance,
  sceneToGame,
  IDENTITY_TRANSFORM,
  gameToScene,
  gameTransformMatrix,
  groupCentroid,
  groupObstacles,
  groupParts,
  isIdentity,
  obstacleCentroid,
  snapFloatingParts,
  transformedObstacle,
  type GameTransform,
  type Level3D,
  type Level3DGroup,
  type Level3DObject,
  type ProtoLevel,
  type SceneDoc,
  type Vec3,
} from "@rle/shared";
import { MapSession } from "./session";
import { disposeObjectResources } from "./resources";
import { listFiles, readJson, subdir, writeText } from "./fs";
import { loadProtoLevel, type DatadirIndex } from "./datadir";

/** Z-up scene frame -> glTF Y-up, as the GLB's root node applies it */
const ZUP_TO_YUP = new THREE.Quaternion(-Math.SQRT1_2, 0, 0, Math.SQRT1_2);

/** a transformable thing in the viewport: a translation wrapper (the gizmo's target) around an affine node */
interface View {
  wrapper: THREE.Group;
  rot: THREE.Group;
  meshes: THREE.Mesh[];
}

export type Selection = { kind: "group" | "part"; id: string } | null;

/** the library directory handle, wrapped because handles are async-iterable and Solid 2 would iterate them */
export interface LibraryRef {
  handle: FileSystemDirectoryHandle;
}

export interface EditorProps {
  index: () => DatadirIndex | null;
  library: () => LibraryRef | null;
  onError: (msg: string) => void;
  onStatus: (msg: string | null) => void;
}

const groupId = (root: number) => `group-${String(root).padStart(3, "0")}`;

export default function Editor3D(props: EditorProps) {
  const [maps, setMaps] = createSignal<string[]>([]);
  const [mapName, setMapName] = createSignal<string | null>(null);
  const [doc, setDoc] = createSignal<Level3D | null>(null);
  const [history, setHistory] = createSignal<{
    past: Level3D[];
    future: Level3D[];
  }>({ past: [], future: [] });
  const session = new MapSession<Level3D, FileSystemDirectoryHandle>();
  const [dirty, setDirty] = createSignal(false);
  let saving = false;
  let disposed = false;
  let sourceAsset: THREE.Object3D | null = null;
  let observer: ResizeObserver | null = null;
  let animationFrame = 0;
  const listeners = new AbortController();
  const [selected, setSelected] = createSignal<Selection>(null);
  const [filter, setFilter] = createSignal("");
  const [expanded, setExpanded] = createSignal<Set<string>>(new Set());
  const [showObstacles, setShowObstacles] = createSignal(false);
  const [showElevation, setShowElevation] = createSignal(false);
  const [gizmoVertical, setGizmoVertical] = createSignal(false);
  const [level, setLevel] = createSignal<ProtoLevel | null>(null);
  /** obstacle index -> suggested snap (Δ along the view ray, support obstacle) */
  const [suspects, setSuspects] = createSignal<
    Map<number, { delta: number; support: number }>
  >(new Map());
  const [info, setInfo] = createSignal<string | null>(null);

  // ── three.js ──
  let container!: HTMLDivElement;
  let renderer: THREE.WebGLRenderer | null = null;
  let camera: THREE.OrthographicCamera | null = null;
  let frustum = 1500;
  let orbit: OrbitControls | null = null;
  let gizmo: TransformControls | null = null;
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x1c1c1c);
  /** the GLB's root ("map", Z-up -> Y-up) */
  const mapRoot = new THREE.Group();
  mapRoot.quaternion.copy(ZUP_TO_YUP);
  scene.add(mapRoot);
  /** editable objects live here (Z-up frame) */
  const objectsRoot = new THREE.Group();
  mapRoot.add(objectsRoot);
  const overlayRoot = new THREE.Group();
  mapRoot.add(overlayRoot);
  const partViews = new Map<string, View>();
  const groupViews = new Map<string, View>();
  /** reconstruction nodes by name, kept out of the scene, cloned into views */
  const sourceNodes = new Map<string, THREE.Object3D>();
  let groundNode: THREE.Object3D | null = null;
  const raycaster = new THREE.Raycaster();
  const selectionBox = new THREE.Box3Helper(new THREE.Box3(), 0xffcc40);
  selectionBox.visible = false;
  scene.add(selectionBox);
  let dragging = false;

  function setup(el: HTMLDivElement) {
    container = el;
    renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    el.appendChild(renderer.domElement);
    camera = new THREE.OrthographicCamera(-1, 1, 1, -1, -100000, 100000);
    camera.position.set(0, 2000, 3000);
    orbit = new OrbitControls(camera, renderer.domElement);
    orbit.enableDamping = true;
    orbit.zoomToCursor = true;
    // left drag pans (or moves the selection, handler below), right drag
    // orbits around the point under the cursor (our own handler, OrbitControls
    // ignores the button), wheel zooms to the cursor
    orbit.mouseButtons = {
      LEFT: THREE.MOUSE.PAN,
      MIDDLE: THREE.MOUSE.DOLLY,
      RIGHT: null as unknown as THREE.MOUSE,
    };
    setupCursorOrbit(renderer.domElement);
    gizmo = new TransformControls(camera, renderer.domElement);
    gizmo.setMode("translate");
    gizmo.showY = false;
    scene.add(gizmo.getHelper());
    gizmo.addEventListener("dragging-changed", (e) => {
      dragging = !!(e as unknown as { value: boolean }).value;
      if (orbit) orbit.enabled = !dragging;
      if (!dragging) commitGizmo();
    });
    gizmo.addEventListener("objectChange", () => refreshSelectionBox());
    const resize = () => {
      const w = el.clientWidth;
      const h = el.clientHeight;
      if (!renderer || !camera || w === 0 || h === 0) return;
      renderer.setSize(w, h, false);
      renderer.setPixelRatio(window.devicePixelRatio);
      applyFrustum();
    };
    observer = new ResizeObserver(resize);
    observer.observe(el);
    resize();
    // click = pick (a click that did not orbit); alt-click picks a single part
    let downAt: [number, number] | null = null;
    renderer.domElement.addEventListener(
      "pointerdown",
      (e) => {
        if (e.button === 0) downAt = [e.clientX, e.clientY];
      },
      { signal: listeners.signal },
    );
    renderer.domElement.addEventListener(
      "pointerup",
      (e) => {
        if (!downAt || e.button !== 0) return;
        const moved = Math.hypot(e.clientX - downAt[0], e.clientY - downAt[1]);
        downAt = null;
        if (moved > 4 || dragging) return;
        pick(e, e.altKey);
      },
      { signal: listeners.signal },
    );
    const tick = () => {
      if (!renderer || !camera) return;
      if (flight) stepFlight();
      else orbit?.update();
      renderer.render(scene, camera);
      animationFrame = requestAnimationFrame(tick);
    };
    tick();
  }

  // ── camera flights: the view buttons glide instead of jumping ──
  interface CameraState {
    position: THREE.Vector3;
    quaternion: THREE.Quaternion;
    target: THREE.Vector3;
    frustum: number;
    zoom: number;
  }
  let flight: {
    from: CameraState;
    to: CameraState;
    start: number;
    ms: number;
  } | null = null;
  function currentState(): CameraState {
    return {
      position: camera!.position.clone(),
      quaternion: camera!.quaternion.clone(),
      target: orbit!.target.clone(),
      frustum,
      zoom: camera!.zoom,
    };
  }
  function flyTo(to: CameraState, ms = 700) {
    if (!camera || !orbit) return;
    flight = { from: currentState(), to, start: performance.now(), ms };
    orbit.enabled = false;
  }
  function stepFlight() {
    if (!flight || !camera || !orbit) return;
    const raw = Math.min(1, (performance.now() - flight.start) / flight.ms);
    const t = raw < 0.5 ? 2 * raw * raw : 1 - Math.pow(-2 * raw + 2, 2) / 2; // ease in-out
    const { from, to } = flight;
    camera.position.lerpVectors(from.position, to.position, t);
    camera.quaternion.slerpQuaternions(from.quaternion, to.quaternion, t);
    orbit.target.lerpVectors(from.target, to.target, t);
    frustum = from.frustum + (to.frustum - from.frustum) * t;
    camera.zoom = from.zoom + (to.zoom - from.zoom) * t;
    applyFrustum();
    if (raw >= 1) {
      flight = null;
      camera.up.set(0, 1, 0);
      orbit.enabled = true;
      orbit.update();
    }
  }
  /** the state OrbitControls would settle in for a camera at `position` looking at `target` */
  function lookState(
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

  /** the part view under a hit, if any */
  function partOfHit(h: THREE.Intersection): Level3DObject | null {
    let node: THREE.Object3D | null = h.object;
    while (node && !partViews.has(node.name)) node = node.parent;
    return node
      ? (doc()?.objects.find((o) => o.id === node!.name) ?? null)
      : null;
  }

  /**
   * Left drag on the selected building or part slides it along the ground
   * plane (elsewhere OrbitControls pans). Right drag orbits the camera
   * around the point under the cursor, keeping the picture in place.
   */
  function setupCursorOrbit(el: HTMLCanvasElement) {
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
    const up = new THREE.Vector3(0, 1, 0);
    // the right button is ours now, so OrbitControls no longer swallows the context menu
    el.addEventListener("contextmenu", (e) => e.preventDefault(), {
      signal: listeners.signal,
    });
    const setRay = (e: PointerEvent) => {
      const rect = el.getBoundingClientRect();
      const ndc = new THREE.Vector2(
        ((e.clientX - rect.left) / rect.width) * 2 - 1,
        -((e.clientY - rect.top) / rect.height) * 2 + 1,
      );
      raycaster.setFromCamera(ndc, camera!);
    };
    el.addEventListener(
      "pointerdown",
      (e) => {
        // the gizmo takes precedence when the cursor is on one of its handles
        if (
          !camera ||
          !orbit ||
          gizmo?.axis ||
          (e.button !== 0 && e.button !== 2)
        )
          return;
        setRay(e);
        const hits = raycaster.intersectObjects(
          [objectsRoot, ...(groundNode ? [groundNode] : [])],
          true,
        );
        if (e.button === 0) {
          // a left drag that starts on the selection moves it along the ground plane
          const s = selected();
          const hitPart = hits[0] ? partOfHit(hits[0]) : null;
          const view = selectedView();
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
            orbit.enabled = false;
            dragging = true;
            el.setPointerCapture(e.pointerId);
          }
          return;
        }
        const pivot = hits[0]?.point.clone() ?? orbit.target.clone();
        const offset = camera.position.clone().sub(orbit.target);
        active = {
          pivot,
          startX: e.clientX,
          startY: e.clientY,
          position: camera.position.clone(),
          quaternion: camera.quaternion.clone(),
          target: orbit.target.clone(),
          right: new THREE.Vector3(1, 0, 0).applyQuaternion(camera.quaternion),
          polar: Math.acos(THREE.MathUtils.clamp(offset.normalize().y, -1, 1)),
        };
        orbit.enabled = false;
        dragging = true;
        el.setPointerCapture(e.pointerId);
      },
      { signal: listeners.signal },
    );
    el.addEventListener(
      "pointermove",
      (e) => {
        if (moving && camera) {
          setRay(e);
          const point = new THREE.Vector3();
          if (!raycaster.ray.intersectPlane(moving.plane, point)) return;
          // the delta in the wrapper's parent frame (a group's local frame for parts)
          const parent = moving.view.wrapper.parent!;
          const delta = parent
            .worldToLocal(point.clone())
            .sub(parent.worldToLocal(moving.start.clone()));
          moving.view.wrapper.position.copy(moving.startPos).add(delta);
          refreshSelectionBox();
          return;
        }
        if (!active || !camera || !orbit) return;
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
        camera.position
          .copy(active.position)
          .sub(active.pivot)
          .applyQuaternion(q)
          .add(active.pivot);
        camera.quaternion.copy(q).multiply(active.quaternion);
        camera.up.copy(up);
        orbit.target
          .copy(active.target)
          .sub(active.pivot)
          .applyQuaternion(q)
          .add(active.pivot);
      },
      { signal: listeners.signal },
    );
    const end = (e: PointerEvent) => {
      if (e.button === 0 && moving) {
        moving = null;
        commitGizmo();
      } else if (e.button === 2 && active) active = null;
      else return;
      dragging = false;
      if (orbit) {
        orbit.enabled = true;
        orbit.update();
      }
    };
    el.addEventListener("pointerup", end, { signal: listeners.signal });
    el.addEventListener("pointercancel", end, { signal: listeners.signal });
  }

  function applyFrustum() {
    if (!camera || !container) return;
    const aspect = container.clientWidth / Math.max(1, container.clientHeight);
    camera.left = -frustum * aspect;
    camera.right = frustum * aspect;
    camera.top = frustum;
    camera.bottom = -frustum;
    camera.updateProjectionMatrix();
  }

  function contentBox(): THREE.Box3 {
    const box = new THREE.Box3();
    if (groundNode) box.expandByObject(groundNode);
    box.expandByObject(objectsRoot);
    return box;
  }

  function frameContent(instant = false) {
    if (!camera || !orbit) return;
    const box = contentBox();
    if (box.isEmpty()) return;
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3()).length();
    const to = lookState(
      center.clone().add(new THREE.Vector3(0, size * 0.5, size * 0.6)),
      center,
      size * 0.35,
    );
    if (instant) applyState(to);
    else flyTo(to);
  }

  /** the original pre-render camera: elevation from the document, looking north, map fitted */
  function gameCamera(instant = false) {
    const d = doc();
    if (!camera || !orbit) return;
    const box = contentBox();
    if (box.isEmpty()) return;
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3()).length();
    const t = ((d?.camera.elevation_deg ?? 35) * Math.PI) / 180;
    const forward = new THREE.Vector3(0, -Math.sin(t), -Math.cos(t));
    const to = lookState(
      center.clone().addScaledVector(forward, -size),
      center,
      d ? d.size[1] / 2 : size * 0.35,
    );
    if (instant) applyState(to);
    else flyTo(to);
  }

  function applyState(st: CameraState) {
    if (!camera || !orbit) return;
    flight = null;
    camera.position.copy(st.position);
    camera.quaternion.copy(st.quaternion);
    camera.up.set(0, 1, 0);
    orbit.target.copy(st.target);
    frustum = st.frustum;
    camera.zoom = st.zoom;
    applyFrustum();
    orbit.enabled = true;
    orbit.update();
  }

  // ── scenes in the library ──
  createEffect(
    () => props.library(),
    (lib) => {
      session.beginLoad();
      setMaps([]);
      if (!lib) return;
      void (async () => {
        const dir = await subdir(lib.handle, ["scenes"]);
        if (!dir) return;
        const files = await listFiles(dir);
        const names = files
          .filter((f) => f.endsWith("-volumes.scene.glb"))
          .map((f) => f.slice(0, -"-volumes.scene.glb".length))
          .sort();
        if (disposed || props.library() !== lib) return;
        setMaps(names);
        if (names.length === 1) void openMap(names[0]!);
      })().catch((error) => {
        if (!disposed && props.library() === lib) props.onError(String(error));
      });
    },
  );

  // ── document ──
  function publishRevision() {
    const current = session.current;
    if (!current) return;
    setHistory({ past: [...current.past], future: [...current.future] });
    setDoc(current.document);
    setDirty(session.dirty);
    syncViews(current.document);
  }
  function pushHistory(next: Level3D) {
    session.edit(next);
    publishRevision();
  }
  function undo() {
    session.undo();
    publishRevision();
  }
  function redo() {
    session.redo();
    publishRevision();
  }
  function updatePart(id: string, patch: Partial<Level3DObject>) {
    const d = doc();
    if (!d) return;
    pushHistory({
      ...d,
      objects: d.objects.map((o) => (o.id === id ? { ...o, ...patch } : o)),
    });
  }
  function updateGroup(id: string, patch: Partial<Level3DGroup>) {
    const d = doc();
    if (!d) return;
    pushHistory({
      ...d,
      groups: d.groups.map((g) => (g.id === id ? { ...g, ...patch } : g)),
    });
  }
  const selectedPart = () => {
    const s = selected();
    return s?.kind === "part"
      ? (doc()?.objects.find((o) => o.id === s.id) ?? null)
      : null;
  };
  const selectedGroup = () => {
    const s = selected();
    return s?.kind === "group"
      ? (doc()?.groups.find((g) => g.id === s.id) ?? null)
      : null;
  };
  /** the transform the panel edits: the selected group's or part's */
  const selectedTransform = (): GameTransform | null =>
    selectedGroup()?.transform ?? selectedPart()?.transform ?? null;
  function setTransform(t: GameTransform) {
    const g = selectedGroup();
    const p = selectedPart();
    if (g) updateGroup(g.id, { transform: t });
    else if (p) updatePart(p.id, { transform: t });
  }

  async function openMap(name: string) {
    const lib = props.library();
    const idx = props.index();
    if (!lib) return;
    const generation = session.beginLoad();
    let preparedAsset: THREE.Object3D | null = null;
    props.onStatus(`loading ${name}…`);
    try {
      const dir = await subdir(lib.handle, ["scenes"]);
      if (!dir) throw new Error("scenes/ missing");
      const glbName = `${name}-volumes.scene.glb`;
      const sceneDoc = parseSceneDoc(
        await readJson<unknown>(dir, `${name}-volumes.scene.json`),
      );
      if (sceneDoc.map.toLowerCase() !== name.toLowerCase()) {
        throw new Error(
          `${name}-volumes.scene.json: source map is ${sceneDoc.map}`,
        );
      }
      const lvl = idx ? await loadProtoLevel(idx, sceneDoc.map) : null;
      const file = await (await dir.getFileHandle(glbName)).getFile();
      const bytes = await file.arrayBuffer();
      const provenance = await documentProvenance(lvl, bytes);
      const gltf = await new GLTFLoader().parseAsync(bytes, "");
      preparedAsset = gltf.scene;
      const nextSources = new Map<string, THREE.Object3D>();
      let nextGround: THREE.Object3D | null = null;
      const root =
        gltf.scene.children.find((c) => c.name === "map") ?? gltf.scene;
      for (const child of [...root.children]) {
        if (child.name === "ground") nextGround = child;
        else
          for (const node of child.children) {
            if (nextSources.has(node.name))
              throw new Error(`Duplicate GLB node ${node.name}`);
            nextSources.set(node.name, node);
          }
      }
      const nextSuspects = new Map<
        number,
        { delta: number; support: number }
      >();
      let d: Level3D | null = null;
      const docName = `${name}.level3d.json`;
      const files = await listFiles(dir);
      if (files.includes(docName)) {
        d = parseLevel3D(await readJson<unknown>(dir, docName), {
          map: sceneDoc.map,
          glb: glbName,
          level: lvl ?? undefined,
          nodes: new Set(nextSources.keys()),
        });
        if (lvl) {
          const terraces = new Set(
            d.objects
              .filter((o) => o.kind === "terrace")
              .map((o) => o.source.obstacle),
          );
          const sus = new Map<number, { delta: number; support: number }>();
          for (const x of snapFloatingParts(lvl.sight_obstacles, terraces, {
            includeOpaque: true,
          }).snapped)
            sus.set(x.index, { delta: x.delta, support: x.support });
          for (const [key, value] of sus) nextSuspects.set(key, value);
        }
      }
      if (!d) {
        if (!lvl)
          throw new Error(
            "connect the datadir to build the level document from the game data",
          );
        const objects: Level3DObject[] = [];
        const terraces = new Set<number>();
        for (const nodeName of [...nextSources.keys()].sort()) {
          const m = /^(building|terrace)-(\d+)$/.exec(nodeName);
          if (!m) continue;
          const obstacle = Number(m[2]);
          if (m[1] === "terrace") terraces.add(obstacle);
          objects.push({
            id: nodeName,
            kind: m[1] as "building" | "terrace",
            node: nodeName,
            source: { map: sceneDoc.map, obstacle },
            obstacle: lvl.sight_obstacles[obstacle]!,
            transform: { ...IDENTITY_TRANSFORM },
          });
        }
        // buildings: parts stacked on the same footprint
        const groupOf = groupObstacles(lvl.sight_obstacles, terraces);
        // parts that may be stored displaced along the view ray: offered as a per-part snap, never applied automatically
        const sus = new Map<number, { delta: number; support: number }>();
        for (const x of snapFloatingParts(lvl.sight_obstacles, terraces, {
          includeOpaque: true,
        }).snapped)
          sus.set(x.index, { delta: x.delta, support: x.support });
        for (const [key, value] of sus) nextSuspects.set(key, value);
        const groups: Level3DGroup[] = [];
        const seen = new Set<string>();
        for (const o of objects) {
          const root = groupOf.get(o.source.obstacle);
          if (root === undefined) continue;
          o.group = groupId(root);
          if (!seen.has(o.group)) {
            seen.add(o.group);
            groups.push({ id: o.group, transform: { ...IDENTITY_TRANSFORM } });
          }
        }
        d = {
          version: 1,
          map: sceneDoc.map,
          size: sceneDoc.size,
          camera: sceneDoc.camera,
          glb: glbName,
          objects,
          groups,
        };
      }
      parseLevel3D(d, {
        scene: sceneDoc,
        map: sceneDoc.map,
        glb: glbName,
        level: lvl ?? undefined,
        nodes: new Set(nextSources.keys()),
        sourceSha256: provenance.source_sha256,
        glbSha256: provenance.glb_sha256,
      });
      const hadProvenance = !!d.provenance;
      d = {
        ...d,
        provenance: {
          ...d.provenance,
          ...provenance,
          source_sha256:
            provenance.source_sha256 ?? d.provenance?.source_sha256,
        },
      };
      if (
        disposed ||
        !session.isCurrent(generation) ||
        props.library() !== lib ||
        props.index() !== idx
      ) {
        disposeObjectResources([preparedAsset]);
        preparedAsset = null;
        return;
      }
      // All asynchronous reads and validation precede publication.
      select(null);
      disposeObjectResources([
        overlayRoot,
        ...(sourceAsset ? [sourceAsset] : []),
        ...(groundNode ? [groundNode] : []),
      ]);
      if (groundNode) mapRoot.remove(groundNode);
      objectsRoot.clear();
      overlayRoot.clear();
      partViews.clear();
      groupViews.clear();
      sourceNodes.clear();
      sourceAsset = preparedAsset;
      preparedAsset = null;
      groundNode = nextGround;
      if (groundNode) mapRoot.add(groundNode);
      for (const [key, value] of nextSources) sourceNodes.set(key, value);
      session.publish(
        generation,
        name,
        d,
        dir,
        files.includes(docName) && hadProvenance,
      );
      setMapName(name);
      setLevel(lvl);
      setSuspects(nextSuspects);
      setHistory({ past: [], future: [] });
      setDirty(session.dirty);
      setDoc(d);
      syncViews(d);
      buildOverlays();
      gameCamera(true);
      setInfo(`${d.groups.length} buildings, ${d.objects.length} parts`);
      props.onStatus(null);
    } catch (e) {
      if (preparedAsset) disposeObjectResources([preparedAsset]);
      if (session.isCurrent(generation) && !disposed) {
        props.onStatus(null);
        props.onError(String(e));
      }
    }
  }

  function makeView(name: string): View {
    const wrapper = new THREE.Group();
    wrapper.name = name;
    const rot = new THREE.Group();
    rot.matrixAutoUpdate = false;
    wrapper.add(rot);
    return { wrapper, rot, meshes: [] };
  }

  function setAffine(v: View, m: number[]) {
    v.wrapper.position.set(m[12]!, m[13]!, m[14]!);
    const rest = new THREE.Matrix4().fromArray(m);
    rest.setPosition(0, 0, 0);
    v.rot.matrix.copy(rest);
    v.rot.matrixWorldNeedsUpdate = true;
  }

  /** make the three.js objects match the document */
  function syncViews(d: Level3D) {
    const aliveGroups = new Set<string>();
    for (const g of d.groups) {
      aliveGroups.add(g.id);
      let v = groupViews.get(g.id);
      if (!v) {
        v = makeView(g.id);
        objectsRoot.add(v.wrapper);
        groupViews.set(g.id, v);
      }
      setAffine(
        v,
        gameTransformMatrix(
          d.camera,
          g.transform,
          groupCentroid(groupParts(d, g.id)),
        ),
      );
      v.wrapper.visible = !g.hidden;
    }
    for (const [id, v] of groupViews) {
      if (aliveGroups.has(id)) continue;
      objectsRoot.remove(v.wrapper);
      groupViews.delete(id);
    }
    const aliveParts = new Set<string>();
    for (const o of d.objects) {
      aliveParts.add(o.id);
      let v = partViews.get(o.id);
      if (!v) {
        const src = sourceNodes.get(o.node);
        if (!src) throw new Error(`Missing source node ${o.node} for ${o.id}`);
        v = makeView(o.id);
        const node = src.clone(true);
        node.traverse((c) => {
          const m = c as THREE.Mesh;
          if (m.isMesh) v!.meshes.push(m);
        });
        v.rot.add(node);
        partViews.set(o.id, v);
      }
      const parent =
        (o.group ? groupViews.get(o.group)?.rot : undefined) ?? objectsRoot;
      if (v.wrapper.parent !== parent) parent.add(v.wrapper);
      setAffine(
        v,
        gameTransformMatrix(
          d.camera,
          o.transform,
          obstacleCentroid(o.obstacle.points),
        ),
      );
      v.wrapper.visible = !o.hidden;
    }
    for (const [id, v] of partViews) {
      if (aliveParts.has(id)) continue;
      v.wrapper.parent?.remove(v.wrapper);
      partViews.delete(id);
    }
    const s = selected();
    if (s && !(s.kind === "group" ? aliveGroups : aliveParts).has(s.id))
      select(null);
    else refreshSelectionBox();
    if (showObstacles()) buildOverlays();
  }

  function selectedView(): View | null {
    const s = selected();
    if (!s) return null;
    return (s.kind === "group" ? groupViews : partViews).get(s.id) ?? null;
  }

  /** read the gizmo's translation back into the document */
  function commitGizmo() {
    const d = doc();
    const v = selectedView();
    const t = selectedTransform();
    if (!d || !v || !t) return;
    const g = selectedGroup();
    const p = selectedPart();
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
    setTransform({ ...t, dx, dy, dz });
  }

  function pick(e: PointerEvent, partOnly: boolean) {
    if (!camera || !renderer) return;
    const rect = renderer.domElement.getBoundingClientRect();
    const ndc = new THREE.Vector2(
      ((e.clientX - rect.left) / rect.width) * 2 - 1,
      -((e.clientY - rect.top) / rect.height) * 2 + 1,
    );
    raycaster.setFromCamera(ndc, camera);
    const hits = raycaster.intersectObject(objectsRoot, true);
    for (const h of hits) {
      const part = partOfHit(h);
      if (!part) continue;
      // a click inside the selected building picks its part; alt always does
      const s = selected();
      if (
        part.group &&
        !partOnly &&
        !(s?.kind === "group" && s.id === part.group)
      )
        select({ kind: "group", id: part.group });
      else select({ kind: "part", id: part.id });
      return;
    }
    select(null);
  }

  const tinted = new Map<THREE.Mesh, THREE.Material | THREE.Material[]>();
  function select(s: Selection) {
    for (const [m, mat] of tinted) {
      for (const owned of Array.isArray(m.material) ? m.material : [m.material])
        owned.dispose();
      m.material = mat;
    }
    tinted.clear();
    setSelected(s);
    const d = doc();
    const v = s
      ? (s.kind === "group" ? groupViews : partViews).get(s.id)
      : null;
    if (gizmo) {
      if (v) gizmo.attach(v.wrapper);
      else gizmo.detach();
    }
    if (s && d) {
      const parts =
        s.kind === "group"
          ? groupParts(d, s.id)
          : d.objects.filter((o) => o.id === s.id);
      for (const p of parts) {
        for (const m of partViews.get(p.id)?.meshes ?? []) {
          tinted.set(m, m.material);
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
      if (s.kind === "part") {
        const g = d.objects.find((o) => o.id === s.id)?.group;
        if (g) setExpanded((x) => new Set(x).add(g));
      }
    }
    refreshSelectionBox();
  }

  function refreshSelectionBox() {
    const v = selectedView();
    if (!v) {
      selectionBox.visible = false;
      return;
    }
    v.wrapper.updateWorldMatrix(true, true);
    selectionBox.box.setFromObject(v.wrapper, true);
    selectionBox.visible = true;
  }

  // ── overlays: obstacle outlines (document state) and elevation lines ──
  function buildOverlays() {
    disposeObjectResources([overlayRoot]);
    overlayRoot.clear();
    const d = doc();
    if (!d) return;
    if (showObstacles()) {
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
      overlayRoot.add(
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
    const lvl = level();
    if (showElevation() && lvl) {
      const pts: number[] = [];
      for (const e of lvl.elevation_lines) {
        const p = gameToScene(d.camera, e.point_a[0], e.point_a[1], 0);
        const q = gameToScene(d.camera, e.point_b[0], e.point_b[1], 0);
        pts.push(p[0], p[1], p[2] + 1, q[0], q[1], q[2] + 1);
      }
      const geo = new THREE.BufferGeometry();
      geo.setAttribute("position", new THREE.Float32BufferAttribute(pts, 3));
      overlayRoot.add(
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
  createEffect(
    () => ({ obstacles: showObstacles(), elevation: showElevation() }),
    () => buildOverlays(),
  );
  createEffect(
    () => gizmoVertical(),
    (v) => {
      if (gizmo) gizmo.showY = v;
    },
  );

  // ── actions ──
  function duplicateSelected() {
    const d = doc();
    const g = selectedGroup();
    const p = selectedPart();
    if (!d) return;
    if (g) {
      let n = 1;
      while (d.groups.some((x) => x.id === `${g.id}-copy${n}`)) n++;
      const id = `${g.id}-copy${n}`;
      const copyGroup: Level3DGroup = {
        ...g,
        id,
        transform: {
          ...g.transform,
          dx: g.transform.dx + 40,
          dy: g.transform.dy + 20,
        },
      };
      const copies = groupParts(d, g.id).map((o) => ({
        ...o,
        id: `${o.id}-${id}`,
        group: id,
      }));
      pushHistory({
        ...d,
        groups: [...d.groups, copyGroup],
        objects: [...d.objects, ...copies],
      });
      select({ kind: "group", id });
    } else if (p) {
      let n = 1;
      while (d.objects.some((x) => x.id === `${p.id}-copy${n}`)) n++;
      const copy: Level3DObject = {
        ...p,
        id: `${p.id}-copy${n}`,
        transform: {
          ...p.transform,
          dx: p.transform.dx + 40,
          dy: p.transform.dy + 20,
        },
      };
      pushHistory({ ...d, objects: [...d.objects, copy] });
      select({ kind: "part", id: copy.id });
    }
  }
  function deleteSelected() {
    const d = doc();
    const g = selectedGroup();
    const p = selectedPart();
    if (!d) return;
    select(null);
    if (g)
      pushHistory({
        ...d,
        groups: d.groups.filter((x) => x.id !== g.id),
        objects: d.objects.filter((o) => o.group !== g.id),
      });
    else if (p)
      pushHistory({ ...d, objects: d.objects.filter((o) => o.id !== p.id) });
  }
  function rotateSelected(delta: number) {
    const t = selectedTransform();
    if (!t) return;
    setTransform({ ...t, rot_deg: (((t.rot_deg + delta) % 360) + 360) % 360 });
  }
  function setTransformField(field: keyof GameTransform, value: number) {
    const t = selectedTransform();
    if (!t || !Number.isFinite(value)) return;
    setTransform({ ...t, [field]: value });
  }
  function setHidden(hidden: boolean) {
    const g = selectedGroup();
    const p = selectedPart();
    if (g) updateGroup(g.id, { hidden });
    else if (p) updatePart(p.id, { hidden });
  }
  async function save() {
    if (!session.current || saving) return;
    const snapshot = session.captureSave();
    saving = true;
    try {
      await writeText(
        snapshot.resources,
        `${snapshot.name}.level3d.json`,
        JSON.stringify(snapshot.document, null, 2),
      );
      session.saved(snapshot);
      if (!disposed && session.current === snapshot.session) {
        setDirty(session.dirty);
        props.onStatus(`saved ${snapshot.name}.level3d.json`);
      }
    } catch (e) {
      if (!disposed) props.onError(String(e));
    } finally {
      saving = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if ((e.target as HTMLElement).tagName === "INPUT") return;
    if (e.key === "z" && (e.ctrlKey || e.metaKey) && !e.shiftKey) {
      e.preventDefault();
      undo();
    } else if (
      (e.key === "z" && (e.ctrlKey || e.metaKey) && e.shiftKey) ||
      (e.key === "y" && e.ctrlKey)
    ) {
      e.preventDefault();
      redo();
    } else if (e.key === "s" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void save();
    } else if (e.key === "Delete" || e.key === "Backspace") deleteSelected();
    else if (e.key === "d" && !e.ctrlKey) duplicateSelected();
    else if (e.key === "q") rotateSelected(-15);
    else if (e.key === "e") rotateSelected(15);
    else if (e.key === "g") gameCamera();
    else if (e.key === "f") frameContent();
    else if (e.key === "Escape") select(null);
  }
  window.addEventListener("keydown", onKey, { signal: listeners.signal });
  onCleanup(() => {
    disposed = true;
    session.beginLoad();
    listeners.abort();
    cancelAnimationFrame(animationFrame);
    observer?.disconnect();
    select(null);
    gizmo?.dispose();
    orbit?.dispose();
    disposeObjectResources([
      overlayRoot,
      selectionBox,
      ...(sourceAsset ? [sourceAsset] : []),
      ...(groundNode ? [groundNode] : []),
    ]);
    renderer?.dispose();
    // The viewport owns this context; release driver-owned default textures and
    // framebuffers too when its canvas is permanently removed.
    renderer?.forceContextLoss();
    renderer?.domElement.remove();
    renderer = null;
  });

  // ── object list: buildings (expandable), then ungrouped parts and terraces ──
  interface Row {
    kind: "group" | "part";
    id: string;
    label: string;
    depth: number;
    hidden: boolean;
    moved: boolean;
    parts?: number;
    suspect?: boolean;
  }
  const rows = (): Row[] => {
    const d = doc();
    if (!d) return [];
    const q = filter().toLowerCase();
    const match = (id: string, name?: string) =>
      !q ||
      id.toLowerCase().includes(q) ||
      (name ?? "").toLowerCase().includes(q);
    const out: Row[] = [];
    const exp = expanded();
    for (const g of d.groups) {
      const parts = groupParts(d, g.id);
      const partMatch = parts.filter((p) => match(p.id, p.name));
      if (!match(g.id, g.name) && partMatch.length === 0) continue;
      out.push({
        kind: "group",
        id: g.id,
        label: g.name ?? g.id,
        depth: 0,
        hidden: !!g.hidden,
        moved: !isIdentity(g.transform),
        parts: parts.length,
        suspect: parts.some((p) => suspects().has(p.source.obstacle)),
      });
      if (exp.has(g.id) || (q && partMatch.length > 0)) {
        for (const p of parts)
          if (!q || match(p.id, p.name))
            out.push({
              kind: "part",
              id: p.id,
              label: p.name ?? p.id,
              depth: 1,
              hidden: !!p.hidden,
              moved: !isIdentity(p.transform),
              suspect: suspects().has(p.source.obstacle),
            });
      }
    }
    for (const o of d.objects) {
      if (o.group || !match(o.id, o.name)) continue;
      out.push({
        kind: "part",
        id: o.id,
        label: o.name ?? o.id,
        depth: 0,
        hidden: !!o.hidden,
        moved: !isIdentity(o.transform),
        suspect: suspects().has(o.source.obstacle),
      });
    }
    return out;
  };
  const isSelected = (r: Row) => {
    const s = selected();
    return !!s && s.kind === r.kind && s.id === r.id;
  };
  const toggleExpanded = (id: string) =>
    setExpanded((x) => {
      const n = new Set(x);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const selectionTitle = () => {
    const g = selectedGroup();
    const p = selectedPart();
    const d = doc();
    if (g && d)
      return `${g.name ?? g.id} (${groupParts(d, g.id).length} parts)`;
    if (p) return p.name ?? p.id;
    return "";
  };

  return (
    <div class="editor">
      <div class="editor-bar">
        <For each={maps()}>
          {(m) => (
            <button
              class={mapName() === m ? "selected" : ""}
              onClick={() => void openMap(m)}
            >
              {m}
            </button>
          )}
        </For>
        <Show when={maps().length === 0}>
          <span class="hint">
            No reconstructions in the library (run{" "}
            <code>pnpm volumes --map &lt;map&gt;</code>).
          </span>
        </Show>
        <span class="spacer" />
        <button disabled={!doc()} onClick={() => gameCamera()} title="g">
          Game camera
        </button>
        <button disabled={!doc()} onClick={() => frameContent()} title="f">
          Frame
        </button>
        <label class="check inline">
          <input
            type="checkbox"
            checked={showObstacles()}
            onChange={(e) => setShowObstacles(e.currentTarget.checked)}
          />{" "}
          obstacles
        </label>
        <label class="check inline">
          <input
            type="checkbox"
            checked={showElevation()}
            onChange={(e) => setShowElevation(e.currentTarget.checked)}
          />{" "}
          elevation lines
        </label>
        <button
          disabled={history().past.length === 0}
          onClick={undo}
          title="ctrl+z"
        >
          Undo
        </button>
        <button
          disabled={history().future.length === 0}
          onClick={redo}
          title="ctrl+shift+z"
        >
          Redo
        </button>
        <button disabled={!dirty()} onClick={() => void save()} title="ctrl+s">
          Save{dirty() ? " *" : ""}
        </button>
        <Show when={info()}>
          {(s) => <span class="editor-status">{s()}</span>}
        </Show>
      </div>
      <div class="editor-body">
        <div class="editor-canvas" ref={setup} />
        <aside class="editor-panel">
          <Show
            when={selectedTransform()}
            fallback={
              <p class="hint">
                Click a building to select it, alt-click or click again for a
                single part; drag the selection to move it along the ground.
                Left drag elsewhere pans, right drag orbits around the point
                under the cursor, wheel zooms to the cursor.
              </p>
            }
          >
            {(t) => (
              <section class="object-detail">
                <h2>{selectionTitle()}</h2>
                <Show when={selectedPart()}>
                  {(p) => (
                    <>
                      <div class="meta-row">
                        <span class="meta-key">source</span>
                        <span>
                          {p().source.map} #{p().source.obstacle}
                        </span>
                      </div>
                      <div class="meta-row">
                        <span class="meta-key">flags</span>
                        <span>
                          {p().obstacle.opaque ? "opaque " : "clear "}
                          {(p().obstacle as unknown as { solid?: boolean })
                            .solid
                            ? "solid"
                            : ""}
                        </span>
                      </div>
                      <div class="meta-row">
                        <span class="meta-key">height</span>
                        <span>
                          {Math.round(
                            Math.min(
                              ...p().obstacle.points.map((q) => q.z_bottom),
                            ),
                          )}
                          –
                          {Math.round(
                            Math.max(
                              ...p().obstacle.points.map((q) => q.z_top),
                            ),
                          )}
                        </span>
                      </div>
                      <Show when={p().group}>
                        {(g) => (
                          <button
                            onClick={() => select({ kind: "group", id: g() })}
                          >
                            Select building {g()}
                          </button>
                        )}
                      </Show>
                      <Show when={suspects().get(p().source.obstacle)}>
                        {(sus) => (
                          <div class="row suspect">
                            <span class="hint">
                              Floats {Math.round(sus().delta)} above #
                              {sus().support}; may be stored displaced along the
                              view ray (same map pixels).
                            </span>
                            <button
                              onClick={() => {
                                const t = p().transform;
                                setTransform({
                                  ...t,
                                  dy: t.dy - sus().delta,
                                  dz: t.dz - sus().delta,
                                });
                              }}
                            >
                              Snap down {Math.round(sus().delta)}
                            </button>
                          </div>
                        )}
                      </Show>
                    </>
                  )}
                </Show>
                <h3>
                  Transform
                  {selectedPart()?.group ? " (within the building)" : ""}
                </h3>
                <For each={["dx", "dy", "dz", "rot_deg"] as const}>
                  {(f) => (
                    <div class="meta-row">
                      <span class="meta-key">{f}</span>
                      <input
                        type="number"
                        step={f === "rot_deg" ? 5 : 1}
                        value={t()[f]}
                        onChange={(e) =>
                          setTransformField(f, Number(e.currentTarget.value))
                        }
                      />
                    </div>
                  )}
                </For>
                <div class="row">
                  <button onClick={() => rotateSelected(-15)} title="q">
                    ⟲ 15°
                  </button>
                  <button onClick={() => rotateSelected(15)} title="e">
                    ⟳ 15°
                  </button>
                  <label class="check inline">
                    <input
                      type="checkbox"
                      checked={gizmoVertical()}
                      onChange={(e) =>
                        setGizmoVertical(e.currentTarget.checked)
                      }
                    />{" "}
                    lift
                  </label>
                </div>
                <div class="row">
                  <button onClick={duplicateSelected} title="d">
                    Duplicate
                  </button>
                  <button onClick={deleteSelected} title="del">
                    Delete
                  </button>
                  <button
                    onClick={() => setTransform({ ...IDENTITY_TRANSFORM })}
                  >
                    Reset
                  </button>
                  <label class="check inline">
                    <input
                      type="checkbox"
                      checked={
                        !!(selectedGroup()?.hidden ?? selectedPart()?.hidden)
                      }
                      onChange={(e) => setHidden(e.currentTarget.checked)}
                    />{" "}
                    hidden
                  </label>
                </div>
              </section>
            )}
          </Show>
          <section class="object-list">
            <div class="search-row">
              <input
                class="search"
                placeholder="filter buildings and parts"
                value={filter()}
                onInput={(e) => setFilter(e.currentTarget.value)}
              />
            </div>
            <ul>
              <For each={rows()}>
                {(r) => (
                  <li
                    class={`${isSelected(r) ? "selected" : ""} ${r.hidden ? "hidden" : ""} depth-${r.depth}`}
                    onClick={() => select({ kind: r.kind, id: r.id })}
                  >
                    <Show
                      when={r.kind === "group"}
                      fallback={
                        <span class="kind">
                          {r.id.startsWith("terrace") ? "▬" : "·"}
                        </span>
                      }
                    >
                      <span
                        class="chev-btn"
                        onClick={(e) => {
                          e.stopPropagation();
                          toggleExpanded(r.id);
                        }}
                      >
                        {expanded().has(r.id) ? "▾" : "▸"}
                      </span>
                    </Show>
                    {r.label}
                    <Show when={r.parts !== undefined}>
                      <span class="count">{r.parts}</span>
                    </Show>
                    <Show when={r.moved}>
                      <span class="tag">moved</span>
                    </Show>
                    <Show when={r.suspect}>
                      <span
                        class="tag suspect"
                        title="may float: stored displaced along the view ray"
                      >
                        float?
                      </span>
                    </Show>
                  </li>
                )}
              </For>
            </ul>
          </section>
        </aside>
      </div>
    </div>
  );
}
