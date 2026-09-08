// App shell: connect the hackable datadir (game data, read) and the
// library (reconstructions and level documents, read/write), then hand
// over to the 3D editor.
import { Show, createEffect, createSignal, onCleanup } from "solid-js";
import {
  getStoredDatadirHandle,
  getStoredLibraryHandle,
  pickDatadir,
  pickLibrary,
  requestDatadirPermission,
  restoreDatadir,
  restoreLibrary,
} from "./fs";
import { scanDatadir, type DatadirIndex } from "./datadir";
import Editor3D, { type LibraryRef } from "./Editor3D";

export default function App() {
  const [index, setIndex] = createSignal<DatadirIndex | null>(null);
  const [needsReconnect, setNeedsReconnect] = createSignal(false);
  // wrapped: a directory handle is async-iterable, and Solid 2 flattens iterables returned by effect computes
  const [library, setLibrary] = createSignal<LibraryRef | null>(null);
  const [libraryNeedsReconnect, setLibraryNeedsReconnect] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  let generation = 0;
  let disposed = false;
  onCleanup(() => {
    disposed = true;
    generation++;
  });

  async function openRoot(handle: FileSystemDirectoryHandle) {
    const request = ++generation;
    setError(null);
    setStatus("scanning datadir…");
    try {
      const next = await scanDatadir(handle);
      if (disposed || request !== generation) return;
      setIndex(next);
      setNeedsReconnect(false);
    } catch (e) {
      if (!disposed && request === generation) setError(String(e));
    } finally {
      if (!disposed && request === generation) setStatus(null);
    }
  }

  // one-shot startup: restore previously granted handles
  createEffect(
    () => undefined,
    () => {
      void (async () => {
        const restored = await restoreDatadir();
        if (restored) await openRoot(restored);
        else if (await getStoredDatadirHandle()) setNeedsReconnect(true);
        const lib = await restoreLibrary();
        if (lib) setLibrary({ handle: lib });
        else if (await getStoredLibraryHandle()) setLibraryNeedsReconnect(true);
      })().catch((error) => {
        if (!disposed) setError(String(error));
      });
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
    if (handle && (await requestDatadirPermission(handle)))
      await openRoot(handle);
  }
  async function onPickLibrary() {
    try {
      setLibrary({ handle: await pickLibrary() });
      setLibraryNeedsReconnect(false);
    } catch (e) {
      if ((e as DOMException).name !== "AbortError") setError(String(e));
    }
  }
  async function onReconnectLibrary() {
    const handle = await getStoredLibraryHandle();
    if (handle && (await requestDatadirPermission(handle, "readwrite"))) {
      setLibrary({ handle });
      setLibraryNeedsReconnect(false);
    }
  }

  return (
    <div class="app editor-app">
      <header class="topbar">
        <h1>RH Level Editor</h1>
        <Show
          when={index()}
          fallback={
            <button
              class="connect"
              onClick={needsReconnect() ? onReconnect : onPick}
            >
              {needsReconnect()
                ? "Reconnect datadir"
                : "Open hackable datadir…"}
            </button>
          }
        >
          {(idx) => (
            <span class="connected">
              datadir: {idx().maps.size} maps{" "}
              <button onClick={onPick}>change</button>
            </span>
          )}
        </Show>
        <Show
          when={library()}
          fallback={
            <button
              class="connect"
              onClick={
                libraryNeedsReconnect() ? onReconnectLibrary : onPickLibrary
              }
            >
              {libraryNeedsReconnect() ? "Reconnect library" : "Open library…"}
            </button>
          }
        >
          {(lib) => (
            <span class="connected">
              library: {lib().handle.name}{" "}
              <button onClick={onPickLibrary}>change</button>
            </span>
          )}
        </Show>
        <span class="spacer" />
        <Show when={status()}>{(s) => <span class="busy">{s()}</span>}</Show>
        <Show when={error()}>
          {(e) => (
            <span class="error" onClick={() => setError(null)}>
              {e()}
            </span>
          )}
        </Show>
      </header>
      <Editor3D
        index={index}
        library={library}
        onError={setError}
        onStatus={setStatus}
      />
    </div>
  );
}
