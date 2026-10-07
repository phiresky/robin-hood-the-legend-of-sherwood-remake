import { For, Show, createEffect, createSignal, onCleanup } from "solid-js";
import type { Level3D } from "@rle/shared";
import type { EditorViewport } from "./editor-viewport.ts";
import {
  nativeLoopPreviewPeriod,
  nativePatchPreviewTerminal,
  nativePatchPreviewLoops,
  type NativePatchPreviewContract,
  stateDeliveryLoopsAfterTransition,
  type NativeLoopPreviewContract,
  type StateDeliveryContract,
} from "../../shared/src/state-delivery.ts";
import {
  loadMissionStateCatalog,
  loadMissionStatePreview,
  type MissionStateCatalogEntry,
} from "./mission-state-catalog.ts";

export default function StatePreview(props: {
  document(): Level3D | null;
  library?(): FileSystemDirectoryHandle | null;
  viewport: EditorViewport;
  active: boolean;
  onError(message: string): void;
}) {
  const [entries, setEntries] = createSignal<MissionStateCatalogEntry[]>([]),
    [selected, setSelected] = createSignal(""),
    [contract, setContract] = createSignal<StateDeliveryContract | null>(null),
    [loopContract, setLoopContract] = createSignal<NativeLoopPreviewContract | null>(null),
    [patchContract, setPatchContract] = createSignal<NativePatchPreviewContract | null>(null),
    [patchStarted, setPatchStarted] = createSignal(false),
    [family, setFamily] = createSignal(""),
    [status, setStatus] = createSignal(""),
    [mode, setMode] = createSignal<"art" | "initial" | "applied">("art"),
    [tick, setTick] = createSignal(0),
    [playing, setPlaying] = createSignal(false);
  let generation = 0,
    request = 0,
    lastLibrary: FileSystemDirectoryHandle | null | undefined,
    lastMap = "",
    lastMission = "";
  const current = () => contract()?.families.find((f) => f.id === family());
  const lastTick = () => {
    const patch = patchContract();
    if (patch) return nativePatchPreviewTerminal(patch);
    const loop = loopContract();
    if (loop) return nativeLoopPreviewPeriod(loop) - 1;
    const c = contract(),
      f = current();
    if (!c || !f) return 0;
    return Math.max(
      f.body_terminal_tick,
      ...f.background_ids.map(
        (id) =>
          c.native.background_states
            ?.find((s) => s.id === id)
            ?.transition.reduce((n, r) => n + r.delay + 1, 0) ?? 0,
      ),
    );
  };
  function applyMode(view: "art" | "initial" | "applied", familyId: string) {
    if (loopContract() || patchContract()) props.viewport.setStatePresentationMode("native-art");
    else if (view === "art") props.viewport.setDeliveredStateMode("native-art");
    else {
      props.viewport.setDeliveredStateMode("physical-endpoint");
      props.viewport.selectDeliveredEndpoint(familyId, view === "initial" ? "initial" : "applied");
    }
    setPlaying(false);
  }
  async function choose(
    entry: MissionStateCatalogEntry,
    root: FileSystemDirectoryHandle,
    context: number,
  ) {
    const attempt = ++request;
    setSelected(entry.id);
    setContract(null);
    setLoopContract(null);
    setPatchContract(null);
    setPatchStarted(false);
    setStatus("Loading state preview…");
    setPlaying(false);
    props.viewport.clearNativeArtPresentation();
    try {
      const loaded = await loadMissionStatePreview(root, entry);
      if (context !== generation || attempt !== request) return;
      const ready =
        loaded.kind === "native-patch"
          ? await props.viewport.setNativePatchPresentation(loaded.contract, root, loaded.source)
          : loaded.kind === "native-loop"
            ? await props.viewport.setNativeArtPresentation(
                loaded.contract.native,
                root,
                loaded.source,
              )
            : await props.viewport.setStateDelivery(loaded.contract, root, loaded.source);
      if (!ready || context !== generation || attempt !== request) return;
      if (loaded.kind === "native-patch") {
        setPatchContract(loaded.contract);
        setFamily(loaded.contract.focus_patch_id);
      } else if (loaded.kind === "native-loop") {
        setLoopContract(loaded.contract);
        setFamily(loaded.contract.focus_element_id);
      } else {
        setContract(loaded.contract);
        setFamily(loaded.contract.families[0]!.id);
      }
      setMode("art");
      setTick(0);
      setStatus("");
    } catch (error) {
      if (context !== generation || attempt !== request) return;
      setStatus(String(error));
      props.onError(String(error));
    }
  }
  createEffect(
    () => ({
      root: props.library?.() ?? null,
      map: props.document()?.map ?? "",
      mission: props.document()?.mission?.importedFrom ?? "",
    }),
    ({ root, map, mission }) => {
      if (root === lastLibrary && map === lastMap && mission === lastMission) return;
      lastLibrary = root;
      lastMap = map;
      lastMission = mission;
      const context = ++generation;
      request++;
      setEntries([]);
      setContract(null);
      setLoopContract(null);
      setPatchContract(null);
      setPatchStarted(false);
      setSelected("");
      setStatus("");
      setPlaying(false);
      props.viewport.clearNativeArtPresentation();
      if (!root || !map || !mission) return;
      void loadMissionStateCatalog(root, map, mission)
        .then((rows) => {
          if (context !== generation) return;
          setEntries(rows);
          if (rows[0]) void choose(rows[0], root, context);
        })
        .catch((error) => {
          if (context !== generation) return;
          setStatus(String(error));
          props.onError(String(error));
        });
    },
  );
  createEffect(
    () => ({
      active: props.active,
      loaded: contract() ?? loopContract() ?? patchContract(),
      familyId: family(),
      view: mode(),
    }),
    ({ active, loaded, familyId, view }) => {
      if (!active) {
        props.viewport.setStatePresentationMode("physical");
        setPlaying(false);
      } else if (loaded && familyId) applyMode(view, familyId);
    },
  );
  const timer = setInterval(() => {
    if (!contract() && !loopContract() && !patchContract()) return;
    const s =
      loopContract() || patchContract()
        ? props.viewport.nativeArtStatus()
        : props.viewport.deliveredStateStatus(family());
    if (!s.ready) return;
    setTick(loopContract() ? (s.tick ?? 0) % (lastTick() + 1) : Math.min(lastTick(), s.tick ?? 0));
    setPlaying(s.playing);
    if (
      !loopContract() &&
      s.playing &&
      (s.tick ?? 0) >= lastTick() &&
      !(patchContract()
        ? nativePatchPreviewLoops(patchContract()!)
        : stateDeliveryLoopsAfterTransition(contract()!, family()))
    ) {
      if (patchContract()) props.viewport.setNativeArtPlaying(false);
      else props.viewport.setDeliveredStatePlaying(false);
      setPlaying(false);
    }
  }, 100);
  onCleanup(() => {
    generation++;
    request++;
    clearInterval(timer);
    props.viewport.clearNativeArtPresentation();
  });
  function play() {
    const patch = patchContract();
    if (patch) {
      if (
        !playing() &&
        (!patchStarted() || (tick() >= lastTick() && !nativePatchPreviewLoops(patch)))
      ) {
        props.viewport.seekNativePatch(family(), "forward", 0);
        setPatchStarted(true);
        setTick(0);
      }
      props.viewport.setNativeArtPlaying(!playing());
      setPlaying(!playing());
      return;
    }
    if (loopContract()) {
      props.viewport.setNativeArtPlaying(!playing());
      setPlaying(!playing());
      return;
    }
    if (playing()) {
      props.viewport.setDeliveredStatePlaying(false);
      setPlaying(false);
      return;
    }
    const s = props.viewport.deliveredStateStatus(family());
    if (
      s.tick === undefined ||
      (s.tick >= lastTick() && !stateDeliveryLoopsAfterTransition(contract()!, family()))
    )
      props.viewport.activateDeliveredState(family());
    props.viewport.setDeliveredStatePlaying(true);
    setPlaying(true);
  }
  function reset() {
    if (patchContract()) {
      props.viewport.seekNativePatch(family(), "initial", 0);
      setPatchStarted(false);
      setTick(0);
      setPlaying(false);
      return;
    }
    if (loopContract()) {
      props.viewport.setNativeArtPlaying(false);
      props.viewport.seekNativeArt(0);
      setTick(0);
      setPlaying(false);
      return;
    }
    props.viewport.setDeliveredStatePlaying(false);
    props.viewport.resetDeliveredState(family());
    setTick(0);
    setPlaying(false);
  }
  return (
    <Show when={entries().length || status()}>
      <section aria-label="State preview">
        <h3>State preview</h3>
        <Show when={status()}>
          <p role="status">{status()}</p>
        </Show>
        <Show when={entries().length}>
          <label>
            State
            <select
              aria-label="State preview asset"
              value={selected()}
              onChange={(event) => {
                const entry = entries().find((e) => e.id === event.currentTarget.value),
                  root = props.library?.();
                if (entry && root) void choose(entry, root, generation);
              }}
            >
              <For each={entries()}>
                {(entry) => <option value={entry.id}>{entry.name}</option>}
              </For>
            </select>
          </label>
        </Show>
        <Show when={contract() || loopContract() || patchContract()}>
          <p class="hint">
            {patchContract()
              ? "Original artwork previews this change. Physical states are not included in this preview."
              : loopContract()
                ? "Original artwork repeats the selected animation. Nearby animation keeps its own timing."
                : "Original artwork plays the recorded transition. 3D views show the object when present in each state."}
          </p>
          <Show when={(contract()?.families.length ?? 0) > 1}>
            <label>
              Part
              <select
                aria-label="State preview part"
                value={family()}
                onChange={(event) => {
                  props.viewport.setDeliveredStatePlaying(false);
                  setFamily(event.currentTarget.value);
                  setTick(0);
                }}
              >
                <For each={contract()?.families ?? []}>
                  {(f) => <option value={f.id}>{f.id}</option>}
                </For>
              </select>
            </label>
          </Show>
          <label>
            View
            <select
              aria-label="State preview view"
              disabled={!!loopContract() || !!patchContract()}
              value={mode()}
              onChange={(event) => {
                props.viewport.setDeliveredStatePlaying(false);
                setMode(event.currentTarget.value as "art" | "initial" | "applied");
              }}
            >
              <option value="art">Original artwork</option>
              <Show when={contract()}>
                <option value="initial">3D initial</option>
                <option value="applied">3D final</option>
              </Show>
            </select>
          </label>
          <Show
            when={
              mode() !== "art" &&
              !!current() &&
              !Array.isArray(current()!.physical[mode() === "initial" ? "initial" : "applied"])
            }
          >
            <p class="hint">No object is present in this state.</p>
          </Show>
          <Show when={mode() === "art"}>
            <div class="actions">
              <button type="button" onClick={play}>
                {playing() ? "Pause" : "Play"}
              </button>
              <button type="button" onClick={reset}>
                Reset
              </button>
            </div>
            <label>
              Frame {tick()} / {lastTick()}
              <input
                aria-label="State preview frame"
                type="range"
                min="0"
                max={lastTick()}
                step="1"
                value={tick()}
                onInput={(event) => {
                  const t = Number(event.currentTarget.value);
                  if (patchContract()) {
                    props.viewport.seekNativePatch(family(), "forward", t);
                    setPatchStarted(true);
                  } else if (loopContract()) {
                    props.viewport.setNativeArtPlaying(false);
                    props.viewport.seekNativeArt(t);
                  } else {
                    props.viewport.setDeliveredStatePlaying(false);
                    props.viewport.seekDeliveredState(family(), t);
                  }
                  setTick(t);
                  setPlaying(false);
                }}
              />
            </label>
          </Show>
        </Show>
      </section>
    </Show>
  );
}
