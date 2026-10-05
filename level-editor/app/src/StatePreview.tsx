import { For, Show, createEffect, createSignal, onCleanup } from "solid-js";
import type { Level3D } from "@rle/shared";
import type { EditorViewport } from "./editor-viewport.ts";
import type { StateDeliveryContract } from "../../shared/src/state-delivery.ts";
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
    if (view === "art") props.viewport.setDeliveredStateMode("native-art");
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
    setStatus("Loading state preview…");
    setPlaying(false);
    props.viewport.clearStateDelivery();
    try {
      const loaded = await loadMissionStatePreview(root, entry);
      if (context !== generation || attempt !== request) return;
      const ready = await props.viewport.setStateDelivery(loaded.contract, root, loaded.source);
      if (!ready || context !== generation || attempt !== request) return;
      setContract(loaded.contract);
      setFamily(loaded.contract.families[0]!.id);
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
      setSelected("");
      setStatus("");
      setPlaying(false);
      props.viewport.clearStateDelivery();
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
    () => ({ active: props.active, loaded: contract(), familyId: family(), view: mode() }),
    ({ active, loaded, familyId, view }) => {
      if (!active) {
        props.viewport.setStatePresentationMode("physical");
        setPlaying(false);
      } else if (loaded && familyId) applyMode(view, familyId);
    },
  );
  const timer = setInterval(() => {
    if (!contract()) return;
    const s = props.viewport.deliveredStateStatus(family());
    if (!s.ready) return;
    setTick(Math.min(lastTick(), s.tick ?? 0));
    setPlaying(s.playing);
    if (s.playing && (s.tick ?? 0) >= lastTick()) {
      props.viewport.setDeliveredStatePlaying(false);
      setPlaying(false);
    }
  }, 100);
  onCleanup(() => {
    generation++;
    request++;
    clearInterval(timer);
    props.viewport.clearStateDelivery();
  });
  function play() {
    if (playing()) {
      props.viewport.setDeliveredStatePlaying(false);
      setPlaying(false);
      return;
    }
    const s = props.viewport.deliveredStateStatus(family());
    if (s.tick === undefined || s.tick >= lastTick())
      props.viewport.activateDeliveredState(family());
    props.viewport.setDeliveredStatePlaying(true);
    setPlaying(true);
  }
  function reset() {
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
        <Show when={contract()}>
          <p class="hint">
            Original artwork plays the recorded transition. 3D views show the initial and final
            models.
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
              value={mode()}
              onChange={(event) => {
                props.viewport.setDeliveredStatePlaying(false);
                setMode(event.currentTarget.value as "art" | "initial" | "applied");
              }}
            >
              <option value="art">Original artwork</option>
              <option value="initial">3D initial</option>
              <option value="applied">3D final</option>
            </select>
          </label>
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
                  props.viewport.setDeliveredStatePlaying(false);
                  const t = Number(event.currentTarget.value);
                  props.viewport.seekDeliveredState(family(), t);
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
