import LibraryPortal from "./LibraryPortal";
import LibraryBrowser from "./LibraryBrowser";
import ScrubNumber from "./ScrubNumber";
import { For, Show, createEffect, createSignal, onCleanup, untrack } from "solid-js";
import {
  parseLevel3D,
  type ExternalAssetSource,
  type Level3D,
  type LevelSpline,
  type ProjectionAssetEntry,
  type Vec3,
} from "@rle/shared";
import type { EditorViewport } from "./editor-viewport";
import { prepareProjectionAsset } from "./projection-library";
import { disposeObjectResources } from "./resources";
import { splineCurve } from "./spline-geometry";
import { readWallPresets, wallPreset, type WallPreset } from "./wall-presets";
import AssetPreview, { AssetPreviewRenderer } from "./AssetPreview";
import AssetPickerDialog from "./AssetPickerDialog";
import { availableWallPresets, builtInWallPresets, cornerAssetIds } from "./spline-presets";
import MaterialPicker from "./MaterialPicker";
import { insertSplinePoint } from "./spline-insertion";
import { terrainHeightAt } from "../../shared/src/authored-terrain.ts";

export default function SplinePanel(props: {
  document: () => Level3D | null;
  library: () => FileSystemDirectoryHandle | null;
  entries: () => ProjectionAssetEntry[];
  viewport: EditorViewport;
  commit(document: Level3D): void;
  onError(message: string): void;
  active?: boolean;
  libraryMount?: HTMLElement;
  previewRenderer?: AssetPreviewRenderer;
  onEditingChange?(editing: boolean): void;
}) {
  const [active, setActive] = createSignal("");
  const [draft, setDraft] = createSignal<LevelSpline | null>(null);
  const [point, setPointValue] = createSignal(0);
  const [selectedPoints, setSelectedPoints] = createSignal<number[] | null>(null);
  const setPoint: typeof setPointValue = (value) => {
    setSelectedPoints(null);
    return setPointValue(value);
  };
  const [section, setSection] = createSignal(-1);
  const [picker, setPicker] = createSignal<"wall" | "corner" | null>(null);
  const [sourceMap, setSourceMap] = createSignal("");
  const [presetSearch, setPresetSearch] = createSignal("");
  const previewRenderer = props.previewRenderer ?? new AssetPreviewRenderer();
  onCleanup(() => {
    if (!props.previewRenderer) previewRenderer.dispose();
  });
  const [busy, setBusy] = createSignal(false);
  const [presets, setPresets] = createSignal(readWallPresets());

  let pendingSources: ExternalAssetSource[] = [];
  let disposed = false;
  let attempt = 0;
  const path = () =>
    draft() ?? props.document()?.splines?.find((path) => path.id === active()) ?? null;
  const pointHeight = (path: LevelSpline, index: number) => {
    const p = path.points[index],
      document = props.document();
    if (!p) return 0;
    return path.kind === "road" && document
      ? (terrainHeightAt(document, p[0], p[1]) ?? p[2]) + (path.pointHeightOffsets?.[index] ?? 0)
      : p[2];
  };
  const choices = () => [
    ...availableWallPresets(props.entries()),
    ...presets().filter((p) => props.entries().some((e) => e.id === p.asset)),
  ];
  const sources = () =>
    props.entries().filter((entry) => choices().some((p) => p.asset === entry.id));
  createEffect(
    () => !!draft() || busy(),
    (editing) => {
      props.onEditingChange?.(editing);
    },
  );
  function exit() {
    setActive("");
    setDraft(null);
    setPoint(0);
    pendingSources = [];
    untrack(() => props.viewport.setSplineEdit(null));
  }
  function publish(next: Level3D) {
    try {
      parseLevel3D(next);
      props.commit(next);
      return true;
    } catch (error) {
      props.onError(String(error));
      return false;
    }
  }
  function change(next: LevelSpline) {
    const document = props.document();
    if (!document) return;
    if (
      !Number.isFinite(next.width) ||
      next.width <= 0 ||
      !Number.isFinite(next.repeatLength) ||
      next.repeatLength < 1 ||
      next.points.some((p) => p.some((v) => !Number.isFinite(v))) ||
      !Number.isFinite(next.sourceAngle ?? 0) ||
      (next.cornerAsset !== undefined &&
        (!Number.isFinite(next.cornerMinAngle ?? 35) ||
          (next.cornerMinAngle ?? 35) <= 0 ||
          (next.cornerMinAngle ?? 35) >= 180 ||
          !Number.isFinite(next.cornerScale ?? 1) ||
          (next.cornerScale ?? 1) <= 0 ||
          (next.cornerScale ?? 1) > 10 ||
          !Number.isFinite(next.cornerRotation ?? 0) ||
          !Number.isFinite(next.cornerWidthScale ?? 1) ||
          (next.cornerWidthScale ?? 1) <= 0 ||
          (next.cornerWidthScale ?? 1) > 10)) ||
      (next.kind === "wall" &&
        ((next.sourceStart ?? 0) < 0 ||
          (next.sourceEnd ?? 1) > 1 ||
          (next.sourceEnd ?? 1) - (next.sourceStart ?? 0) < 0.05))
    ) {
      props.onError(
        "Use positive width/repeat values and retain at least 5% of the source segment",
      );
      return;
    }
    if (
      next.kind === "wall" &&
      next.points.length >= 2 &&
      splineCurve(next, document.camera).getLength() / next.repeatLength > 512
    ) {
      props.onError("Increase repeat length: a wall path supports at most 512 repeats");
      return;
    }
    if (draft()) {
      setDraft(next);
      return;
    }
    if (document)
      publish({
        ...document,
        splines: document.splines?.map((path) => (path.id === next.id ? next : path)),
      });
  }
  function patch(values: Partial<LevelSpline>) {
    const current = path();
    if (current) change({ ...current, ...values });
  }
  function NumberField(field: {
    label: string;
    value: number;
    step?: number;
    min?: number;
    max?: number;
    patch(value: number): Partial<LevelSpline>;
  }) {
    return (
      <ScrubNumber
        label={field.label}
        value={field.value}
        step={field.step ?? 1}
        min={field.min}
        max={field.max}
        onPreview={(value) => {
          const current = path();
          if (current) props.viewport.previewSpline({ ...current, ...field.patch(value) });
        }}
        onCommit={(value) => {
          props.viewport.previewSpline(null);
          patch(field.patch(value));
        }}
        onCancel={() => props.viewport.previewSpline(null)}
      />
    );
  }
  function insertPoint(index: number, fraction = 0.5, position?: Vec3) {
    const current = path();
    if (!current || busy()) return;
    if (current.points.length >= 256) {
      props.onError("A path supports at most 256 control points");
      return;
    }
    change(insertSplinePoint(current, index, fraction, position));
    setPoint(index + 1);
    setSection(-1);
  }
  function move(index: number, position: Vec3) {
    const current = path();
    if (current) patch({ points: current.points.map((p, i) => (i === index ? position : p)) });
  }
  async function loadCorner(id: string, fromPreset = false) {
    const current = path(),
      document = props.document(),
      root = props.library();
    if (!current || !document || !root || (busy() && !fromPreset)) return;
    if (id === current.cornerAsset) return;
    if (!id) {
      patch({ cornerAsset: undefined, cornerDisabled: undefined });
      return;
    }
    if (!cornerAssetIds.has(id))
      return props.onError("Choose a reviewed exterior-only corner model");
    const entry = props.entries().find((e) => e.id === id);
    if (!entry) return props.onError("Corner model is missing from the shared library");
    const token = ++attempt;
    setBusy(true);
    let prepared: Awaited<ReturnType<typeof prepareProjectionAsset>> | null = null;
    try {
      prepared = await prepareProjectionAsset(root, entry, document.map);
      if (
        disposed ||
        token !== attempt ||
        document !== props.document() ||
        path()?.id !== current.id
      )
        return;
      const reference = prepared.reference,
        existing = document.assetSources?.find((s) => s.id === id);
      if (
        existing &&
        (existing.model_sha256 !== reference.model_sha256 ||
          existing.descriptor_sha256 !== reference.descriptor_sha256)
      )
        throw new Error("The scene uses a different revision of this corner asset");
      if (!props.viewport.adoptAsset(reference, prepared.asset, prepared.sources))
        disposeObjectResources([prepared.asset]);
      prepared = null;
      const latest = path()!;
      const next = {
        ...latest,
        cornerAsset: id,
        cornerMinAngle: latest.cornerMinAngle ?? 35,
        cornerScale: latest.cornerScale ?? 1,
      };
      if (draft()) {
        pendingSources = pendingSources.filter((s) => s.id !== id).concat(reference);
        setDraft((latest) =>
          latest?.id === current.id
            ? {
                ...latest,
                cornerAsset: id,
                cornerMinAngle: latest.cornerMinAngle ?? 35,
                cornerScale: latest.cornerScale ?? 1,
              }
            : latest,
        );
      } else
        publish({
          ...document,
          assetSources: existing
            ? document.assetSources
            : [...(document.assetSources ?? []), reference],
          splines: document.splines?.map((s) => (s.id === current.id ? next : s)),
        });
    } catch (error) {
      props.onError(String(error));
    } finally {
      if (prepared) disposeObjectResources([prepared.asset]);
      if (!disposed) setBusy(false);
    }
  }
  async function loadWall(id: string) {
    const current = path(),
      document = props.document(),
      root = props.library();
    if (!current || current.kind !== "wall" || !document || !root || busy()) return;
    if (!id || id === current.asset) return;
    const entry = props.entries().find((e) => e.id === id);
    if (!entry) return props.onError("Wall model is missing from the shared library");
    const token = ++attempt;
    setBusy(true);
    let prepared: Awaited<ReturnType<typeof prepareProjectionAsset>> | null = null;
    try {
      prepared = await prepareProjectionAsset(root, entry, document.map);
      if (
        disposed ||
        token !== attempt ||
        document !== props.document() ||
        path()?.id !== current.id
      )
        return;
      const existing = document.assetSources?.find((s) => s.id === id);
      if (
        existing &&
        (existing.model_sha256 !== prepared.reference.model_sha256 ||
          existing.descriptor_sha256 !== prepared.reference.descriptor_sha256)
      )
        throw new Error("The scene already uses a different revision of this wall asset");
      const reference = prepared.reference;
      if (!props.viewport.adoptAsset(reference, prepared.asset, prepared.sources))
        disposeObjectResources([prepared.asset]);
      prepared = null;
      if (!existing) pendingSources = pendingSources.filter((s) => s.id !== id).concat(reference);
      const preset =
        builtInWallPresets.find((p) => p.asset === id) ?? presets().find((p) => p.asset === id);
      const replaceSource = (latest: LevelSpline): LevelSpline => ({
        ...latest,
        ...(preset ? wallPreset(preset) : {}),
        id: latest.id,
        name: latest.name,
        asset: id,
        cornerAsset: latest.cornerAsset,
        cornerMinAngle: latest.cornerMinAngle,
        cornerScale: latest.cornerScale,
        cornerWidthScale: latest.cornerWidthScale,
        cornerRotation: latest.cornerRotation,
        cornerDisabled: latest.cornerDisabled,
      });
      if (draft())
        setDraft((latest) => (latest?.id === current.id ? replaceSource(latest) : latest));
      else
        publish({
          ...document,
          assetSources: existing
            ? document.assetSources
            : [...(document.assetSources ?? []), reference],
          splines: document.splines?.map((s) => (s.id === current.id ? replaceSource(s) : s)),
        });
    } catch (error) {
      props.onError(String(error));
    } finally {
      if (prepared) disposeObjectResources([prepared.asset]);
      if (!disposed) setBusy(false);
    }
  }
  async function begin(kind: "river" | "road" | "wall", preset?: WallPreset) {
    if (preset) preset = wallPreset(preset);
    const document = props.document(),
      root = props.library();
    if (!document || (kind === "wall" && !root) || busy()) return;
    const token = ++attempt;
    setBusy(true);
    let prepared: Awaited<ReturnType<typeof prepareProjectionAsset>> | null = null;
    try {
      let reference: ExternalAssetSource | undefined;
      let wallWidth = 35,
        wallRepeat = 180,
        sourceAngle = 0;
      if (kind === "wall") {
        const wanted = preset?.asset;
        const entry = wanted ? sources().find((entry) => entry.id === wanted) : sources()[0];
        if (!entry) throw new Error("Publish a wall asset to the shared library first");
        prepared = await prepareProjectionAsset(root!, entry, document.map);
        if (
          disposed ||
          token !== attempt ||
          document !== props.document() ||
          root !== props.library()
        )
          return;
        const existing = document.assetSources?.find((source) => source.id === entry.id);
        if (
          existing &&
          (existing.model_sha256 !== prepared.reference.model_sha256 ||
            existing.descriptor_sha256 !== prepared.reference.descriptor_sha256)
        )
          throw new Error("The scene already uses a different revision of this asset");
        reference = prepared.reference;
        if (!preset) throw new Error("Choose a prepared wall preset");
        wallWidth = preset.width;
        wallRepeat = preset.repeatLength;
        sourceAngle = preset.sourceAngle ?? 0;
        if (!props.viewport.adoptAsset(reference, prepared.asset, prepared.sources))
          disposeObjectResources([prepared.asset]);
        prepared = null;
      }
      exit();
      pendingSources = reference ? [reference] : [];
      const newPathId = "path-" + crypto.randomUUID();
      setDraft({
        id: newPathId,
        name: kind === "river" ? "River" : kind === "road" ? "Footpath" : "Battlement wall",
        kind,
        points: [],
        ...(kind === "wall" ? {} : { pointWidths: [], pointMaterials: [] }),
        closed: false,
        width: kind === "river" ? 110 : kind === "road" ? 26 : wallWidth,
        repeatLength: kind === "river" ? 150 : wallRepeat,
        ...(reference
          ? { asset: reference.id, axis: "x" as const, sourceAngle, sourceStart: 0, sourceEnd: 1 }
          : {}),
        ...(preset ? wallPreset(preset) : {}),
        cornerAsset: undefined,
      });
      if (preset?.cornerAsset) {
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
        if (disposed || token !== attempt || path()?.id !== newPathId) return;
        await loadCorner(preset.cornerAsset, true);
      }
    } catch (error) {
      props.onError(String(error));
    } finally {
      if (prepared) disposeObjectResources([prepared.asset]);
      if (!disposed) setBusy(false);
    }
  }
  function finish() {
    const document = props.document();
    if (busy() || !document) return;
    setDraft((current) => {
      if (!current || current.points.length < (current.closed ? 3 : 2)) return current;
      const assetSources = [...(document.assetSources ?? [])];
      for (const source of pendingSources)
        if (!assetSources.some((s) => s.id === source.id)) assetSources.push(source);
      if (!publish({ ...document, assetSources, splines: [...(document.splines ?? []), current] }))
        return current;
      setActive(current.id);
      pendingSources = [];
      return null;
    });
  }
  function removePoint() {
    const current = path();
    if (!current || current.points.length <= (draft() ? 0 : current.closed ? 3 : 2)) return;
    const index = Math.min(point(), current.points.length - 1);
    patch({
      points: current.points.filter((_, i) => i !== index),
      pointWidths: current.pointWidths?.filter((_, i) => i !== index),
      pointHeightOffsets: current.pointHeightOffsets?.filter((_, i) => i !== index),
      pointMaterials: current.pointMaterials?.filter((_, i) => i !== index),
      pointMaterialMixes: current.pointMaterialMixes?.filter((_, i) => i !== index),
      cornerDisabled: current.cornerDisabled
        ?.filter((i) => i !== index)
        .map((i) => (i > index ? i - 1 : i)),
    });
    setPoint(Math.max(0, index - 1));
  }
  createEffect(
    () => ({ enabled: props.active !== false, map: props.document()?.map }),
    ({ enabled }) =>
      props.viewport.setSplineSelection(
        enabled
          ? (id) => {
              if (draft() || busy()) return;
              pendingSources = [];
              setActive(id);
              setPoint(0);
              setSection(-1);
            }
          : null,
      ),
  );
  let editingMap: string | undefined;
  createEffect(
    () => props.document()?.map,
    (map) => {
      if (map !== editingMap) {
        editingMap = map;
        attempt++;
        exit();
      }
    },
  );
  createEffect(
    () => ({
      current: props.active === false ? null : path(),
      selected: point(),
      selectedSection: section(),
      selectedPoints: selectedPoints(),
      drawing: !!draft(),
    }),
    ({ current, selected, selectedSection, selectedPoints, drawing }) => {
      untrack(() =>
        props.viewport.setSplineEdit(
          current
            ? {
                path: current,
                point: selected,
                selectedPoints: selectedPoints ?? [selected],
                section: selectedSection,
                drawing,
                append(position) {
                  // A pointer gesture can retain this callback while reactive
                  // viewport publication is pending. The setter sees pending
                  // writes as well as the last published draft.
                  setDraft((latest) => {
                    if (!latest || latest.id !== current.id) return latest;
                    const last = latest.points.at(-1);
                    if (last && Math.hypot(last[0] - position[0], last[1] - position[1]) < 1)
                      return latest;
                    if (latest.points.length >= 256) {
                      props.onError("A path supports at most 256 control points");
                      return latest;
                    }
                    setPoint(latest.points.length);
                    const height = latest.points[0]?.[2] ?? position[2];
                    return {
                      ...latest,
                      points: [...latest.points, [position[0], position[1], height]],
                      pointWidths: latest.pointWidths
                        ? [...latest.pointWidths, latest.pointWidths.at(-1) ?? latest.width]
                        : undefined,
                      pointHeightOffsets: latest.pointHeightOffsets
                        ? [...latest.pointHeightOffsets, latest.pointHeightOffsets.at(-1) ?? 0]
                        : undefined,
                      pointMaterials: latest.pointMaterials
                        ? [
                            ...latest.pointMaterials,
                            latest.pointMaterials.at(-1) ??
                              (latest.kind === "river" ? "water_still" : "path_dirt"),
                          ]
                        : undefined,
                      pointMaterialMixes: latest.pointMaterialMixes
                        ? [...latest.pointMaterialMixes, latest.pointMaterialMixes.at(-1) ?? null]
                        : undefined,
                    };
                  });
                },
                move,
                movePoints(indices, delta) {
                  const latest = path();
                  if (latest)
                    patch({
                      points: latest.points.map((p, i) =>
                        indices.includes(i)
                          ? [p[0] + delta[0], p[1] + delta[1], p[2] + delta[2]]
                          : p,
                      ),
                    });
                },
                selectPoints(indices) {
                  if (indices.length) setPointValue(indices[0]!);
                  setSelectedPoints(indices);
                  setSection(-1);
                },
                insert: insertPoint,
                selectPoint(index) {
                  setPoint(index);
                  setSection(-1);
                },
                selectSection(index) {
                  setSection(index);
                  setPoint(index);
                },
              }
            : null,
        ),
      );
    },
  );
  const onKey = (event: KeyboardEvent) => {
    if (
      props.active === false ||
      document.querySelector("dialog[open]") ||
      !path() ||
      (event.target instanceof HTMLElement &&
        (event.target.isContentEditable || /INPUT|TEXTAREA|SELECT/.test(event.target.tagName)))
    )
      return;
    if (event.key === "Escape") {
      event.stopImmediatePropagation();
      event.preventDefault();
      exit();
    } else if (event.key === "Enter" && draft()) {
      event.stopImmediatePropagation();
      event.preventDefault();
      finish();
    } else if (event.key === "Delete" || event.key === "Backspace") {
      event.stopImmediatePropagation();
      event.preventDefault();
      removePoint();
    }
  };
  window.addEventListener("keydown", onKey, true);
  onCleanup(() => {
    disposed = true;
    attempt++;
    window.removeEventListener("keydown", onKey, true);
    props.viewport.setSplineSelection(null);
    props.viewport.setSplineEdit(null);
  });
  return (
    <section class="spline-panel">
      <h2>Paths &amp; walls</h2>
      <Show when={!path() && props.active !== false}>
        <Show when={props.document()?.splines?.length}>
          <h3>On this map</h3>
          <div class="spline-list" aria-label="Saved paths">
            <For each={props.document()?.splines ?? []}>
              {(item) => (
                <button
                  class={active() === item.id ? "selected" : ""}
                  onClick={() => {
                    pendingSources = [];
                    setActive(item.id);
                    setPoint(0);
                  }}
                >
                  <strong>{item.name}</strong>
                  <span>
                    {item.kind === "road" ? "Footpath" : item.kind === "river" ? "River" : "Wall"} ·{" "}
                    {item.points.length} points
                  </span>
                </button>
              )}
            </For>
          </div>
        </Show>
      </Show>
      <LibraryPortal mount={props.libraryMount} active={props.active !== false}>
        <h3>Create a path</h3>
        <p class="hint">
          {path()
            ? "Finish or stop editing the current path to create another."
            : "Choose a surface, then click the map to place points."}
        </p>
        <div class="spline-preset-grid">
          <For each={["road", "river"] as const}>
            {(kind) => (
              <button
                class="asset-card"
                disabled={busy() || !props.document() || !!path()}
                onClick={() => void begin(kind)}
              >
                <svg
                  class={`surface-preset-preview ${kind}`}
                  viewBox="0 0 160 100"
                  aria-hidden="true"
                >
                  <path d="M-10 85 C35 85 25 20 75 25 S115 80 170 15" />
                </svg>
                <span class="asset-card-info">
                  <strong>{kind === "road" ? "Footpath" : "River"}</strong>
                </span>
              </button>
            )}
          </For>
        </div>
        <details class="spline-settings">
          <summary>Walls &amp; fences</summary>
          <LibraryBrowser
            search={presetSearch()}
            onSearch={setPresetSearch}
            searchLabel="Search walls and fences"
            label="Walls and fences"
            maxHeight="360px"
            filters={
              <label>
                Source map
                <select
                  aria-label="Preset source map"
                  value={sourceMap()}
                  onChange={(event) => setSourceMap(event.currentTarget.value)}
                >
                  <option value="">All maps</option>
                  <For each={[...new Set(sources().map((entry) => entry.source_map))].sort()}>
                    {(name) => <option value={name}>{name}</option>}
                  </For>
                </select>
              </label>
            }
            beforeGrid={
              <Show when={!props.library()}>
                <p class="hint">Load an asset library to draw walls and fences.</p>
              </Show>
            }
          >
            <For
              each={choices().filter(
                (p) =>
                  (!sourceMap() ||
                    props.entries().find((e) => e.id === p.asset)?.source_map === sourceMap()) &&
                  `${p.name} ${p.asset}`
                    .toLowerCase()
                    .includes(presetSearch().trim().toLowerCase()),
              )}
            >
              {(preset) => {
                const entry = () => props.entries().find((entry) => entry.id === preset.asset);
                return (
                  <button
                    class="asset-card"
                    disabled={busy() || !props.document() || !!path()}
                    onClick={() => void begin("wall", preset)}
                  >
                    <Show when={entry() && props.library()}>
                      <AssetPreview
                        entry={entry()!}
                        root={props.library()!}
                        renderer={previewRenderer}
                      />
                    </Show>
                    <span class="asset-card-info">
                      <strong>{preset.name}</strong>
                    </span>
                  </button>
                );
              }}
            </For>
          </LibraryBrowser>
        </details>
      </LibraryPortal>
      <Show when={picker() && props.library()}>
        <AssetPickerDialog
          renderer={previewRenderer}
          title={picker() === "wall" ? "Choose wall type" : "Choose corner type"}
          root={props.library()!}
          entries={
            picker() === "wall"
              ? sources()
              : props.entries().filter((entry) => cornerAssetIds.has(entry.id))
          }
          selected={
            picker() === "wall"
              ? path()?.asset
              : cornerAssetIds.has(path()?.cornerAsset ?? "")
                ? path()?.cornerAsset
                : undefined
          }
          emptyLabel={picker() === "corner" ? "Continuous join — no corner model" : undefined}
          onClose={() => setPicker(null)}
          onSelect={(id) => {
            const kind = picker();
            setPicker(null);
            if (kind === "wall") void loadWall(id);
            else void loadCorner(id);
          }}
        />
      </Show>
      <Show when={path()}>
        {(current) => (
          <>
            <header class="spline-edit-header">
              <strong>
                {draft() ? "Drawing" : "Editing"}{" "}
                {current().kind === "road" ? "footpath" : current().kind}
              </strong>
              <span>{current().points.length} points</span>
            </header>
            <div class="spline-actions spline-edit-actions">
              <Show
                when={draft()}
                fallback={
                  <button class="spline-primary" onClick={exit}>
                    Done editing
                  </button>
                }
              >
                <button
                  class="spline-primary"
                  disabled={busy() || current().points.length < (current().closed ? 3 : 2)}
                  onClick={finish}
                >
                  Finish path
                </button>
                <button onClick={exit}>Cancel</button>
              </Show>
              <Show when={!draft()}>
                <button
                  onClick={() => {
                    const document = props.document();
                    if (
                      document &&
                      publish({
                        ...document,
                        splines: document.splines?.filter((item) => item.id !== current().id),
                      })
                    )
                      exit();
                  }}
                >
                  Delete path
                </button>
              </Show>
            </div>
            <p class="hint" role="status">
              {draft()
                ? "Click the ground to add points. Enter finishes; Escape cancels."
                : "Drag the cyan points to reshape. Drag empty ground to pan; right-drag to orbit."}
            </p>
            <label>
              Name
              <input
                aria-label="Path name"
                value={current().name}
                onChange={(event) => patch({ name: event.currentTarget.value })}
              />
            </label>
            <div class="spline-fields">
              <NumberField
                label="Path width"
                value={current().width}
                min={1}
                patch={(value) => ({ width: value, pointWidths: undefined })}
              />
              <Show when={current().kind === "wall"}>
                <NumberField
                  label="Path repeat length"
                  value={current().repeatLength}
                  min={1}
                  patch={(value) => ({ repeatLength: value })}
                />
              </Show>
            </div>
            <Show when={current().points.length > 0}>
              <NumberField
                label="Path elevation"
                value={pointHeight(current(), 0)}
                patch={(value) => ({
                  points: current().points.map((p) => [p[0], p[1], value]),
                  pointHeightOffsets:
                    current().kind === "road"
                      ? current().points.map(
                          (p) => value - (terrainHeightAt(props.document()!, p[0], p[1]) ?? p[2]),
                        )
                      : current().pointHeightOffsets,
                })}
              />
            </Show>
            <Show when={current().kind === "river"}>
              <label class="check">
                <input
                  type="checkbox"
                  checked={current().channel?.enabled ?? true}
                  onChange={(event) =>
                    patch({
                      channel: {
                        bedDepth: 24,
                        bankSlope: 1,
                        ...current().channel,
                        enabled: event.currentTarget.checked,
                      },
                    })
                  }
                />
                Automatically shape riverbed
              </label>
              <NumberField
                label="Riverbed depth (pixels)"
                value={current().channel?.bedDepth ?? 24}
                min={0}
                patch={(value) => ({
                  channel: { enabled: true, bankSlope: 1, ...current().channel, bedDepth: value },
                })}
              />
              <NumberField
                label="Bank slope (rise / run)"
                value={current().channel?.bankSlope ?? 1}
                min={0.01}
                step={0.1}
                patch={(value) => ({
                  channel: { enabled: true, bedDepth: 24, ...current().channel, bankSlope: value },
                })}
              />
            </Show>
            <label class="check">
              <input
                type="checkbox"
                checked={current().closed}
                disabled={current().points.length < 3}
                onChange={(event) => patch({ closed: event.currentTarget.checked })}
              />{" "}
              Closed loop
            </label>
            <Show when={current().kind === "wall"}>
              <div class="spline-actions">
                <button disabled={busy()} onClick={() => setPicker("wall")}>
                  Change wall type
                </button>
                <button disabled={busy()} onClick={() => setPicker("corner")}>
                  Change corner type
                </button>
              </div>
              <details class="spline-settings">
                <summary>Corner settings</summary>
                <Show when={cornerAssetIds.has(current().cornerAsset ?? "")}>
                  <NumberField
                    label="Corner minimum angle"
                    value={current().cornerMinAngle ?? 35}
                    min={1}
                    max={179}
                    patch={(value) => ({ cornerMinAngle: value })}
                  />
                  <NumberField
                    label="Corner tower scale"
                    value={current().cornerScale ?? 1}
                    min={0.1}
                    max={10}
                    step={0.1}
                    patch={(value) => ({ cornerScale: value })}
                  />
                  <NumberField
                    label="Corner tower width"
                    value={current().cornerWidthScale ?? 1}
                    min={0.1}
                    max={10}
                    step={0.1}
                    patch={(value) => ({ cornerWidthScale: value })}
                  />
                  <NumberField
                    label="Tower rotation offset"
                    value={current().cornerRotation ?? 0}
                    patch={(value) => ({ cornerRotation: value })}
                  />
                  <label class="check">
                    <input
                      type="checkbox"
                      aria-label="Tower at selected corner"
                      checked={!current().cornerDisabled?.includes(point())}
                      onChange={(e) =>
                        patch({
                          cornerDisabled: e.currentTarget.checked
                            ? current().cornerDisabled?.filter((i) => i !== point())
                            : [...(current().cornerDisabled ?? []), point()],
                        })
                      }
                    />{" "}
                    Tower at selected corner
                  </label>
                </Show>
              </details>
              <details class="spline-settings">
                <summary>Wall alignment &amp; presets</summary>
                <button
                  onClick={() => {
                    try {
                      const preset = wallPreset(current());
                      const next = presets()
                        .filter((p) => p.name !== preset.name)
                        .concat(preset);
                      localStorage.setItem("rle.wallPresets", JSON.stringify(next));
                      setPresets(next);
                    } catch (error) {
                      props.onError("Could not save wall preset: " + error);
                    }
                  }}
                >
                  Save as wall preset
                </button>
                <p class="hint">
                  Presets use the path name and are available across levels in this browser.
                </p>
                <label class="check">
                  <input
                    type="checkbox"
                    aria-label="Flip battlement side"
                    checked={current().flipCrossSection ?? false}
                    onChange={(event) => patch({ flipCrossSection: event.currentTarget.checked })}
                  />{" "}
                  Flip battlement side
                </label>
                <label>
                  Source direction
                  <select
                    aria-label="Wall source direction"
                    value={current().axis}
                    onChange={(event) => patch({ axis: event.currentTarget.value as "x" | "y" })}
                  >
                    <option value="x">Along X</option>
                    <option value="y">Along Y</option>
                  </select>
                </label>
                <NumberField
                  label="Source alignment angle"
                  value={current().sourceAngle ?? 0}
                  patch={(value) => ({ sourceAngle: value })}
                />
                <div class="spline-fields">
                  <NumberField
                    label="Trim start %"
                    value={(current().sourceStart ?? 0) * 100}
                    min={0}
                    max={(current().sourceEnd ?? 1) * 100 - 5}
                    patch={(value) => ({ sourceStart: value / 100 })}
                  />
                  <NumberField
                    label="Trim end %"
                    value={(current().sourceEnd ?? 1) * 100}
                    min={(current().sourceStart ?? 0) * 100 + 5}
                    max={100}
                    patch={(value) => ({ sourceEnd: value / 100 })}
                  />
                </div>
              </details>
            </Show>
            <Show when={current().kind !== "wall"}>
              <details class="spline-settings">
                <summary>Surface texture</summary>
                <p class="hint">
                  {current().texture
                    ? "Using a custom texture."
                    : "Using the built-in terrain texture."}
                </p>
                <Show when={current().texture}>
                  <NumberField
                    label="Path repeat length"
                    value={current().repeatLength}
                    min={1}
                    patch={(value) => ({ repeatLength: value })}
                  />
                </Show>
                <label>
                  Surface texture tile
                  <input
                    type="file"
                    accept="image/png,image/jpeg,image/webp"
                    onChange={(event) => {
                      const file = event.currentTarget.files?.[0],
                        id = current().id;
                      if (!file) return;
                      if (file.size > 8 * 1024 * 1024) {
                        props.onError("Use a tile smaller than 8 MB");
                        return;
                      }
                      const reader = new FileReader();
                      reader.onload = () => {
                        if (!disposed && path()?.id === id)
                          patch({ texture: String(reader.result) });
                      };
                      reader.onerror = () => props.onError("Could not read surface texture");
                      reader.readAsDataURL(file);
                    }}
                  />
                </label>
                <Show when={current().texture}>
                  <button onClick={() => patch({ texture: undefined })}>
                    Use default surface tile
                  </button>
                </Show>
              </details>
            </Show>
            <Show when={current().points.length > 0}>
              <h3>Control points</h3>
              <Show when={current().points.length > 1}>
                <label>
                  Section
                  <select
                    aria-label="Selected section"
                    value={section()}
                    onChange={(event) => {
                      const index = Number(event.currentTarget.value);
                      setSection(index);
                      if (index >= 0) setPoint(index);
                    }}
                  >
                    <option value={-1}>Choose a section</option>
                    <For each={current().points.slice(0, current().closed ? undefined : -1)}>
                      {(_, index) => (
                        <option value={index()}>
                          Point {index() + 1} → Point{" "}
                          {((index() + 1) % current().points.length) + 1}
                        </option>
                      )}
                    </For>
                  </select>
                </label>
                <Show when={section() >= 0}>
                  <div class="spline-actions">
                    <button onClick={() => setPoint(section())}>Edit start point</button>
                    <button onClick={() => setPoint((section() + 1) % current().points.length)}>
                      Edit end point
                    </button>
                  </div>
                </Show>
              </Show>
              <Show when={current().kind !== "wall"}>
                <NumberField
                  label="Point width"
                  value={current().pointWidths?.[point()] ?? current().width}
                  min={1}
                  patch={(value) => ({
                    pointWidths: current().points.map((_, i) =>
                      i === point() ? value : (current().pointWidths?.[i] ?? current().width),
                    ),
                  })}
                />
                <LibraryPortal mount={props.libraryMount} active={props.active !== false}>
                  <section class="path-material-library">
                    <MaterialPicker
                      defaultCategory={current().kind === "river" ? "river" : "path"}
                      label="Point material"
                      value={
                        current().pointMaterials?.[point()] ??
                        (current().kind === "river" ? "water_still" : "path_dirt")
                      }
                      customMaterials={props.document()?.customMaterials ?? []}
                      onCustomMaterialsChange={(customMaterials) => {
                        const document = props.document();
                        if (document) publish({ ...document, customMaterials });
                      }}
                      onChange={(id) =>
                        patch({
                          texture: undefined,
                          pointMaterialMixes: current().pointMaterialMixes?.map((mix, i) =>
                            i === point() ? null : mix,
                          ),
                          pointMaterials: current().points.map((_, i) =>
                            i === point()
                              ? id
                              : (current().pointMaterials?.[i] ??
                                (current().kind === "river" ? "water_still" : "path_dirt")),
                          ),
                        })
                      }
                    />
                  </section>
                </LibraryPortal>
                <Show when={current().pointMaterialMixes?.[point()]}>
                  <p class="hint">
                    This inserted point preserves a material blend. Choosing a material replaces
                    that blend.
                  </p>
                </Show>
                <p class="hint">
                  Width and material blend between points. Equal materials at both ends give a
                  uniform section.
                </p>
              </Show>
              <label>
                Selected point
                <select
                  aria-label="Selected control point"
                  value={point()}
                  onChange={(e) => setPoint(Number(e.currentTarget.value))}
                >
                  <For each={current().points}>
                    {(_, index) => (
                      <option value={index()}>
                        Point {index() + 1} of {current().points.length}
                      </option>
                    )}
                  </For>
                </select>
              </label>
            </Show>
            <details class="spline-settings">
              <summary>Point coordinates</summary>
              <Show when={current().points[point()]}>
                {(position) => (
                  <div class="spline-coordinates">
                    <For each={["X", "Y", "Z"]}>
                      {(label, axis) => (
                        <NumberField
                          label={"Control point " + label}
                          value={
                            Math.round(
                              (axis() === 2
                                ? pointHeight(current(), point())
                                : position()[axis()]!) * 10,
                            ) / 10
                          }
                          patch={(number) => {
                            const value: Vec3 = [...position()];
                            value[axis()] = number;
                            return {
                              points: current().points.map((p, i) => (i === point() ? value : p)),
                              pointHeightOffsets:
                                axis() === 2 && current().kind === "road"
                                  ? current().points.map((p, i) =>
                                      i === point()
                                        ? number -
                                          (terrainHeightAt(props.document()!, p[0], p[1]) ?? p[2])
                                        : (current().pointHeightOffsets?.[i] ?? 0),
                                    )
                                  : current().pointHeightOffsets,
                            };
                          }}
                        />
                      )}
                    </For>
                  </div>
                )}
              </Show>
            </details>
            <div class="spline-actions">
              <button
                disabled={current().points.length < 2 || current().points.length >= 256}
                onClick={() =>
                  insertPoint(
                    Math.min(point(), current().points.length - (current().closed ? 1 : 2)),
                  )
                }
              >
                Insert point
              </button>
              <button
                disabled={current().points.length <= (draft() ? 0 : current().closed ? 3 : 2)}
                onClick={removePoint}
              >
                Remove point
              </button>
            </div>
          </>
        )}
      </Show>
    </section>
  );
}
