import { For, Show, createEffect, createSignal } from "solid-js";
import type { Mission, ProtoLevel } from "@rle/shared";
import {
  getStoredDatadirHandle,
  getStoredLibraryHandle,
  pickDatadir,
  pickLibrary,
  requestDatadirPermission,
  restoreDatadir,
  restoreLibrary,
} from "./fs";
import {
  loadMapImage,
  loadMission,
  loadProtoLevel,
  missionsForMap,
  scanDatadir,
  type DatadirIndex,
} from "./datadir";
import Viewer from "./Viewer";
import { DEFAULT_TOGGLES, TOGGLE_LABELS, type OverlayToggles } from "./overlays";
import { releaseLibrary, scanLibrary, type LibraryAsset, type LibraryIndex } from "./library";
import { AssetDetail, LibrarySection } from "./Library";
import { ComposeViewer, PlacementsPanel } from "./Compose";
import {
  finalizeDraft,
  guideRect,
  newDraft,
  openDraftFile,
  saveDraftFile,
  wallSegmentSet,
  type DraftSelection,
} from "./compose";
import type { MapDraft } from "@rle/shared";

type Mode = "inspect" | "compose";

export default function App() {
  const [mode, setMode] = createSignal<Mode>("inspect");
  const [index, setIndex] = createSignal<DatadirIndex | null>(null);
  const [needsReconnect, setNeedsReconnect] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal<string | null>(null);

  const [mapName, setMapName] = createSignal<string | null>(null);
  const [ambiance, setAmbiance] = createSignal<string>("Day");
  const [missionName, setMissionName] = createSignal<string | null>(null);

  const [image, setImage] = createSignal<ImageBitmap | null>(null);
  const [level, setLevel] = createSignal<ProtoLevel | null>(null);
  const [mission, setMission] = createSignal<Mission | null>(null);

  const [library, setLibrary] = createSignal<LibraryIndex | null>(null);
  const [libraryNeedsReconnect, setLibraryNeedsReconnect] = createSignal(false);
  const [selectedAsset, setSelectedAsset] = createSignal<LibraryAsset | null>(null);

  const [draft, setDraft] = createSignal<MapDraft | null>(null);
  const [draftDocId, setDraftDocId] = createSignal(0);
  const [draftHandle, setDraftHandle] = createSignal<FileSystemFileHandle | null>(null);
  const [dirty, setDirty] = createSignal(false);
  const [selection, setSelection] = createSignal<DraftSelection>(null);
  const [wallDraw, setWallDraw] = createSignal(false);

  const [toggles, setToggles] = createSignal<OverlayToggles>({ ...DEFAULT_TOGGLES });
  const [layerFilter, setLayerFilter] = createSignal<number | null>(null);
  const [cursor, setCursor] = createSignal<[number, number]>([0, 0]);

  async function openRoot(handle: FileSystemDirectoryHandle) {
    setBusy("scanning datadir…");
    setError(null);
    try {
      setIndex(await scanDatadir(handle));
      setNeedsReconnect(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  async function openLibrary(handle: FileSystemDirectoryHandle) {
    setBusy("scanning library…");
    setError(null);
    try {
      const prev = library();
      setSelectedAsset(null);
      setLibrary(await scanLibrary(handle));
      if (prev) releaseLibrary(prev);
      setLibraryNeedsReconnect(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  // one-shot startup: restore previously granted datadir/library handles
  createEffect(
    () => undefined,
    () => {
      void (async () => {
        const restored = await restoreDatadir();
        if (restored) await openRoot(restored);
        else if (await getStoredDatadirHandle()) setNeedsReconnect(true);
        const lib = await restoreLibrary();
        if (lib) await openLibrary(lib);
        else if (await getStoredLibraryHandle()) setLibraryNeedsReconnect(true);
      })();
    },
  );

  async function onPickLibrary() {
    try {
      await openLibrary(await pickLibrary());
    } catch (e) {
      if ((e as DOMException).name !== "AbortError") setError(String(e));
    }
  }

  async function onReconnectLibrary() {
    const handle = await getStoredLibraryHandle();
    if (handle && (await requestDatadirPermission(handle))) await openLibrary(handle);
  }

  async function onPick() {
    try {
      await openRoot(await pickDatadir());
    } catch (e) {
      if ((e as DOMException).name !== "AbortError") setError(String(e));
    }
  }

  async function onReconnect() {
    const handle = await getStoredDatadirHandle();
    if (handle && (await requestDatadirPermission(handle))) await openRoot(handle);
  }

  async function selectMap(name: string) {
    const idx = index()!;
    setMapName(name);
    setMission(null);
    setMissionName(null);
    const ambs = idx.maps.get(name) ?? [];
    const amb = ambs.includes(ambiance()) ? ambiance() : (ambs[0] ?? "Day");
    setAmbiance(amb);
    setBusy(`loading ${name}…`);
    setError(null);
    try {
      const [img, lvl] = await Promise.all([
        loadMapImage(idx, name, amb),
        loadProtoLevel(idx, name),
      ]);
      setImage(img);
      setLevel(lvl);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  async function selectAmbiance(amb: string) {
    setAmbiance(amb);
    const name = mapName();
    if (!name) return;
    setBusy(`loading ${amb}…`);
    try {
      setImage(await loadMapImage(index()!, name, amb));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  async function selectMission(name: string | null) {
    setMissionName(name);
    if (!name) return setMission(null);
    setBusy(`loading ${name}…`);
    try {
      setMission(await loadMission(index()!, name));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  // --- compose mode: draft document actions ---

  function loadDraftDoc(d: MapDraft, handle: FileSystemFileHandle | null) {
    setDraft(d);
    setDraftHandle(handle);
    setDirty(false);
    setSelection(null);
    setWallDraw(false);
    setDraftDocId((n) => n + 1);
  }

  function onNewDraft() {
    const name = window.prompt("Draft name", "untitled");
    if (name === null) return;
    loadDraftDoc(newDraft(name || "untitled"), null);
  }

  async function onOpenDraft() {
    try {
      const { draft: d, handle } = await openDraftFile();
      loadDraftDoc(d, handle);
    } catch (e) {
      if ((e as DOMException).name !== "AbortError") setError(String(e));
    }
  }

  async function onSaveDraft() {
    const d = draft();
    if (!d) return;
    // size is determined here: content bounds + margin, placements normalized
    // to a (0,0) origin in the written file (the in-memory draft keeps its
    // coords so the view doesn't jump)
    let final = finalizeDraft(d, library());
    if (!draftHandle()) {
      // first save: let the user override the computed size
      const s = window.prompt(
        "Map size (width x height)",
        `${final.size[0]}x${final.size[1]}`,
      );
      if (s === null) return;
      const m = s.match(/^\s*(\d+)\s*[x×,]\s*(\d+)\s*$/);
      if (!m) return setError(`invalid size: ${s}`);
      final = { ...final, size: [Number(m[1]), Number(m[2])] };
    }
    try {
      setDraftHandle(await saveDraftFile(final, draftHandle()));
      setDirty(false);
    } catch (e) {
      if ((e as DOMException).name !== "AbortError") setError(String(e));
    }
  }

  const updateDraft = (fn: (d: MapDraft) => MapDraft) => {
    setDraft((d) => (d ? fn(d) : d));
    setDirty(true);
  };

  const placeAsset = (asset: LibraryAsset, pos: [number, number]) =>
    updateDraft((d) => ({
      ...d,
      placements: [...d.placements, { asset: asset.descriptor.id, pos }],
    }));

  const movePlacement = (idx: number, pos: [number, number]) =>
    updateDraft((d) => ({
      ...d,
      placements: d.placements.map((p, i) => (i === idx ? { ...p, pos } : p)),
    }));

  const commitWall = (asset: LibraryAsset, points: [number, number][]) => {
    // direction-aware set: all directional wall pieces from the same source
    // map; absent when the started asset has no direction (single-segment run)
    const set = wallSegmentSet(asset, library());
    updateDraft((d) => ({
      ...d,
      walls: [
        ...(d.walls ?? []),
        {
          asset: asset.descriptor.id,
          ...(set.length > 0 ? { segment_set: set } : {}),
          points,
        },
      ],
    }));
  };

  const editWallRun = (
    wall: number,
    fn: (points: [number, number][]) => [number, number][],
  ) =>
    updateDraft((d) => ({
      ...d,
      walls: (d.walls ?? []).map((r, i) =>
        i === wall ? { ...r, points: fn(r.points) } : r,
      ),
    }));

  const moveWallPoint = (wall: number, pt: number, pos: [number, number]) =>
    editWallRun(wall, (pts) => pts.map((p, j) => (j === pt ? pos : p)));

  const insertWallPoint = (wall: number, after: number, pos: [number, number]) =>
    editWallRun(wall, (pts) => [...pts.slice(0, after + 1), pos, ...pts.slice(after + 1)]);

  const removeWallPoint = (wall: number, pt: number) =>
    editWallRun(wall, (pts) => (pts.length > 2 ? pts.filter((_, j) => j !== pt) : pts));

  const deleteSelection = (sel: NonNullable<DraftSelection>) => {
    if (sel.kind === "placement")
      updateDraft((d) => ({
        ...d,
        placements: d.placements.filter((_, i) => i !== sel.idx),
      }));
    else
      updateDraft((d) => ({
        ...d,
        walls: (d.walls ?? []).filter((_, i) => i !== sel.idx),
      }));
    setSelection((s) =>
      s === null || s.kind !== sel.kind
        ? s
        : s.idx === sel.idx
          ? null
          : s.idx > sel.idx
            ? { kind: s.kind, idx: s.idx - 1 }
            : s,
    );
  };

  // wall-draw mode only applies while a spline-segment asset is selected
  const wallAsset = () => {
    const a = selectedAsset();
    return wallDraw() && a?.descriptor.scale_class === "spline-segment" ? a : null;
  };

  const selectAsset = (asset: LibraryAsset | null) => {
    setSelectedAsset(asset);
    if (asset?.descriptor.scale_class !== "spline-segment") setWallDraw(false);
  };

  const toggle = (key: keyof OverlayToggles) =>
    setToggles((t) => ({ ...t, [key]: !t[key] }));

  const layerCount = () => level()?.motion_data.layers.length ?? 0;

  // highlight the asset's source region when its source map is the loaded map
  const selectionBbox = () => {
    const asset = selectedAsset();
    const map = mapName();
    if (!asset || !map) return null;
    if (asset.descriptor.source.map.toLowerCase() !== map.toLowerCase()) return null;
    return asset.descriptor.source.bbox;
  };

  return (
    <div class="app">
      <aside class="sidebar">
        <h1>RH Level Editor</h1>
        <div class="row mode-switch">
          <button
            class={mode() === "inspect" ? "selected" : ""}
            onClick={() => setMode("inspect")}
          >
            Inspect
          </button>
          <button
            class={mode() === "compose" ? "selected" : ""}
            onClick={() => setMode("compose")}
          >
            Compose
          </button>
        </div>
        <Show when={mode() === "compose"}>
          <section>
            <h2>Draft</h2>
            <Show
              when={library()}
              fallback={
                <p class="hint">Connect the asset library below to start composing.</p>
              }
            >
              <div class="row">
                <button onClick={onNewDraft}>New…</button>
                <button onClick={() => void onOpenDraft()}>Open…</button>
                <button disabled={!draft()} onClick={() => void onSaveDraft()}>
                  Save
                </button>
              </div>
              <Show when={draft()}>
                {(d) => (
                  <div class="draft-info">
                    <div class="draft-name">
                      {d().name}
                      <Show when={dirty()}>
                        <span class="dirty" title="unsaved changes">
                          ● unsaved changes
                        </span>
                      </Show>
                    </div>
                    <div class="hint">
                      {(() => {
                        const g = guideRect(d(), library());
                        return g ? `${Math.ceil(g[2])}×${Math.ceil(g[3])}` : "empty";
                      })()}{" "}
                      · {d().placements.length} placement
                      {d().placements.length === 1 ? "" : "s"}
                    </div>
                    <Show when={selectedAsset()?.descriptor.scale_class === "spline-segment"}>
                      <div class="row">
                        <button
                          class={wallDraw() ? "selected" : ""}
                          onClick={() => setWallDraw(!wallDraw())}
                        >
                          Draw wall
                        </button>
                      </div>
                    </Show>
                    <Show when={wallAsset()}>
                      {(a) => (
                        <p class="hint">
                          drawing wall of <b>{a().descriptor.name}</b> — click to add
                          points, Enter or double-click to finish, Esc to cancel
                        </p>
                      )}
                    </Show>
                    <Show when={!wallAsset() && selectedAsset()}>
                      {(a) => (
                        <p class="hint">
                          placing <b>{a().descriptor.name}</b> — click the canvas to
                          drop, Esc to stop
                        </p>
                      )}
                    </Show>
                  </div>
                )}
              </Show>
            </Show>
          </section>
        </Show>
        <Show when={mode() === "inspect"}>
        <Show
          when={index()}
          fallback={
            <div class="connect">
              <button onClick={onPick}>Open hackable datadir…</button>
              <Show when={needsReconnect()}>
                <button onClick={onReconnect}>Reconnect previous folder</button>
              </Show>
              <p class="hint">
                Pick a folder produced by <code>convert_datadir</code> (e.g.{" "}
                <code>datadirs/fullgame_gog_hackable</code>)
              </p>
            </div>
          }
        >
          {(idx) => (
            <>
              <section>
                <h2>Maps</h2>
                <ul class="maps">
                  <For each={[...idx().maps.keys()].sort()}>
                    {(name) => (
                      <li
                        class={mapName() === name ? "selected" : ""}
                        onClick={() => selectMap(name)}
                      >
                        {name}
                      </li>
                    )}
                  </For>
                </ul>
              </section>
              <Show when={mapName()}>
                <section>
                  <h2>Ambiance</h2>
                  <div class="row">
                    <For each={idx().maps.get(mapName()!) ?? []}>
                      {(amb) => (
                        <button
                          class={ambiance() === amb ? "selected" : ""}
                          onClick={() => selectAmbiance(amb)}
                        >
                          {amb}
                        </button>
                      )}
                    </For>
                  </div>
                </section>
                <section>
                  <h2>Mission</h2>
                  <select
                    value={missionName() ?? ""}
                    onChange={(e) => selectMission(e.currentTarget.value || null)}
                  >
                    <option value="">— none —</option>
                    <For each={missionsForMap(idx(), mapName()!)}>
                      {(m) => <option value={m}>{m}</option>}
                    </For>
                  </select>
                </section>
                <section>
                  <h2>Overlays</h2>
                  <For each={Object.keys(TOGGLE_LABELS) as (keyof OverlayToggles)[]}>
                    {(key) => (
                      <label class="check">
                        <input
                          type="checkbox"
                          checked={toggles()[key]}
                          onChange={() => toggle(key)}
                        />
                        {TOGGLE_LABELS[key]}
                      </label>
                    )}
                  </For>
                </section>
                <Show when={layerCount() > 1}>
                  <section>
                    <h2>Layer</h2>
                    <div class="row">
                      <button
                        class={layerFilter() === null ? "selected" : ""}
                        onClick={() => setLayerFilter(null)}
                      >
                        all
                      </button>
                      <For each={[...Array(layerCount()).keys()]}>
                        {(i) => (
                          <button
                            class={layerFilter() === i ? "selected" : ""}
                            onClick={() => setLayerFilter(i)}
                          >
                            {i}
                          </button>
                        )}
                      </For>
                    </div>
                  </section>
                </Show>
              </Show>
            </>
          )}
        </Show>
        </Show>
        <LibrarySection
          library={library}
          needsReconnect={libraryNeedsReconnect}
          onPick={onPickLibrary}
          onReconnect={onReconnectLibrary}
          onRefresh={() => {
            const lib = library();
            if (lib) void openLibrary(lib.root);
          }}
          selected={selectedAsset}
          onSelect={selectAsset}
        />
        <div class="statusbar">
          <Show when={busy()}>{(b) => <span class="busy">{b()}</span>}</Show>
          <Show when={error()}>{(e) => <span class="error">{e()}</span>}</Show>
          <span class="coords">
            {cursor()[0].toFixed(0)}, {cursor()[1].toFixed(0)}
          </span>
        </div>
      </aside>
      <Show
        when={mode() === "compose"}
        fallback={
          <Viewer
            image={image}
            level={level}
            mission={mission}
            toggles={toggles}
            layerFilter={layerFilter}
            selection={selectionBbox}
            onCursor={(x, y) => setCursor([x, y])}
          />
        }
      >
        <ComposeViewer
          draft={draft}
          docId={draftDocId}
          library={library}
          placing={() => (wallAsset() ? null : selectedAsset())}
          onCancelPlace={() => selectAsset(null)}
          wallAsset={wallAsset}
          onCommitWall={commitWall}
          onCancelWall={() => setWallDraw(false)}
          selected={selection}
          onSelect={setSelection}
          onPlace={placeAsset}
          onMove={movePlacement}
          onDelete={deleteSelection}
          onMoveWallPoint={moveWallPoint}
          onInsertWallPoint={insertWallPoint}
          onRemoveWallPoint={removeWallPoint}
          onCursor={(x, y) => setCursor([x, y])}
        />
      </Show>
      <Show when={mode() === "inspect" && selectedAsset()}>
        {(asset) => <AssetDetail asset={asset} onClose={() => setSelectedAsset(null)} />}
      </Show>
      <Show when={mode() === "compose" ? draft() : null}>
        {(d) => (
          <PlacementsPanel
            draft={d}
            library={library}
            selected={selection}
            onSelect={setSelection}
            onDelete={deleteSelection}
          />
        )}
      </Show>
    </div>
  );
}
