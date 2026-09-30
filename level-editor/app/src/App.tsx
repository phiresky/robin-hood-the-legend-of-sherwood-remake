// Published assets load over HTTP; map edits stay in browser storage.
import { Show, createEffect, createSignal, onCleanup } from "solid-js";
import type { DatadirIndex } from "./datadir";
import { openHttpGameData } from "./http-game-data.ts";
import StatusDialog from "./StatusDialog";
import ErrorDialog from "./ErrorDialog";
import RobinMascot from "./RobinMascot";
import Editor3D, { type LibraryRef } from "./Editor3D";
import { connectionAttempts, connectLatest } from "./connection-attempt";
import { openHttpLibrary } from "./http-library.ts";

export default function App() {
  const [index, setIndex] = createSignal<DatadirIndex | null>(null);
  // wrapped: a directory handle is async-iterable, and Solid 2 flattens iterables returned by effect computes
  const [library, setLibrary] = createSignal<LibraryRef | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const [statusBusy, setStatusBusy] = createSignal(false);
  const onStatus = (message: string | null, busy = false) => {
    setStatusBusy(busy);
    setStatus(message);
  };
  const datadirAttempts = connectionAttempts();
  const libraryAttempts = connectionAttempts();
  onCleanup(() => {
    datadirAttempts.dispose();
    libraryAttempts.dispose();
  });

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

  // Load game data and published assets over HTTP on startup.
  createEffect(
    () => undefined,
    () => {
      const datadirReady = connectDatadir(async (current) => {
        const next = await openHttpGameData();
        if (current()) setIndex(next);
      });
      void connectLibrary(async (current) => {
        const lib = await openHttpLibrary();
        // Opening a map before the initial index settles lets its arrival
        // invalidate that load. Fetch both in parallel, then expose the chooser.
        await datadirReady;
        if (!current()) return;
        setLibrary(lib);
      });
    },
  );

  return (
    <div class="app editor-app">
      <Editor3D
        index={index}
        library={library}
        onError={(message) => {
          setStatus(null);
          setError(message);
        }}
        onStatus={onStatus}
        toolbarStart={() => (
          <>
            <RobinMascot />
            <h1 title="Robin Hood Map Editor">Editor</h1>
          </>
        )}
      />
      <Show when={status()}>
        {(message) => (
          <StatusDialog message={message()} busy={statusBusy()} onClose={() => setStatus(null)} />
        )}
      </Show>
      <Show when={error()}>
        {(message) => <ErrorDialog message={message()} onClose={() => setError(null)} />}
      </Show>
    </div>
  );
}
