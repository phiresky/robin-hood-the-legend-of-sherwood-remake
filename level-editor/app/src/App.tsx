import { For, Show, createEffect, createSignal } from "solid-js";
import type { Mission, ProtoLevel } from "@rle/shared";
import {
  getStoredDatadirHandle,
  pickDatadir,
  requestDatadirPermission,
  restoreDatadir,
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

export default function App() {
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

  // one-shot startup: restore a previously granted datadir handle
  createEffect(
    () => undefined,
    () => {
      void (async () => {
        const restored = await restoreDatadir();
        if (restored) return openRoot(restored);
        if (await getStoredDatadirHandle()) setNeedsReconnect(true);
      })();
    },
  );

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

  const toggle = (key: keyof OverlayToggles) =>
    setToggles((t) => ({ ...t, [key]: !t[key] }));

  const layerCount = () => level()?.motion_data.layers.length ?? 0;

  return (
    <div class="app">
      <aside class="sidebar">
        <h1>RH Level Editor</h1>
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
        <div class="statusbar">
          <Show when={busy()}>{(b) => <span class="busy">{b()}</span>}</Show>
          <Show when={error()}>{(e) => <span class="error">{e()}</span>}</Show>
          <span class="coords">
            {cursor()[0].toFixed(0)}, {cursor()[1].toFixed(0)}
          </span>
        </div>
      </aside>
      <Viewer
        image={image}
        level={level}
        mission={mission}
        toggles={toggles}
        layerFilter={layerFilter}
        onCursor={(x, y) => setCursor([x, y])}
      />
    </div>
  );
}
