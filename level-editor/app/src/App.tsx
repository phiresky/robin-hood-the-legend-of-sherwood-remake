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
import { connectionAttempts, connectLatest } from "./connection-attempt";

export default function App() {
  const [index, setIndex] = createSignal<DatadirIndex | null>(null);
  const [needsReconnect, setNeedsReconnect] = createSignal(false);
  // wrapped: a directory handle is async-iterable, and Solid 2 flattens iterables returned by effect computes
  const [library, setLibrary] = createSignal<LibraryRef | null>(null);
  const [libraryNeedsReconnect, setLibraryNeedsReconnect] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const datadirAttempts = connectionAttempts();
  const libraryAttempts = connectionAttempts();
  onCleanup(() => {
    datadirAttempts.dispose();
    libraryAttempts.dispose();
  });

  async function openRoot(
    handle: FileSystemDirectoryHandle,
    current: () => boolean,
  ) {
    if (!current()) return;
    setStatus("scanning datadir…");
    const next = await scanDatadir(handle);
    if (!current()) return;
    setIndex(next);
    setNeedsReconnect(false);
  }

  const connectDatadir = (operation: (current: () => boolean) => Promise<void>) =>
    connectLatest(
      datadirAttempts,
      async (current) => {
        setError(null);
        setStatus(null);
        await operation(current);
      },
      (error) => setError(String(error)),
      () => setStatus(null),
    );
  const connectLibrary = (operation: (current: () => boolean) => Promise<void>) =>
    connectLatest(
      libraryAttempts,
      async (current) => {
        setError(null);
        await operation(current);
      },
      (error) => setError(String(error)),
    );

  // one-shot startup: restore previously granted handles
  createEffect(
    () => undefined,
    () => {
      void connectDatadir(async (current) => {
        const restored = await restoreDatadir();
        if (!current()) return;
        if (restored) await openRoot(restored, current);
        else {
          const stored = await getStoredDatadirHandle();
          if (current()) setNeedsReconnect(stored !== null);
        }
      });
      void connectLibrary(async (current) => {
        const lib = await restoreLibrary();
        if (!current()) return;
        if (lib) setLibrary({ handle: lib });
        else {
          const stored = await getStoredLibraryHandle();
          if (current()) setLibraryNeedsReconnect(stored !== null);
        }
      });
    },
  );

  function onPick() {
    return connectDatadir(async (current) => {
      const handle = await pickDatadir(current);
      if (current()) await openRoot(handle, current);
    });
  }
  function onReconnect() {
    return connectDatadir(async (current) => {
      const handle = await getStoredDatadirHandle();
      if (!current() || !handle) return;
      const granted = await requestDatadirPermission(handle);
      if (current() && granted) await openRoot(handle, current);
    });
  }
  function onPickLibrary() {
    return connectLibrary(async (current) => {
      const handle = await pickLibrary(current);
      if (!current()) return;
      setLibrary({ handle });
      setLibraryNeedsReconnect(false);
    });
  }
  function onReconnectLibrary() {
    return connectLibrary(async (current) => {
      const handle = await getStoredLibraryHandle();
      if (!current() || !handle) return;
      const granted = await requestDatadirPermission(handle, "readwrite");
      if (!current() || !granted) return;
      setLibrary({ handle });
      setLibraryNeedsReconnect(false);
    });
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
