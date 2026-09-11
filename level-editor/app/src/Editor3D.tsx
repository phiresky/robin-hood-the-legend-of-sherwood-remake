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
import type * as THREE from "three";
import {
  IDENTITY_TRANSFORM,
  groupParts,
  isIdentity,
  type GameTransform,
  type Level3D,
  type Level3DGroup,
  type Level3DObject,
  type ProtoLevel,
} from "@rle/shared";
import {
  SessionPublication,
  type SessionSnapshot,
} from "./session-publication";
import {
  duplicateSelection,
  deleteSelection,
  patchPart,
  patchGroup,
  type Selection,
} from "./document-commands";
import { prepareMapCandidate } from "./map-candidate";
import { EditorViewport } from "./editor-viewport";
import { disposeObjectResources } from "./resources";
import { listFiles, subdir, writeText } from "./fs";
import type { DatadirIndex } from "./datadir";

export type { Selection } from "./document-commands";

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

export default function Editor3D(props: EditorProps) {
  const [maps, setMaps] = createSignal<string[]>([]);
  const [revision, setRevision] = createSignal<SessionSnapshot<Level3D> | null>(
    null,
  );
  const mapName = () => revision()?.name ?? null;
  const doc = () => revision()?.document ?? null;
  const dirty = () => revision()?.dirty ?? false;
  const history = () => revision() ?? { past: [], future: [] };
  const session = new SessionPublication<Level3D, FileSystemDirectoryHandle>(
    (snapshot, reason) => {
      setRevision(snapshot);
      if (reason === "revision") viewport.syncViews(snapshot.document);
    },
  );
  let saving = false;
  let disposed = false;
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
  const viewport = new EditorViewport({
    document: doc,
    selection: selected,
    level,
    showObstacles,
    showElevation,
    onSelection: (selection) => {
      setSelected(selection);
      if (selection?.kind === "part") {
        const group = doc()?.objects.find((o) => o.id === selection.id)?.group;
        if (group) setExpanded((current) => new Set(current).add(group));
      }
    },
    commitTransform: setTransform,
  });
  const select = (selection: Selection) => viewport.select(selection);

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
  function pushHistory(next: Level3D) {
    session.edit(next);
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
      const candidate = await prepareMapCandidate(name, lib.handle, idx);
      preparedAsset = candidate.asset;
      const {
        document: d,
        directory: dir,
        level: lvl,
        sources: nextSources,
        ground: nextGround,
        suspects: nextSuspects,
      } = candidate;
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
      viewport.replaceMap(preparedAsset, nextGround, nextSources);
      preparedAsset = null;
      session.publish(generation, name, d, dir, candidate.saved);
      setLevel(lvl);
      setSuspects(nextSuspects);
      viewport.syncViews(d);
      viewport.buildOverlays();
      viewport.gameCamera(true);
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

  createEffect(
    () => ({ obstacles: showObstacles(), elevation: showElevation() }),
    () => viewport.buildOverlays(),
  );
  createEffect(
    () => gizmoVertical(),
    (v) => viewport.setGizmoVertical(v),
  );

  // ── actions ──
  function duplicateSelected() {
    const document = doc();
    const selection = selected();
    if (!document || !selection) return;
    const result = duplicateSelection(document, selection);
    pushHistory(result.document);
    select(result.selection);
  }
  function deleteSelected() {
    const document = doc();
    const selection = selected();
    if (!document || !selection) return;
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
    else if (e.key === "g") viewport.gameCamera();
    else if (e.key === "f") viewport.frameContent();
    else if (e.key === "Escape") select(null);
  }
  window.addEventListener("keydown", onKey, {
    signal: viewport.listeners.signal,
  });
  onCleanup(() => {
    disposed = true;
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
        <button
          disabled={!doc()}
          onClick={() => viewport.gameCamera()}
          title="g"
        >
          Game camera
        </button>
        <button
          disabled={!doc()}
          onClick={() => viewport.frameContent()}
          title="f"
        >
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
        <div class="editor-canvas" ref={(element) => viewport.setup(element)} />
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
