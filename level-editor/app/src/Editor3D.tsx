import EditorHelp from "./EditorHelp";
import TerrainPanel from "./TerrainPanel";
import NewMapSettings from "./NewMapSettings";
import WorkspacePanel from "./WorkspacePanel";
import { followTerrainEdit, followTerrainTransform } from "./terrain-follow";
import MissionPanel from "./MissionPanel";
import InteriorConnectionsPanel from "./InteriorConnectionsPanel";
// Edit JSON maps assembled from pinned library assets, with game and orbit cameras.
import { For, Show, createEffect, createMemo, createSignal, onCleanup, untrack } from "solid-js";
import type { JSX } from "@solidjs/web";
import type * as THREE from "three";
import {
  serializeStoredMap,
  parseLevel3D,
  groupParts,
  type GameTransform,
  type Level3D,
  type Level3DGroup,
  type Level3DObject,
  type ProtoLevel,
  type ProjectionAssetEntry,
} from "@rle/shared";
import { SessionPublication, type SessionSnapshot } from "./session-publication";
import {
  duplicateSelection,
  setGroupState,
  stateOwner,
  deleteSelection,
  patchPart,
  patchGroup,
  type Selection,
} from "./document-commands";
import { createNewMap, defaultNewMapOptions } from "./new-map";
import MapCard from "./MapCard";
import { encodeMapThumbnail, writeMapThumbnail } from "./map-thumbnail";
import SplinePanel from "./SplinePanel";
import LightingPanel from "./LightingPanel";
import AssetLibrary from "./AssetLibrary";
import { AssetPreviewRenderer } from "./AssetPreview";
import ScrubNumber from "./ScrubNumber";
import { ASSET_DRAG_TYPE } from "./asset-library";
import { insertProjectionAsset } from "./asset-commands";
import {
  listProjectionAssets,
  prepareProjectionPlacement,
  readPinnedAssetDescriptors,
  libraryFile,
} from "./projection-library";
import { prepareMapCandidate } from "./map-candidate";
import { EditorViewport } from "./editor-viewport";
import { disposeObjectResources } from "./resources";
import { listFiles, subdir, writeText } from "./fs";
import { MissionEntities, readMission } from "./mission";
import { loadEditableMission, remainingMissionPreview } from "./import-mission.ts";
import { PopulationView, type SceneEntities } from "./population-view";
import type { DatadirIndex } from "./datadir";
import { missionsForMap } from "./mission-catalog.ts";
import { downloadMap, downloadMapFile } from "./http-library.ts";
import { MapExportWorker } from "./map-export-client.ts";
import type { BakeProgress } from "./map-bake-render.ts";

export type { Selection } from "./document-commands";

/** the library directory handle, wrapped because handles are async-iterable and Solid 2 would iterate them */
export interface LibraryRef {
  handle: FileSystemDirectoryHandle;
  mapLabels?: () => Promise<ReadonlyMap<string, string>>;
  documentMap?: (name: string) => string;
  savedMapName?: (name: string) => string;
  availableMapName?: (name: string) => Promise<string>;
  saveMap?: (name: string, document: unknown, thumbnail: Blob) => Promise<string>;
  isBuiltIn?: (name: string) => boolean;
  deleteMap?: (name: string) => Promise<void>;
  renameMap?: (name: string, next: string) => Promise<string>;
}

export interface EditorProps {
  index: () => DatadirIndex | null;
  library: () => LibraryRef | null;
  onError: (msg: string) => void;
  onStatus: (msg: string | null, busy?: boolean) => void;
  toolbarStart?: () => JSX.Element;
  toolbarEnd?: () => JSX.Element;
}

export default function Editor3D(props: EditorProps) {
  const previewRenderer = new AssetPreviewRenderer();
  onCleanup(() => previewRenderer.dispose());
  let viewportElement!: HTMLDivElement;
  let newMapDialog!: HTMLDialogElement;
  const [newMapName, setNewMapName] = createSignal("Untitled map");
  const [newMapOptions, setNewMapOptions] = createSignal(defaultNewMapOptions());
  const [assetDisplayMode, setAssetDisplayMode] = createSignal<"visible" | "outline" | "hidden">(
    "visible",
  );
  const [creatingMap, setCreatingMap] = createSignal(false);
  const [newMapError, setNewMapError] = createSignal("");
  const [panel, setPanel] = createSignal("Assets");
  const [viewSettings, setViewSettings] = createSignal(false);
  const [libraryMount, setLibraryMount] = createSignal<HTMLElement>();
  const [libraryOpen, setLibraryOpen] = createSignal(true);
  const [libraryWidth, setLibraryWidth] = createSignal(284);
  let libraryResize: { x: number; width: number } | undefined;
  const maxLibraryWidth = () => Math.max(200, Math.min(600, window.innerWidth * 0.45));
  const resizeLibrary = (width: number) =>
    setLibraryWidth(Math.max(200, Math.min(maxLibraryWidth(), width)));
  const [helpOpen, setHelpOpen] = createSignal(false);
  const [editingPath, setEditingPath] = createSignal(false);
  const [missionName, setMissionName] = createSignal("");
  const availableMissions = () =>
    missionsForMap(props.index(), doc()?.sourceMap ?? doc()?.map ?? mapName());
  const [missionInfo, setMissionInfo] = createSignal("");
  const [populationPlaying, setPopulationPlaying] = createSignal(true);
  const [populationRoutes, setPopulationRoutes] = createSignal(false);
  const [perspective, setPerspective] = createSignal(0);
  const [rotationSnap, setRotationSnap] = createSignal(false);
  const [spriteOrientationLock, setSpriteOrientationLock] = createSignal(true);
  const [showEntities, setShowEntities] = createSignal(true);
  const [smoothTextures, setSmoothTextures] = createSignal(
    localStorage.getItem("rle.smoothTextures") !== "false",
  );
  const [synthesizedTextures, setSynthesizedTextures] = createSignal(
    localStorage.getItem("rle.synthesizedTextures") !== "false",
  );
  const [patchPreviewRevision, setPatchPreviewRevision] = createSignal(0);
  let openAttempt = 0;
  let mapLoadAbort: AbortController | undefined;
  let loadedIndex: DatadirIndex | null = null;
  let loadedLibrary: LibraryRef | null = null;
  let transientMapName: string | null = null;
  const [maps, setMaps] = createSignal<string[]>([]);
  const [mapLabels, setMapLabels] = createSignal<ReadonlyMap<string, string>>(new Map());
  const mapLabel = (name: string) => mapLabels().get(name) ?? name;
  const [assetEntries, setAssetEntries] = createSignal<ProjectionAssetEntry[]>([]);
  const [libraryLoading, setLibraryLoading] = createSignal(false);
  const [libraryError, setLibraryError] = createSignal("");
  const [mapLoadProgress, setMapLoadProgress] = createSignal<{
    completed: number;
    total: number;
    phase: string;
  } | null>(null);
  const [dropActive, setDropActive] = createSignal(false);
  const [addingAsset, setAddingAsset] = createSignal(false);
  let paletteAttempt = 0;
  const [revision, setRevision] = createSignal<SessionSnapshot<Level3D> | null>(null);
  const mapName = createMemo(() => revision()?.name ?? null);
  const doc = createMemo(() => revision()?.document ?? null);
  const dirty = createMemo(() => revision()?.dirty ?? false);
  const history = createMemo(() => revision() ?? { past: [], future: [] });
  const canInsert = createMemo(() => !!doc() && !addingAsset());
  const [transformBaseline, setTransformBaseline] = createSignal<Level3D | null>(null);
  const session = new SessionPublication<Level3D, FileSystemDirectoryHandle>((snapshot, reason) => {
    setRevision(snapshot);
    if (reason === "load") setTransformBaseline(snapshot.document);
    if (reason === "saved") setTransformBaseline(session.current!.saved);
    if (reason === "revision") {
      viewport.syncViews(snapshot.document, false);
      setMissionName(snapshot.document.mission?.importedFrom ?? "");
      const mission = snapshot.document.mission;
      setMissionInfo(
        mission
          ? `${mission.spawnPoints.length} editable PC spawns, ${mission.soldiers.length} editable soldiers. Import limitations are listed in Mission.`
          : "",
      );
    }
  });
  let saving = false;
  const [compiling, setCompiling] = createSignal(false);
  const [exportProgress, setExportProgress] = createSignal<BakeProgress | null>(null);
  let exportWorker: MapExportWorker | undefined;
  let exportCancelled = false;
  function cancelExport() {
    exportCancelled = true;
    exportWorker?.dispose();
  }
  onCleanup(cancelExport);
  let disposed = false;
  const [selected, setSelected] = createSignal<Selection>(null);
  const [revealSelectionInList, setRevealSelectionInList] = createSignal(false);
  const [filter, setFilter] = createSignal("");
  const [expanded, setExpanded] = createSignal<Set<string>>(new Set());
  const [showObstacles, setShowObstacles] = createSignal(false);
  const [showElevation, setShowElevation] = createSignal(false);
  const [gizmoVertical, setGizmoVertical] = createSignal(false);
  const [coordinateRotation, setCoordinateRotation] = createSignal(45);
  const [level, setLevel] = createSignal<ProtoLevel | null>(null);
  /** obstacle index -> suggested snap (Δ along the view ray, support obstacle) */
  const [suspects, setSuspects] = createSignal<Map<number, { delta: number; support: number }>>(
    new Map(),
  );
  const [info, setInfo] = createSignal<string | null>(null);
  const viewport = new EditorViewport({
    document: doc,
    selection: selected,
    level,
    showObstacles,
    showElevation,
    onSelection: (selection, revealInList) => {
      setRevealSelectionInList(revealInList);
      setSelected(selection);
      if (selection?.kind === "part") {
        const group = doc()?.objects.find((o) => o.id === selection.id)?.group;
        if (group) setExpanded((current) => new Set(current).add(group));
      }
    },
    commitTransform: setTransform,
    onError: props.onError,
  });
  const select = (selection: Selection, revealInList = true) =>
    viewport.select(selection, revealInList);
  createEffect(
    () => ({ selection: selected(), document: doc() }),
    ({ selection, document }) => {
      if (disposed) return;
      const exists =
        selection &&
        (selection.kind === "group" ? document?.groups : document?.objects)?.some(
          (object) => object.id === selection.id,
        );
      if (selection && !exists) setSelected(null);
      viewport.syncSelection(exists ? selection : null);
    },
  );

  // ── scenes in the library ──
  createEffect(
    () => props.library(),
    (lib) => {
      viewport.setSceneryLibrary(lib?.handle ?? null);
      session.beginLoad();
      mapLoadAbort?.abort();
      openAttempt++;
      setMaps([]);
      transientMapName = null;
      setMapLabels(new Map());
      if (!lib) return;
      void (async () => {
        const dir = await subdir(lib.handle, ["scenes"]);
        if (!dir) return;
        const files = await listFiles(dir);
        const labels = (await lib.mapLabels?.()) ?? new Map<string, string>();
        const names = files
          .filter((f) => f.endsWith(".rhlos-map.json"))
          .map((f) => f.slice(0, -".rhlos-map.json".length))
          .sort();
        if (disposed || props.library() !== lib) return;
        setMaps(names);
        setMapLabels(labels);
      })().catch((error) => {
        if (!disposed && props.library() === lib) props.onError(String(error));
      });
    },
  );

  createEffect(
    () => props.library(),
    (library) => {
      const attempt = ++paletteAttempt;
      setAssetEntries([]);
      setLibraryError("");
      setLibraryLoading(!!library);
      if (!library) return;
      void listProjectionAssets(library.handle)
        .then((entries) => {
          if (!disposed && attempt === paletteAttempt && props.library() === library)
            setAssetEntries(entries);
        })
        .catch((error) => {
          if (!disposed && attempt === paletteAttempt) setLibraryError(String(error));
        })
        .finally(() => {
          if (!disposed && attempt === paletteAttempt) setLibraryLoading(false);
        });
    },
  );

  function viewportElementCenter() {
    const bounds = viewportElement.getBoundingClientRect();
    return { x: bounds.left + bounds.width / 2, y: bounds.top + bounds.height / 2 };
  }

  type PreparedAsset = Awaited<ReturnType<typeof prepareProjectionPlacement>>;
  type WarmAsset = {
    id: string;
    library: LibraryRef;
    map: string;
    retired: boolean;
    value: PreparedAsset | null;
    promise: Promise<PreparedAsset>;
  };
  let warmAsset: WarmAsset | null = null;
  function clearWarmAsset() {
    const old = warmAsset;
    warmAsset = null;
    if (old) {
      old.retired = true;
      if (old.value) disposeObjectResources([old.value.asset]);
      old.value = null;
    }
  }
  function preloadAsset(entry: ProjectionAssetEntry) {
    const document = doc(),
      library = props.library();
    if (!document || !library) return null;
    if (
      warmAsset?.id === entry.id &&
      warmAsset.library === library &&
      warmAsset.map === document.map
    )
      return warmAsset;
    clearWarmAsset();
    const next: WarmAsset = {
      id: entry.id,
      library,
      map: document.map,
      retired: false,
      value: null,
      promise: prepareProjectionPlacement(library.handle, entry, document.map).then((value) => {
        if (next.retired) {
          disposeObjectResources([value.asset]);
          throw new Error("Asset preload cancelled");
        }
        next.value = value;
        return value;
      }),
    };
    // Hover failures are reported if the user actually attempts placement.
    void next.promise.catch(() => {});
    return (warmAsset = next);
  }
  async function takeAsset(entry: ProjectionAssetEntry) {
    const pending = preloadAsset(entry);
    if (!pending) throw new Error("Open a map before placing assets");
    const result = await pending.promise;
    pending.value = null;
    if (warmAsset === pending) warmAsset = null;
    return result;
  }
  type AssetDrag = {
    entry: ProjectionAssetEntry;
    base: Level3D;
    attempt: number;
    inside: boolean;
    dropped: boolean;
    position: [number, number, number] | null;
    elevationOffset: number;
    result: ReturnType<typeof insertProjectionAsset> | null;
  };
  let assetDrag: AssetDrag | null = null;
  function hideAssetDrag() {
    if (assetDrag) {
      assetDrag.inside = false;
      if (assetDrag.result && doc()) viewport.syncViews(doc()!, false);
    }
    setDropActive(false);
  }
  function cancelAssetDrag() {
    hideAssetDrag();
    assetDrag = null;
    clearWarmAsset();
  }
  function updateAssetDrag() {
    const drag = assetDrag;
    if (!drag || !drag.result || !drag.position || !drag.inside) return;
    if (doc() !== drag.base || openAttempt !== drag.attempt) {
      cancelAssetDrag();
      return;
    }
    const group = drag.result.document.groups.find(
      (group) => group.id === drag.result!.selection.id,
    )!;
    [group.transform.dx, group.transform.dy, group.transform.dz] = drag.position;
    group.transform.dz += drag.elevationOffset;
    viewport.syncViews(drag.result.document, false);
    if (drag.dropped) {
      assetDrag = null;
      pushHistory(drag.result.document);
      select(drag.result.selection);
      setInfo(`Added ${drag.entry.name}`);
      setDropActive(false);
    }
  }
  async function startAssetDrag(entry: ProjectionAssetEntry) {
    if (!doc() || addingAsset()) return;
    if (assetDrag) cancelAssetDrag();
    const drag: AssetDrag = {
      entry,
      base: doc()!,
      attempt: openAttempt,
      inside: false,
      dropped: false,
      position: null,
      elevationOffset: 0,
      result: null,
    };
    assetDrag = drag;
    let prepared: PreparedAsset | null = null;
    try {
      prepared = await takeAsset(entry);
      if (disposed || assetDrag !== drag || doc() !== drag.base || openAttempt !== drag.attempt)
        return;
      drag.result = insertProjectionAsset(
        drag.base,
        prepared.descriptor,
        prepared.reference,
        [0, 0, 0],
        prepared.additionalAssets,
      );
      drag.elevationOffset = drag.result.document.groups.find(
        (group) => group.id === drag.result!.selection.id,
      )!.transform.dz;
      parseLevel3D(drag.result.document, { level: level() ?? undefined });
      if (
        !viewport.adoptAsset(
          prepared.reference,
          prepared.asset,
          prepared.sources,
          prepared.additionalAssets.map((member) => member.reference),
        )
      )
        disposeObjectResources([prepared.asset]);
      prepared = null;
      updateAssetDrag();
    } catch (error) {
      if (!disposed && assetDrag === drag) {
        cancelAssetDrag();
        props.onError(String(error));
      }
    } finally {
      if (prepared) disposeObjectResources([prepared.asset]);
    }
  }

  async function addAsset(entry: ProjectionAssetEntry, placement?: [number, number, number]) {
    const document = doc();
    const library = props.library();
    const attempt = openAttempt;
    if (!document || !library || addingAsset()) return;
    setAddingAsset(true);
    let prepared: PreparedAsset | null = null;
    try {
      prepared = await takeAsset(entry);
      if (disposed || attempt !== openAttempt || props.library() !== library || doc() !== document)
        return;
      const center = viewportElementCenter();
      const position = placement ?? viewport.assetDropPosition(center.x, center.y);
      if (!position) throw new Error("Point the camera toward the ground before adding an asset.");
      const result = insertProjectionAsset(
        document,
        prepared.descriptor,
        prepared.reference,
        position,
        prepared.additionalAssets,
      );
      parseLevel3D(result.document, { level: level() ?? undefined });
      const adopted = viewport.adoptAsset(
        prepared.reference,
        prepared.asset,
        prepared.sources,
        prepared.additionalAssets.map((member) => member.reference),
      );
      if (!adopted) disposeObjectResources([prepared.asset]);
      prepared = null;
      pushHistory(result.document);
      select(result.selection);
      setInfo(`Added ${entry.name}`);
    } catch (error) {
      if (!disposed && attempt === openAttempt) props.onError(String(error));
    } finally {
      if (prepared) disposeObjectResources([prepared.asset]);
      if (!disposed) setAddingAsset(false);
    }
  }

  async function createMap(name: string) {
    const library = props.library();
    if (!library || creatingMap()) return;
    setCreatingMap(true);
    setNewMapError("");
    try {
      if (dirty()) {
        if (!window.confirm("Save your unsaved changes before creating a new map?")) return;
        await save();
        if (dirty())
          throw new Error("Save the current map successfully before creating another map.");
      }
      if (disposed || props.library() !== library) return;
      name = await createNewMap(library.handle, name, newMapOptions());
      if (disposed || props.library() !== library) return;
      setMaps((current) => [...new Set([...current, name])].sort());
      newMapDialog.close();
      setPanel("Assets");
      setViewSettings(false);
      await openMap(name);
    } catch (error) {
      if (!disposed) setNewMapError(error instanceof Error ? error.message : String(error));
    } finally {
      if (!disposed) setCreatingMap(false);
    }
  }

  // ── document ──
  function pushHistory(next: Level3D) {
    session.edit(next);
  }
  function commitTerrain(next: Level3D) {
    const previous = doc();
    pushHistory(previous ? followTerrainEdit(previous, next) : next);
  }
  function undo() {
    session.undo();
  }
  function redo() {
    session.redo();
  }
  function updatePart(id: string, patch: Partial<Level3DObject>) {
    const d = doc();
    if (!d) return;
    pushHistory(patchPart(d, id, patch));
  }
  function updateGroup(id: string, patch: Partial<Level3DGroup>) {
    const d = doc();
    if (!d) return;
    pushHistory(patchGroup(d, id, patch));
  }
  const selectedPart = () => {
    const s = selected();
    return s?.kind === "part" ? (doc()?.objects.find((o) => o.id === s.id) ?? null) : null;
  };
  const selectedPatchPreviews = () => {
    doc();
    patchPreviewRevision();
    return viewport.patchPreviews(selected());
  };
  const selectedGroup = () => {
    const s = selected();
    return s?.kind === "group" ? (doc()?.groups.find((g) => g.id === s.id) ?? null) : null;
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

  const [managingMap, setManagingMap] = createSignal<string | null>(null);
  async function manageMap(name: string, action: "delete" | "rename") {
    const library = props.library();
    if (!library || managingMap() || library.isBuiltIn?.(name)) return;
    const next = action === "rename" ? window.prompt("Rename map", mapLabel(name)) : null;
    if (action === "rename" && (next === null || next === mapLabel(name))) return;
    if (
      action === "delete" &&
      !window.confirm(`Delete “${mapLabel(name)}”? This cannot be undone.`)
    )
      return;
    setManagingMap(name);
    try {
      if (action === "delete") await library.deleteMap!(name);
      else await library.renameMap!(name, next!);
      const directory = await subdir(library.handle, ["scenes"]);
      if (!directory) throw new Error("scenes/ missing");
      const files = await listFiles(directory);
      if (disposed || props.library() !== library) return;
      setMaps(
        files
          .filter((file) => file.endsWith(".rhlos-map.json"))
          .map((file) => file.slice(0, -".rhlos-map.json".length))
          .sort(),
      );
      setMapLabels((await library.mapLabels?.()) ?? new Map());
    } catch (error) {
      if (!disposed) props.onError(String(error));
    } finally {
      if (!disposed) setManagingMap(null);
    }
  }

  function cancelMapLoad() {
    mapLoadAbort?.abort();
    openAttempt++;
    session.beginLoad();
    setMapLoadProgress(null);
    props.onStatus(null);
  }

  function confirmDiscard() {
    return (
      (!dirty() && !editingPath()) ||
      window.confirm("This map has unsaved changes. Discard them and continue?")
    );
  }

  function closeMap() {
    if (!confirmDiscard()) return;
    cancelAssetDrag();
    mapLoadAbort?.abort();
    openAttempt++;
    session.close();
    setRevision(null);
    setTransformBaseline(null);
    viewport.clearMap();
    setLevel(null);
    setEditingPath(false);
    setMissionName("");
    setMissionInfo("");
    setInfo(null);
    setMapLoadProgress(null);
    props.onStatus(null);
    if (transientMapName) {
      const previous = transientMapName;
      setMaps((names) => names.filter((name) => name !== previous));
      transientMapName = null;
    }
  }

  async function openMap(name: string, requestedMission?: string, importedFile?: File) {
    cancelAssetDrag();
    const lib = props.library();
    const idx = props.index();
    if (!lib) return;
    mapLoadAbort?.abort();
    const loadAbort = new AbortController();
    mapLoadAbort = loadAbort;
    const attempt = ++openAttempt;
    const generation = session.beginLoad();
    const current = () =>
      !disposed && attempt === openAttempt && props.index() === idx && props.library() === lib;
    let preparedAsset: THREE.Object3D | null = null;
    let preparedEntities: SceneEntities | null = null;
    let importedMission: Level3D["mission"];
    props.onStatus(null);
    setMapLoadProgress({ completed: 0, total: 1, phase: "Reading map" });
    try {
      let importedDocument: unknown;
      if (importedFile) {
        importedDocument = JSON.parse(await importedFile.text());
        const id = (importedDocument as { map?: unknown }).map;
        if (typeof id !== "string" || !id || id === "." || id === ".." || /[\\/\0]/.test(id))
          throw new Error("Invalid map name in dropped JSON");
        name = lib.availableMapName
          ? await lib.availableMapName(id)
          : (lib.savedMapName?.(id) ?? id);
      }
      const mission = requestedMission && idx ? await readMission(idx, requestedMission) : null;
      if (mission) {
        const matching =
          (doc()?.sourceMap ?? doc()?.map)?.toLowerCase() === mission.map.toLowerCase()
            ? mapName()
            : maps().find((m) => m.toLowerCase() === mission.map.toLowerCase());
        if (!matching) throw new Error(`No published map for ${mission.map} in this library`);
        name = matching;
      }
      if (disposed || attempt !== openAttempt) return;
      const currentDocument = doc();
      const currentLevel = level();
      if (
        !importedFile &&
        name === mapName() &&
        currentDocument &&
        loadedIndex === idx &&
        loadedLibrary === lib
      ) {
        if (mission && idx && currentLevel) {
          importedMission = await loadEditableMission(idx, mission, currentLevel, lib.handle);
          preparedEntities = await MissionEntities.load(
            idx,
            remainingMissionPreview(mission),
            currentLevel,
            currentDocument.camera,
            current,
          ).catch((error) => {
            importedMission?.importWarnings?.push(
              `Other mission previews unavailable: ${String(error)}`,
            );
            return null;
          });
        } else if (mission) throw new Error("Mission import requires its source level data");
        else if (currentDocument.population && !currentDocument.mission)
          preparedEntities = await PopulationView.load(
            lib.handle,
            currentDocument.population,
            currentDocument.camera,
            current,
          );
        if (
          disposed ||
          attempt !== openAttempt ||
          props.index() !== idx ||
          props.library() !== lib
        ) {
          preparedEntities?.dispose();
          return;
        }
        if (doc() !== currentDocument)
          throw new Error("Map changed during mission import; please load the mission again.");
        if (importedMission) pushHistory({ ...currentDocument, mission: importedMission });
        else if (!mission && currentDocument.mission) {
          const { mission: _mission, ...mapOnly } = currentDocument;
          pushHistory(mapOnly);
        }
        viewport.replaceEntities(preparedEntities);
        viewport.setEntitiesVisible(showEntities());
        viewport.setPopulationPlaying(populationPlaying());
        viewport.setPopulationRoutesVisible(populationRoutes());
        setMissionName(mission?.name ?? "");
        setMissionInfo(
          importedMission
            ? `${importedMission.spawnPoints.length} editable PC spawns, ${importedMission.soldiers.length} editable soldiers. Import limitations are listed in Mission.`
            : preparedEntities
              ? `${preparedEntities.count} entities. ${preparedEntities.warnings.join("; ")}`
              : "",
        );
        preparedEntities = null;
        props.onStatus(null);
        setMapLoadProgress(null);
        return;
      }
      if (!confirmDiscard()) {
        props.onStatus(null);
        setMapLoadProgress(null);
        return;
      }
      const approvedDocument = doc();
      const candidate = await prepareMapCandidate(
        name,
        lib.handle,
        idx,
        (completed, total, phase) => {
          if (attempt === openAttempt) setMapLoadProgress({ completed, total, phase });
        },
        lib.documentMap?.(name) ?? name,
        importedDocument,
        loadAbort.signal,
      );
      preparedAsset = candidate.asset;
      if (mission && idx) {
        if (!candidate.level) throw new Error("Mission requires level data");
        importedMission = await loadEditableMission(idx, mission, candidate.level, lib.handle);
        preparedEntities = await MissionEntities.load(
          idx,
          remainingMissionPreview(mission),
          candidate.level,
          candidate.document.camera,
          current,
        ).catch((error) => {
          importedMission?.importWarnings?.push(
            `Other mission previews unavailable: ${String(error)}`,
          );
          return null;
        });
      } else if (candidate.document.population && !candidate.document.mission) {
        preparedEntities = await PopulationView.load(
          lib.handle,
          candidate.document.population,
          candidate.document.camera,
          current,
        );
      }
      const {
        document: baseDocument,
        directory: dir,
        level: lvl,
        sources: nextSources,
        ground: nextGround,
        suspects: nextSuspects,
      } = candidate;
      const d = importedMission ? { ...baseDocument, mission: importedMission } : baseDocument;
      if (
        disposed ||
        !session.isCurrent(generation) ||
        attempt !== openAttempt ||
        props.library() !== lib ||
        props.index() !== idx
      ) {
        preparedEntities?.dispose();
        preparedEntities = null;
        disposeObjectResources([preparedAsset]);
        preparedAsset = null;
        return;
      }
      if (doc() !== approvedDocument && !confirmDiscard()) {
        preparedEntities?.dispose();
        if (preparedAsset) disposeObjectResources([preparedAsset]);
        props.onStatus(null);
        setMapLoadProgress(null);
        return;
      }
      // All asynchronous reads and validation precede publication.
      viewport.replaceMap(preparedAsset, nextGround, nextSources, d.assetSources);
      preparedAsset = null;
      viewport.replaceEntities(preparedEntities);
      viewport.setEntitiesVisible(showEntities());
      viewport.setPopulationPlaying(populationPlaying());
      viewport.setPopulationRoutesVisible(populationRoutes());
      setMissionName(mission?.name ?? d.mission?.importedFrom ?? "");
      setMissionInfo(
        d.mission
          ? `${d.mission.spawnPoints.length} editable PC spawns, ${d.mission.soldiers.length} editable soldiers. Import limitations are listed in Mission.`
          : preparedEntities
            ? `${preparedEntities.count} entities. ${preparedEntities.warnings.join("; ")}`
            : "",
      );
      preparedEntities = null;
      if (transientMapName && transientMapName !== name) {
        const previous = transientMapName;
        setMaps((names) => names.filter((name) => name !== previous));
        transientMapName = null;
      }
      if (importedFile) {
        if (!maps().includes(name)) transientMapName = name;
        setMaps((names) => [...new Set([...names, name])].sort());
        const sourceName = lib.documentMap?.(name) ?? name;
        if (sourceName !== name)
          setMapLabels((labels) =>
            new Map(labels).set(
              name,
              `${labels.get(sourceName) ?? sourceName}${name.slice(sourceName.length)}`,
            ),
          );
      }
      session.publish(generation, name, baseDocument, dir, candidate.saved && !importedFile);
      if (importedMission) pushHistory(d);
      loadedIndex = idx;
      loadedLibrary = lib;
      setLevel(lvl);
      setSuspects(nextSuspects);
      viewport.syncViews(d);
      viewport.buildOverlays();
      viewport.gameCamera(true);
      setInfo(`${d.groups.length} buildings, ${d.objects.length} parts`);
      props.onStatus(
        candidate.warnings.length ? `Warning: ${candidate.warnings.join("\n")}` : null,
      );
      setMapLoadProgress(null);
    } catch (e) {
      preparedEntities?.dispose();
      if (preparedAsset) disposeObjectResources([preparedAsset]);
      if (session.isCurrent(generation) && !disposed) {
        props.onStatus(null);
        setMapLoadProgress(null);
        props.onError(String(e));
      }
    }
  }

  createEffect(
    () => props.index(),
    () => {
      mapLoadAbort?.abort();
      openAttempt++;
      session.beginLoad();
      viewport.replaceEntities(null);
      setMissionName("");
      setMissionInfo("");
    },
  );
  createEffect(
    () => perspective(),
    (value) => viewport.setPerspective(value),
  );
  createEffect(
    () => rotationSnap(),
    (value) => viewport.setRotationSnap(value),
  );
  createEffect(
    () => spriteOrientationLock(),
    (value) => viewport.setSpriteOrientationLock(value),
  );
  createEffect(
    () => assetDisplayMode(),
    (value) => viewport.setAssetDisplayMode(value),
  );
  createEffect(
    () => showEntities(),
    (value) => viewport.setEntitiesVisible(value),
  );
  createEffect(
    () => ({ smooth: smoothTextures(), synthesized: synthesizedTextures() }),
    (value) => {
      viewport.setTextureDisplay(value.smooth, value.synthesized);
      localStorage.setItem("rle.smoothTextures", String(value.smooth));
      localStorage.setItem("rle.synthesizedTextures", String(value.synthesized));
    },
  );

  createEffect(
    () => ({ obstacles: showObstacles(), elevation: showElevation() }),
    () => untrack(() => viewport.buildOverlays()),
  );
  createEffect(
    () => gizmoVertical(),
    (v) => viewport.setGizmoVertical(v),
  );

  createEffect(
    () => coordinateRotation(),
    (degrees) => viewport.setCoordinateRotation(degrees),
  );

  // ── actions ──
  const selectedStatePart = () => {
    const d = doc();
    const p = selectedPart();
    return d && p ? stateOwner(d, p.id) : undefined;
  };
  function duplicateSelected() {
    const document = doc();
    const selection = selected();
    if (!document || !selection || selectedStatePart()) return;
    const result = duplicateSelection(document, selection);
    pushHistory(result.document);
    select(result.selection);
  }
  function deleteSelected() {
    const document = doc();
    const selection = selected();
    if (!document || !selection || selectedStatePart()) return;
    const next = deleteSelection(document, selection);
    select(null);
    pushHistory(next);
  }
  function rotateSelected(delta: number) {
    const t = selectedTransform();
    if (!t) return;
    setTransform({ ...t, rot_deg: (((t.rot_deg + delta) % 360) + 360) % 360 });
  }
  function setTransformField(field: keyof GameTransform, value: number) {
    const t = selectedTransform();
    if (!t || !Number.isFinite(value)) return;
    const document = doc(),
      selection = selected();
    if (!document || !selection) return;
    const next = { ...t, [field]: value };
    setTransform(
      field === "dx" || field === "dy" ? followTerrainTransform(document, selection, next) : next,
    );
  }
  function previewTransformField(field: keyof GameTransform, value: number) {
    const document = doc(),
      transform = selectedTransform(),
      selection = selected();
    if (!document || !transform || !selection) return;
    const proposed = { ...transform, [field]: value };
    const changes = {
      transform:
        field === "dx" || field === "dy"
          ? followTerrainTransform(document, selection, proposed)
          : proposed,
    };
    const next =
      selection.kind === "group"
        ? patchGroup(document, selection.id, changes)
        : patchPart(document, selection.id, changes);
    viewport.syncViews(next, false);
  }
  function setHidden(hidden: boolean) {
    const g = selectedGroup();
    const p = selectedPart();
    if (g) updateGroup(g.id, { hidden });
    else if (p && !selectedStatePart()) updatePart(p.id, { hidden });
  }
  async function save() {
    if (!session.current || saving) return;
    const snapshot = session.captureSave();
    const library = props.library();
    saving = true;
    try {
      const thumbnail = viewport.captureThumbnail();
      const encodedThumbnail = await encodeMapThumbnail(thumbnail);
      const descriptors = snapshot.document.assetSources?.length
        ? await readPinnedAssetDescriptors(library!.handle, snapshot.document.assetSources)
        : new Map();
      const stored = serializeStoredMap(snapshot.document, descriptors);
      let savedName: string;
      if (library?.saveMap) {
        savedName = await library.saveMap(snapshot.name, stored, encodedThumbnail);
      } else {
        await writeText(
          snapshot.resources,
          `${snapshot.name}.rhlos-map.json`,
          JSON.stringify(stored, null, 2),
        );
        await writeMapThumbnail(snapshot.resources, snapshot.name, encodedThumbnail);
        savedName = library?.savedMapName?.(snapshot.name) ?? snapshot.name;
      }
      if (transientMapName === snapshot.name) transientMapName = null;
      const labels = await library?.mapLabels?.();
      if (!disposed && props.library() === library) {
        if (labels) setMapLabels(labels);
        setMaps((names) => [...new Set([...names, savedName])].sort());
      }
      session.saved(snapshot, savedName);
    } catch (e) {
      if (!disposed) props.onError(String(e));
    } finally {
      saving = false;
    }
  }

  async function downloadSavedMap(name: string) {
    try {
      const library = props.library();
      if (!library) throw new Error("Connect the library before downloading a saved map.");
      const directory = await subdir(library.handle, ["scenes"]);
      if (!directory) throw new Error("scenes/ missing");
      const file = await (await directory.getFileHandle(`${name}.rhlos-map.json`)).getFile();
      downloadMapFile(name, file);
    } catch (error) {
      props.onError(String(error));
    }
  }

  async function download() {
    const document = doc();
    if (!document) return;
    try {
      const descriptors = document.assetSources?.length
        ? await readPinnedAssetDescriptors(props.library()!.handle, document.assetSources)
        : new Map();
      downloadMap(document.map, serializeStoredMap(document, descriptors));
    } catch (error) {
      props.onError(String(error));
    }
  }

  async function exportMod() {
    const document = doc();
    if (!document || compiling()) return;
    setCompiling(true);
    exportCancelled = false;
    const progress = (value: BakeProgress) => {
      if (disposed || exportCancelled) throw new Error("Map export was cancelled.");
      setExportProgress(value);
    };
    progress({ stage: "Reading asset definitions…", completed: 0, total: 0 });
    props.onStatus("Exporting map…", true);
    try {
      // Let the busy state paint before borrowing the viewport's GPU resources.
      await new Promise((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));
      const library = props.library();
      if (!library) throw new Error("Connect the asset library before compiling.");
      const assets = await readPinnedAssetDescriptors(
        library.handle,
        document.assetSources ?? [],
        document.sceneAssets,
      );
      progress({ stage: "Compiling gameplay and connections…", completed: 0, total: 0 });
      const worker = new MapExportWorker();
      exportWorker = worker;
      const { compiled, pixels, appearance } = await viewport.bakeMapAsync(
        document,
        assets,
        progress,
        (bounds, preparedAssets) => worker.compile(document, bounds, preparedAssets ?? assets),
      );
      const { collectSceneryResources } = await import("./scenery-resources.ts");
      const scenery = await collectSceneryResources(
        compiled,
        assets,
        async (path) =>
          new Uint8Array(await (await libraryFile(library.handle, path)).arrayBuffer()),
        (completed, total) => progress({ stage: "Reading scenery sprites…", completed, total }),
      );
      progress({ stage: "Encoding images and packaging ZIP…", completed: 0, total: 0 });
      const bytes = await worker.package(compiled, pixels, appearance, scenery);
      if (disposed) return;
      const url = URL.createObjectURL(
        new Blob([new Uint8Array(bytes)], { type: "application/zip" }),
      );
      const link = window.document.createElement("a");
      link.href = url;
      link.download = `${compiled.name}.zip`;
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
      props.onStatus(
        `Exported ${compiled.name}.zip with ${compiled.warnings.length} warnings; see compile-report.json in the ZIP for omitted or incomplete gameplay. Put it in the game’s configured mods directory, then choose ${compiled.details.title} in Custom Missions. The editable map is included.`,
      );
    } catch (error) {
      if (!disposed) {
        props.onStatus(null);
        if (!exportCancelled) props.onError(`Map compilation failed: ${String(error)}`);
      }
    } finally {
      exportWorker?.dispose();
      exportWorker = undefined;
      if (!disposed) {
        setCompiling(false);
        setExportProgress(null);
      }
    }
  }

  function onKey(e: KeyboardEvent) {
    if (!doc() || window.document.querySelector("dialog[open]")) return;
    if (["INPUT", "SELECT", "TEXTAREA"].includes((e.target as HTMLElement).tagName)) return;
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
    } else if (e.key === "g") viewport.gameCamera();
    else if (e.key === "f") viewport.frameContent();
    else if (panel() === "Assets") {
      if (e.key === "Delete" || e.key === "Backspace") deleteSelected();
      else if (e.key === "d" && !e.ctrlKey) duplicateSelected();
      else if (e.key === "q") rotateSelected(-15);
      else if (e.key === "e") rotateSelected(15);
      else if (e.key === "Escape") select(null);
    }
  }
  window.addEventListener("keydown", onKey, {
    signal: viewport.listeners.signal,
  });
  const warnBeforeUnload = (event: BeforeUnloadEvent) => {
    if (!dirty() && !editingPath()) return;
    event.preventDefault();
    event.returnValue = "";
  };
  createEffect(
    () => dirty() || editingPath(),
    (unsaved) => {
      window.removeEventListener("beforeunload", warnBeforeUnload);
      if (unsaved) window.addEventListener("beforeunload", warnBeforeUnload);
    },
  );
  onCleanup(() => window.removeEventListener("beforeunload", warnBeforeUnload));
  onCleanup(() => {
    cancelAssetDrag();
    disposed = true;
    mapLoadAbort?.abort();
    session.dispose();
    viewport.dispose();
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
  const baselineTransforms = createMemo(() => {
    const baseline = transformBaseline();
    return {
      groups: new Map(baseline?.groups.map((g) => [g.id, g.transform])),
      parts: new Map(baseline?.objects.map((p) => [p.id, p.transform])),
    };
  });
  function transformMoved(current: GameTransform, baseline: GameTransform | undefined) {
    // Newly inserted items have no saved placement to have moved away from.
    return (
      !!baseline &&
      (current.dx !== baseline.dx ||
        current.dy !== baseline.dy ||
        current.dz !== baseline.dz ||
        current.rot_deg !== baseline.rot_deg)
    );
  }
  const rows = (): Row[] => {
    const d = doc();
    if (!d) return [];
    const q = filter().toLowerCase();
    const match = (id: string, name?: string) =>
      !q || id.toLowerCase().includes(q) || (name ?? "").toLowerCase().includes(q);
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
        moved: transformMoved(g.transform, baselineTransforms().groups.get(g.id)),
        parts: parts.length,
        suspect: parts.some(
          (p) => p.source.obstacle !== undefined && suspects().has(p.source.obstacle),
        ),
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
              moved: transformMoved(p.transform, baselineTransforms().parts.get(p.id)),
              suspect: p.source.obstacle !== undefined && suspects().has(p.source.obstacle),
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
        moved: transformMoved(o.transform, baselineTransforms().parts.get(o.id)),
        suspect: o.source.obstacle !== undefined && suspects().has(o.source.obstacle),
      });
    }
    return out;
  };
  let sceneObjectList: HTMLUListElement | undefined;
  createEffect(
    () => ({
      selection: selected(),
      revealInList: revealSelectionInList(),
      panel: panel(),
      expanded: expanded(),
      filter: filter(),
    }),
    ({ selection, revealInList, panel }) => {
      // List selections are already visible; keep the user's scroll position.
      if (!selection || !revealInList || panel !== "Assets") return undefined;
      // Wait for the selected row and any expanded parent to finish rendering.
      const frame = requestAnimationFrame(() => {
        const list = sceneObjectList;
        const row = list?.querySelector<HTMLElement>('li[aria-pressed="true"]');
        if (!list || !row) return;
        const bounds = list.getBoundingClientRect();
        const item = row.getBoundingClientRect();
        const top = bounds.top + list.clientTop;
        const bottom = top + list.clientHeight;
        if (item.top < top) list.scrollTop += item.top - top;
        else if (item.bottom > bottom) list.scrollTop += item.bottom - bottom;
      });
      return () => cancelAnimationFrame(frame);
    },
  );
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
    if (g && d) return `${g.name ?? g.id} (${groupParts(d, g.id).length} parts)`;
    if (p) return p.name ?? p.id;
    return "";
  };

  return (
    <div class="editor">
      <dialog
        class="new-map-dialog"
        ref={(element) => {
          newMapDialog = element;
        }}
        aria-labelledby="new-map-title"
        onCancel={(event) => {
          if (creatingMap()) event.preventDefault();
        }}
      >
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void createMap(String(new FormData(event.currentTarget).get("mapName")));
          }}
        >
          <h2 id="new-map-title">New map</h2>
          <p>
            Choose a reference size and starting ground grid. Resize later without deleting content.
          </p>
          <fieldset disabled={creatingMap()}>
            <label>
              Map name
              <input
                name="mapName"
                aria-label="Map name"
                autofocus
                required
                maxlength={64}
                value={newMapName()}
                onInput={(event) => setNewMapName(event.currentTarget.value)}
              />
            </label>
            <NewMapSettings value={newMapOptions()} onChange={setNewMapOptions} />
            <p class="hint">Saved in this browser. Use Download to export the map JSON.</p>
            <Show when={dirty()}>
              <p class="hint">Your current map will be saved before creating the new one.</p>
            </Show>
            <Show when={newMapError()}>
              <p class="library-error" role="alert">
                {newMapError()}
              </p>
            </Show>
            <div class="dialog-actions">
              <button type="button" onClick={() => newMapDialog.close()}>
                Cancel
              </button>
              <button type="submit" class="primary-action">
                {creatingMap() ? "Creating…" : "Create map"}
              </button>
            </div>
          </fieldset>
        </form>
      </dialog>
      <header class="topbar editor-bar">
        {props.toolbarStart?.()}
        <Show when={mapName()}>
          <span class="active-map-name" data-map-name={mapName()}>
            {mapLabel(mapName()!)}
          </span>
        </Show>

        <Show when={doc()}>
          <nav class="editor-modes" aria-label="Editor mode">
            <For each={["Assets", "Paths", "Terrain", "Mission"]}>
              {(name) => (
                <button
                  aria-pressed={panel() === name ? "true" : "false"}
                  class={panel() === name ? "selected" : ""}
                  disabled={editingPath() && name !== "Paths"}
                  title={
                    editingPath() && name !== "Paths"
                      ? "Finish or cancel the path before switching tools"
                      : undefined
                  }
                  onClick={() => {
                    setPanel(name);
                    setViewSettings(false);
                  }}
                >
                  {name}
                </button>
              )}
            </For>
          </nav>
          <button
            aria-pressed={viewSettings() ? "true" : "false"}
            onClick={() => setViewSettings(!viewSettings())}
          >
            View settings
          </button>
        </Show>
        <span class="spacer" />
        <Show when={doc()}>
          <button disabled={history().past.length === 0} onClick={undo} title="ctrl+z">
            Undo
          </button>
          <button disabled={history().future.length === 0} onClick={redo} title="ctrl+shift+z">
            Redo
          </button>
          <button
            class="primary-action"
            disabled={!dirty() || editingPath()}
            onClick={() => void save()}
            title="ctrl+s"
          >
            Save{dirty() ? " *" : ""}
          </button>
          <button disabled={!doc()} onClick={() => void download()}>
            Download
          </button>
          <button
            disabled={
              !doc() || compiling() || editingPath() || addingAsset() || !!mapLoadProgress()
            }
            onClick={() => void exportMod()}
            title="Compile geometry and connections from placed asset definitions"
          >
            {compiling() ? "Compiling…" : "Export mod ZIP"}
          </button>
          <button
            aria-expanded={helpOpen() ? "true" : "false"}
            aria-controls="editor-help"
            onClick={() => setHelpOpen(!helpOpen())}
          >
            Help
          </button>
        </Show>
        {props.toolbarEnd?.()}
        <Show when={doc()}>
          <button class="close-map" aria-label="Close map" title="Close map" onClick={closeMap}>
            ×
          </button>
        </Show>
      </header>
      <Show when={exportProgress()}>
        {(progress) => (
          <dialog
            class="map-load-dialog"
            aria-label="Exporting map"
            ref={(dialog) =>
              queueMicrotask(() => {
                if (dialog.isConnected) dialog.showModal();
              })
            }
            onCancel={(event) => {
              event.preventDefault();
              cancelExport();
            }}
          >
            <h2>Exporting map</h2>
            <div class="map-load-progress" role="status" aria-live="polite">
              <div class="map-load-progress-label">
                <span>{progress().stage}</span>
                <span>
                  {progress().total > 0
                    ? `${Math.round((progress().completed / progress().total) * 100)}%`
                    : ""}
                </span>
              </div>
              <progress
                max={Math.max(1, progress().total)}
                value={progress().total > 0 ? progress().completed : undefined}
              />
            </div>
            <button onClick={cancelExport}>Cancel export</button>
          </dialog>
        )}
      </Show>
      <Show when={mapLoadProgress()}>
        {(progress) => (
          <dialog
            class="map-load-dialog"
            aria-label="Loading map"
            ref={(dialog) =>
              queueMicrotask(() => {
                if (dialog.isConnected) dialog.showModal();
              })
            }
            onCancel={(event) => {
              event.preventDefault();
              cancelMapLoad();
            }}
          >
            <h2>Loading map</h2>
            <div class="map-load-progress" role="status" aria-live="polite">
              <div class="map-load-progress-label">
                <span>{progress().phase}</span>
                <span>
                  {progress().total > 1
                    ? `${progress().completed} / ${progress().total} assets`
                    : ""}
                </span>
              </div>
              <progress max={Math.max(1, progress().total)} value={progress().completed} />
            </div>
            <button onClick={cancelMapLoad}>Cancel</button>
          </dialog>
        )}
      </Show>
      <div class={`editor-body${doc() ? "" : " selecting-map"}`}>
        <div
          id="asset-browser"
          class={`asset-browser${libraryOpen() ? "" : " collapsed"}`}
          style={{ width: libraryOpen() ? `${libraryWidth()}px` : "44px" }}
        >
          <div class="asset-library-host" hidden={panel() !== "Assets"}>
            <AssetLibrary
              renderer={previewRenderer}
              root={props.library()?.handle ?? null}
              entries={assetEntries()}
              collapsed={!libraryOpen()}
              onToggle={() => setLibraryOpen(!libraryOpen())}
              onPreload={(entry) => {
                preloadAsset(entry);
              }}
              onDragStart={(entry) => {
                void startAssetDrag(entry);
              }}
              onDragReturn={hideAssetDrag}
              loading={libraryLoading()}
              error={libraryError()}
              canInsert={canInsert()}
              onAdd={(entry) => void addAsset(entry)}
              onDragEnd={() => {
                if (!assetDrag?.dropped) cancelAssetDrag();
              }}
            />
          </div>
          <section class="shared-library mode-library" hidden={panel() === "Assets"}>
            <header class="library-heading">
              <button
                onClick={() => setLibraryOpen(!libraryOpen())}
                aria-expanded={libraryOpen() ? "true" : "false"}
                aria-controls="mode-library-content"
                aria-label={libraryOpen() ? "Hide library" : "Show library"}
              >
                {libraryOpen() ? "←" : "→"}
              </button>
              <h2>
                {panel() === "Terrain"
                  ? "Terrain materials"
                  : panel() === "Mission"
                    ? "Characters"
                    : "Paths & walls"}
              </h2>
            </header>
            <div id="mode-library-content" class="mode-library-content" hidden={!libraryOpen()}>
              <Show when={doc() && panel() === "Mission" && availableMissions().length > 0}>
                <label class="mission-picker">
                  Mission
                  <select
                    aria-label="Mission"
                    value={missionName()}
                    onChange={(e) => {
                      const value = e.currentTarget.value;
                      e.currentTarget.value = missionName();
                      if (value) void openMap("", value);
                      else if (mapName()) void openMap(mapName()!);
                    }}
                  >
                    <option value="">Map only</option>
                    <For each={availableMissions()}>
                      {(mission) => <option value={mission.id}>{mission.label}</option>}
                    </For>
                  </select>
                </label>
              </Show>
              <div ref={setLibraryMount} />
            </div>
          </section>
          <div
            class="library-resizer"
            hidden={!libraryOpen()}
            role="separator"
            tabindex={0}
            aria-label="Resize library"
            aria-orientation="vertical"
            aria-valuemin={200}
            aria-valuemax={maxLibraryWidth()}
            aria-valuenow={libraryWidth()}
            onPointerDown={(event) => {
              if (event.button !== 0) return;
              event.preventDefault();
              libraryResize = {
                x: event.clientX,
                width: event.currentTarget.parentElement!.getBoundingClientRect().width,
              };
              event.currentTarget.setPointerCapture(event.pointerId);
            }}
            onPointerMove={(event) => {
              if (libraryResize)
                resizeLibrary(libraryResize.width + event.clientX - libraryResize.x);
            }}
            onPointerUp={(event) => {
              libraryResize = undefined;
              event.currentTarget.releasePointerCapture(event.pointerId);
            }}
            onLostPointerCapture={() => {
              libraryResize = undefined;
            }}
            onKeyDown={(event) => {
              if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
              event.preventDefault();
              resizeLibrary(
                event.key === "Home"
                  ? 200
                  : event.key === "End"
                    ? maxLibraryWidth()
                    : libraryWidth() + (event.key === "ArrowLeft" ? -16 : 16),
              );
            }}
          />
        </div>
        <div
          class={`editor-canvas ${dropActive() ? "asset-drop-active" : ""}`}
          ref={(element) => {
            viewportElement = element;
            untrack(() => viewport.setup(element));
          }}
          onDragOver={(event) => {
            if (event.dataTransfer?.types.includes("Files")) {
              event.preventDefault();
              event.dataTransfer.dropEffect = "copy";
              setDropActive(true);
              return;
            }
            if (!doc() || addingAsset() || !event.dataTransfer?.types.includes(ASSET_DRAG_TYPE))
              return;
            event.preventDefault();
            event.dataTransfer.dropEffect = "copy";
            setDropActive(true);
            if (assetDrag) {
              assetDrag.position = viewport.assetDropPosition(event.clientX, event.clientY);
              assetDrag.inside = !!assetDrag.position;
              updateAssetDrag();
            }
          }}
          onDragLeave={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget as Node | null)) hideAssetDrag();
          }}
          onDrop={(event) => {
            setDropActive(false);
            if (event.dataTransfer?.types.includes("Files")) {
              event.preventDefault();
              const files = [...event.dataTransfer.files];
              if (files.length !== 1 || !files[0]!.name.toLowerCase().endsWith(".json")) {
                props.onError("Drop one map JSON file onto the viewport.");
              } else if (!props.library()) {
                props.onError("Wait for the asset library to load before importing a map.");
              } else {
                void openMap(files[0]!.name, undefined, files[0]);
              }
              return;
            }
            const id = event.dataTransfer?.getData(ASSET_DRAG_TYPE);
            if (!id) return;
            event.preventDefault();
            const entry =
              (assetDrag?.entry.id === id ? assetDrag.entry : undefined) ??
              assetEntries().find((entry) => entry.id === id);
            const placement = viewport.assetDropPosition(event.clientX, event.clientY);
            if (assetDrag && entry?.id === assetDrag.entry.id && placement) {
              assetDrag.position = placement;
              assetDrag.inside = true;
              assetDrag.dropped = true;
              updateAssetDrag();
            } else if (entry && placement) void addAsset(entry, placement);
          }}
        >
          <Show when={doc()}>
            <div class="viewport-navigation" aria-label="Quick camera controls">
              <button onClick={() => viewport.gameCamera()} title="Game camera (g)">
                Game camera
              </button>
              <For each={["N", "E", "S", "W"] as const}>
                {(direction) => (
                  <button
                    title={`Orient ${direction} up`}
                    onClick={() => viewport.setCardinalView(direction)}
                  >
                    {direction}
                  </button>
                )}
              </For>
              <button onClick={() => viewport.topView()}>Top</button>
              <button
                aria-label="Turn camera left 90 degrees"
                onClick={() => viewport.rotateViewQuarterTurn(-1)}
              >
                ↶
              </button>
              <button
                aria-label="Turn camera right 90 degrees"
                onClick={() => viewport.rotateViewQuarterTurn(1)}
              >
                ↷
              </button>
            </div>
          </Show>
          <Show when={!doc()}>
            <section class="map-selection" aria-label="Select Map">
              <span class="eyebrow">MAP WORKSPACE</span>
              <div class="map-selection-heading">
                <h2>Select Map</h2>
                <button
                  disabled={!props.library() || editingPath()}
                  title={!props.library() ? "Waiting for assets" : "Create a blank map"}
                  onClick={() => {
                    setNewMapError("");
                    newMapDialog.showModal();
                  }}
                >
                  New map
                </button>
              </div>
              <p>
                {!props.library()
                  ? "Loading maps…"
                  : maps().length
                    ? "I've marked the maps where I've done 3D model refinement. The ones marked WIP are pure projection maps."
                    : "No maps yet. Create a new map to begin."}
              </p>
              <div class="map-grid">
                <For each={maps()}>
                  {(name) => (
                    <MapCard
                      name={name}
                      label={mapLabel(name)}
                      library={props.library()!}
                      onOpen={() => void openMap(name)}
                      onDownload={() => void downloadSavedMap(name)}
                      disabled={!!managingMap()}
                      onDelete={
                        props.library()?.deleteMap && !props.library()?.isBuiltIn?.(name)
                          ? () => void manageMap(name, "delete")
                          : undefined
                      }
                      onRename={
                        props.library()?.renameMap && !props.library()?.isBuiltIn?.(name)
                          ? () => void manageMap(name, "rename")
                          : undefined
                      }
                    />
                  )}
                </For>
              </div>
            </section>
          </Show>
          <Show when={helpOpen() && doc()}>
            <EditorHelp
              mode={panel()}
              hasMissionLoader={availableMissions().length > 0}
              onClose={() => setHelpOpen(false)}
            />
          </Show>
        </div>
        <aside class="editor-panel" aria-label="Inspector">
          <div class="inspector-content" hidden={panel() !== "Mission" || viewSettings()}>
            <MissionPanel
              libraryMount={libraryMount()}
              library={() => props.library()?.handle ?? null}
              document={doc}
              commit={pushHistory}
              onError={props.onError}
              active={panel() === "Mission"}
              viewport={viewport}
            />
          </div>
          <div class="inspector-content" hidden={panel() !== "Terrain" || viewSettings()}>
            <TerrainPanel
              libraryMount={libraryMount()}
              viewport={viewport}
              active={panel() === "Terrain"}
              document={doc}
              commit={commitTerrain}
              onError={props.onError}
              disabled={editingPath()}
            />
          </div>
          <div class="inspector-content" hidden={panel() !== "Paths" || viewSettings()}>
            <SplinePanel
              previewRenderer={previewRenderer}
              libraryMount={libraryMount()}
              document={doc}
              library={() => props.library()?.handle ?? null}
              entries={assetEntries}
              viewport={viewport}
              commit={commitTerrain}
              onError={props.onError}
              active={panel() === "Paths"}
              onEditingChange={setEditingPath}
            />
          </div>
          <div class="inspector-content" hidden={!viewSettings()}>
            <section class="view-settings">
              <h2>Camera &amp; display</h2>
              <div class="camera-directions" aria-label="Camera direction">
                <button onClick={() => viewport.gameCamera()} title="Game camera (g)">
                  Game camera
                </button>
                <For each={["N", "E", "S", "W"] as const}>
                  {(direction) => (
                    <button onClick={() => viewport.setCardinalView(direction)}>{direction}</button>
                  )}
                </For>
                <button onClick={() => viewport.topView()}>Top view</button>
                <button
                  aria-label="Rotate view left 90 degrees"
                  onClick={() => viewport.rotateViewQuarterTurn(-1)}
                >
                  ↶ 90°
                </button>
                <button
                  aria-label="Rotate view right 90 degrees"
                  onClick={() => viewport.rotateViewQuarterTurn(1)}
                >
                  ↷ 90°
                </button>
              </div>
              <label>
                Asset display
                <select
                  aria-label="Asset display"
                  value={assetDisplayMode()}
                  onChange={(event) =>
                    setAssetDisplayMode(
                      event.currentTarget.value as "visible" | "outline" | "hidden",
                    )
                  }
                >
                  <option value="visible">Visible</option>
                  <option value="outline">Outline</option>
                  <option value="hidden">Hidden</option>
                </select>
              </label>
              <ScrubNumber
                label="Gizmo rotation (°)"
                step={1}
                value={coordinateRotation()}
                onPreview={(degrees) => viewport.setCoordinateRotation(degrees)}
                onCommit={setCoordinateRotation}
                onCancel={() => viewport.setCoordinateRotation(coordinateRotation())}
              />
              <label class="check">
                <input
                  type="checkbox"
                  checked={gizmoVertical()}
                  onChange={(event) => setGizmoVertical(event.currentTarget.checked)}
                />{" "}
                Show vertical gizmo handle
              </label>
              <div class="view-overlays">
                <label class="check">
                  <input
                    type="checkbox"
                    checked={showObstacles()}
                    onChange={(e) => setShowObstacles(e.currentTarget.checked)}
                  />{" "}
                  Obstacles
                </label>
                <label class="check">
                  <input
                    type="checkbox"
                    checked={showElevation()}
                    onChange={(e) => setShowElevation(e.currentTarget.checked)}
                  />{" "}
                  Elevation lines
                </label>
              </div>
              <label class="perspective-control">
                <span>
                  Perspective{" "}
                  <output>{perspective() === 0 ? "Orthographic" : `${perspective()}°`}</output>
                </span>
                <input
                  aria-label="Perspective"
                  type="range"
                  min="0"
                  max="65"
                  step="1"
                  value={perspective()}
                  onInput={(e) => setPerspective(Number(e.currentTarget.value))}
                />
              </label>
              <p class="hint">
                Increase perspective for depth and distance scaling. Orbit to view characters from
                different sides and heights.
              </p>
              <label class="check">
                <input
                  type="checkbox"
                  checked={rotationSnap()}
                  onChange={(e) => setRotationSnap(e.currentTarget.checked)}
                />{" "}
                Lock rotation to 16 angles
              </label>
              <label class="check">
                <input
                  type="checkbox"
                  checked={spriteOrientationLock()}
                  onChange={(e) => setSpriteOrientationLock(e.currentTarget.checked)}
                />{" "}
                Lock sprite orientations
              </label>
              <p class="hint">
                Off: sprites face the camera. On: sprites use fixed 22.5° projection angles. Prone
                characters always stay locked.
              </p>
              <label class="check">
                <input
                  type="checkbox"
                  checked={showEntities()}
                  onChange={(e) => setShowEntities(e.currentTarget.checked)}
                />{" "}
                Mission entities
              </label>
              <label class="check">
                <input
                  type="checkbox"
                  checked={smoothTextures()}
                  onChange={(e) => setSmoothTextures(e.currentTarget.checked)}
                />{" "}
                Smooth textures
              </label>
              <label class="check">
                <input
                  type="checkbox"
                  checked={synthesizedTextures()}
                  onChange={(e) => setSynthesizedTextures(e.currentTarget.checked)}
                />{" "}
                Synthesized hidden surfaces
              </label>
              <For
                each={(() => {
                  doc();
                  patchPreviewRevision();
                  return viewport.patchPreviews();
                })()}
              >
                {(patch) => (
                  <label class="check">
                    <input
                      type="checkbox"
                      checked={patch.revealed}
                      onChange={(event) => {
                        viewport.setPatchRevealed(patch.id, event.currentTarget.checked);
                        setPatchPreviewRevision((value) => value + 1);
                      }}
                    />{" "}
                    Reveal interior: {patch.name}
                  </label>
                )}
              </For>
              <Show when={doc()?.population}>
                {(population) => (
                  <details>
                    <summary>
                      Town population — {population().actors.length} people,{" "}
                      {population().items.length} items
                    </summary>
                    <label class="check">
                      <input
                        type="checkbox"
                        checked={populationPlaying()}
                        onChange={(e) => {
                          setPopulationPlaying(e.currentTarget.checked);
                          viewport.setPopulationPlaying(e.currentTarget.checked);
                        }}
                      />{" "}
                      Animate routines and patrols
                    </label>
                    <label class="check">
                      <input
                        type="checkbox"
                        checked={populationRoutes()}
                        onChange={(e) => {
                          setPopulationRoutes(e.currentTarget.checked);
                          viewport.setPopulationRoutesVisible(e.currentTarget.checked);
                        }}
                      />{" "}
                      Show patrol and civilian routes
                    </label>
                    <p class="hint">
                      Preview of authored routines. Combat, dialogue and item collection require a
                      playable mission export.
                    </p>
                    <For each={population().actors}>
                      {(actor) => (
                        <details>
                          <summary>{actor.name}</summary>
                          <p>{actor.duty}</p>
                          <Show when={actor.information}>
                            <p>{actor.information}</p>
                          </Show>
                        </details>
                      )}
                    </For>
                    <For each={population().items}>
                      {(item) => (
                        <p>
                          {item.name} ×{item.quantity} — {item.purpose}
                        </p>
                      )}
                    </For>
                  </details>
                )}
              </Show>
              <Show when={missionName()}>
                <p class="mission-summary">
                  {missionName()} — {missionInfo()}
                </p>
                <p class="hint">
                  PC spawns and soldiers are editable in Mission. Other entities are previews only;
                  mission scripts are not run. See Mission for import limitations.
                </p>
              </Show>
            </section>
            <WorkspacePanel document={doc} commit={commitTerrain} onError={props.onError} />
            <LightingPanel document={doc} commit={pushHistory} />
            <section class="view-settings export-settings">
              <h2>Test your map</h2>
              <p class="hint">
                Choose Export mod ZIP, put the downloaded ZIP in the game’s configured mods
                directory, then select your map in Custom Missions. Use the base game data for
                characters and shared resources. The editor’s population preview does not run
                mission scripts.
              </p>
              <h2>Export frame</h2>
              <p class="hint">
                Terrain generates walking areas and height layers automatically. Asset gameplay is
                compiled from local definitions. Assets need authored walkable surfaces and door
                connections. Player spawns belong to missions. Missing map definitions stop export.
                Mission scripts, lifts, jumps and interactive state changes are not supported yet.
              </p>
              <p class="hint">
                An optional crop for compilation. Assets remain editable outside the frame,
                including parts you intend to crop.
              </p>
              <div class="row">
                <button
                  disabled={!doc()}
                  onClick={() => {
                    try {
                      pushHistory({ ...doc()!, exportBounds: viewport.fitExportBounds() });
                    } catch (error) {
                      props.onError(String(error));
                    }
                  }}
                >
                  {doc()?.exportBounds ? "Fit to content" : "Set export frame"}
                </button>
                <Show when={doc()?.exportBounds}>
                  <button onClick={() => pushHistory({ ...doc()!, exportBounds: undefined })}>
                    Remove frame
                  </button>
                </Show>
              </div>
              <Show when={doc()?.exportBounds}>
                {(bounds) => (
                  <div class="export-fields">
                    <For each={["Left", "Top", "Width", "Height"]}>
                      {(label, index) => (
                        <ScrubNumber
                          label={`Export ${label.toLowerCase()}`}
                          step={1}
                          min={index() > 1 ? 1 : undefined}
                          value={bounds()[index()]!}
                          onPreview={(value) => {
                            const next = [...bounds()] as [number, number, number, number];
                            next[index()] = Math.round(value);
                            viewport.syncViews({ ...doc()!, exportBounds: next }, false);
                          }}
                          onCommit={(value) => {
                            const next = [...bounds()] as [number, number, number, number];
                            next[index()] = Math.round(value);
                            pushHistory({ ...doc()!, exportBounds: next });
                          }}
                          onCancel={() => viewport.syncViews(doc()!, false)}
                        />
                      )}
                    </For>
                  </div>
                )}
              </Show>
              <Show when={!doc()?.exportBounds}>
                <p class="hint">No custom crop set.</p>
              </Show>
            </section>
          </div>
          <div
            class="inspector-content selection-inspector"
            hidden={panel() !== "Assets" || viewSettings()}
          >
            <Show
              when={selectedTransform()}
              fallback={
                <p class="hint">
                  Select an object in the scene or the list below to edit its properties. Alt-click
                  to select a single part.
                </p>
              }
            >
              {(t) => (
                <section class="object-detail">
                  <h2>{selectionTitle()}</h2>
                  <div class="row">
                    <button
                      class="asset-action"
                      onClick={duplicateSelected}
                      title="Duplicate (D)"
                      aria-label="Duplicate"
                      disabled={!!selectedStatePart()}
                    >
                      <svg
                        width="18"
                        height="18"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.8"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                      >
                        <rect x="8" y="8" width="12" height="12" rx="2" />
                        <path d="M16 8V4a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h4" />
                      </svg>
                    </button>
                    <button
                      class="asset-action asset-action-delete"
                      onClick={deleteSelected}
                      title="Delete (Del)"
                      aria-label="Delete"
                      disabled={!!selectedStatePart()}
                    >
                      <svg
                        width="18"
                        height="18"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.8"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                      >
                        <path d="M3 6h18M9 6V4h6v2M5 6l1 14h12l1-14M10 10v6M14 10v6" />
                      </svg>
                    </button>
                    <button
                      class="asset-action"
                      aria-label="Hidden"
                      aria-pressed={
                        (selectedGroup()?.hidden ?? selectedPart()?.hidden) ? "true" : "false"
                      }
                      title={
                        (selectedGroup()?.hidden ?? selectedPart()?.hidden)
                          ? "Show asset"
                          : "Hide asset"
                      }
                      disabled={!!selectedStatePart()}
                      onClick={() =>
                        setHidden(!(selectedGroup()?.hidden ?? selectedPart()?.hidden))
                      }
                    >
                      <svg
                        width="18"
                        height="18"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.8"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                      >
                        <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z" />
                        <circle cx="12" cy="12" r="3" />
                        <Show when={selectedGroup()?.hidden ?? selectedPart()?.hidden}>
                          <path d="m3 3 18 18" />
                        </Show>
                      </svg>
                    </button>
                  </div>
                  <Show when={selectedPart()}>
                    {(p) => (
                      <>
                        <div class="meta-row">
                          <span class="meta-key">source</span>
                          <span>
                            {p().source.map}{" "}
                            {p().kind === "scenery"
                              ? "scenery"
                              : (p().source.mission_profile ?? `#${p().source.obstacle}`)}
                          </span>
                        </div>
                        <Show when={p().obstacle}>
                          {(ob) => (
                            <>
                              <div class="meta-row">
                                <span class="meta-key">flags</span>
                                <span>
                                  {ob().opaque ? "opaque " : "clear "}
                                  {(ob() as unknown as { solid?: boolean }).solid ? "solid" : ""}
                                </span>
                              </div>
                              <div class="meta-row">
                                <span class="meta-key">height</span>
                                <span>
                                  {Math.round(Math.min(...ob().points.map((q) => q.z_bottom)))}–
                                  {Math.round(Math.max(...ob().points.map((q) => q.z_top)))}
                                </span>
                              </div>
                            </>
                          )}
                        </Show>
                        <Show when={p().group}>
                          {(g) => (
                            <button onClick={() => select({ kind: "group", id: g() })}>
                              Select building {g()}
                            </button>
                          )}
                        </Show>
                        <Show
                          when={(() => {
                            const index = p().source.obstacle;
                            return index === undefined ? undefined : suspects().get(index);
                          })()}
                        >
                          {(sus) => (
                            <div class="row suspect">
                              <span class="hint">
                                Floats {Math.round(sus().delta)} above #{sus().support}; may be
                                stored displaced along the view ray (same map pixels).
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
                  <div class="transform-fields">
                    <For each={["dx", "dy", "rot_deg", "dz"] as const}>
                      {(f) => (
                        <ScrubNumber
                          label={
                            {
                              dx: "X",
                              dy: "Y",
                              dz: "Height",
                              rot_deg: "Rotation (°)",
                            }[f]
                          }
                          step={f === "rot_deg" ? 5 : 1}
                          value={t()[f]}
                          onPreview={(value) => previewTransformField(f, value)}
                          onCommit={(value) => setTransformField(f, value)}
                          onCancel={() => {
                            if (!disposed && doc()) viewport.syncViews(doc()!, false);
                          }}
                        />
                      )}
                    </For>
                  </div>
                  <Show when={selectedPatchPreviews().length > 0}>
                    <h3>Linked patches</h3>
                    <For each={selectedPatchPreviews()}>
                      {(patch) => (
                        <label class="check">
                          <input
                            type="checkbox"
                            checked={patch.revealed}
                            onChange={(event) => {
                              viewport.setPatchRevealed(patch.id, event.currentTarget.checked);
                              setPatchPreviewRevision((value) => value + 1);
                            }}
                          />{" "}
                          Reveal interior: {patch.name}
                        </label>
                      )}
                    </For>
                  </Show>
                  <Show when={selectedGroup()?.states}>
                    <label>
                      State{" "}
                      <select
                        value={selectedGroup()?.states?.active}
                        onChange={(event) =>
                          pushHistory(
                            setGroupState(
                              doc()!,
                              selectedGroup()!.id,
                              event.currentTarget.value as "initial" | "applied",
                            ),
                          )
                        }
                      >
                        <option value="initial">Initial</option>
                        <option value="applied">Applied</option>
                      </select>
                    </label>
                  </Show>
                  <Show when={selectedStatePart()}>
                    <p>Select the whole group to change its state, duplicate it, or delete it.</p>
                  </Show>
                </section>
              )}
            </Show>
            <InteriorConnectionsPanel
              document={doc}
              library={() => props.library()?.handle ?? null}
              commit={pushHistory}
              onError={props.onError}
            />
            <section class="object-list">
              <h3>
                Scene objects <span class="object-count">{doc()?.objects.length ?? 0}</span>
              </h3>
              <div class="search-row">
                <input
                  class="search"
                  aria-label="Find scene objects"
                  placeholder="Find objects…"
                  value={filter()}
                  onInput={(e) => setFilter(e.currentTarget.value)}
                />
              </div>
              <Show when={doc() && !rows().length}>
                <p class="hint">
                  {filter()
                    ? "No objects match your search."
                    : "Your scene has no objects yet. Add one from Assets."}
                </p>
              </Show>
              <ul
                ref={(element) => {
                  sceneObjectList = element;
                }}
              >
                <For each={rows()}>
                  {(r) => (
                    <li
                      class={`${isSelected(r) ? "selected" : ""} ${r.hidden ? "hidden" : ""} depth-${r.depth}`}
                      tabindex={0}
                      role="button"
                      aria-pressed={isSelected(r) ? "true" : "false"}
                      title={r.label}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          select({ kind: r.kind, id: r.id }, false);
                        }
                      }}
                      onClick={() => select({ kind: r.kind, id: r.id }, false)}
                    >
                      <Show
                        when={r.kind === "group"}
                        fallback={
                          <span class="kind">{r.id.startsWith("terrace") ? "▬" : "·"}</span>
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
          </div>
        </aside>
      </div>
      <footer class="editor-footer">
        <span class="document-state">
          {mapName() ? mapLabel(mapName()!) : "No map open"}
          {doc() ? (dirty() ? " · Unsaved changes" : " · Saved") : ""}
        </span>
        <span class="editor-status" role="status">
          {info()}
        </span>
        <span class="footer-hint">Drag to pan · Right-drag to orbit · Scroll to zoom</span>
      </footer>
    </div>
  );
}
