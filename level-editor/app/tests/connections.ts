import { get, set, del } from "idb-keyval";
import { pickDatadir, pickLibrary } from "../src/fs";
import { connectionAttempts } from "../src/connection-attempt";

/** Real IndexedDB transactions and cloneable OPFS handles, with only the native
 * picker mocked. Runs in the lifecycle runner's disposable browser profile. */
export async function checkConnectionPersistence() {
  const root = await navigator.storage.getDirectory();
  const old = await root.getDirectoryHandle("old-choice", { create: true });
  const latest = await root.getDirectoryHandle("latest-choice", { create: true });
  const original = window.showDirectoryPicker;
  const assertStored = async (key: string, expected: FileSystemDirectoryHandle) => {
    const stored = await get<FileSystemDirectoryHandle>(key);
    if (!stored || !(await stored.isSameEntry(expected)))
      throw new Error(`Stale connection replaced ${key}`);
  };
  try {
    for (const [key, pick] of [
      ["datadir-handle", pickDatadir],
      ["library-handle", pickLibrary],
    ] as const) {
      const attempts = connectionAttempts();
      let finishOld!: (handle: FileSystemDirectoryHandle) => void;
      window.showDirectoryPicker = () => new Promise((resolve) => { finishOld = resolve; });
      const first = pick(attempts.begin());
      window.showDirectoryPicker = async () => latest;
      await pick(attempts.begin());
      finishOld(old);
      await first;
      await assertStored(key, latest);

      // Retire after the picker resolves, but before the IndexedDB updater runs.
      window.showDirectoryPicker = () => new Promise((resolve) => { finishOld = resolve; });
      const queued = pick(attempts.begin());
      finishOld(old);
      await Promise.resolve();
      attempts.dispose();
      await queued;
      await assertStored(key, latest);
    }
    // Writes made by unrelated slots must not supersede one another.
    await set("datadir-handle", old);
    await assertStored("library-handle", latest);
  } finally {
    window.showDirectoryPicker = original;
    await del("datadir-handle");
    await del("library-handle");
  }
}
