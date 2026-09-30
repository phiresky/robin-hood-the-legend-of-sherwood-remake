import { publishedMapLabel } from "../src/map-label";
import { Show, createSignal } from "solid-js";
import StatusDialog from "../src/StatusDialog";
import ErrorDialog from "../src/ErrorDialog";
import { render } from "@solidjs/web";
import RobinMascot from "../src/RobinMascot";
import Editor3D from "../src/Editor3D";
import { openHttpLibrary } from "../src/http-library";
import { encodeMapThumbnail } from "../src/map-thumbnail";
import { checkHttpLibrary } from "./http-library";
import "../src/styles.css";
const result = document.querySelector("#result")!;
function assert(value: unknown, message: string) {
  if (!value) throw new Error(message);
}
async function until(test: () => boolean) {
  for (let n = 0; n < 150; n++) {
    if (test()) return;
    await new Promise((resolve) => setTimeout(resolve, 30));
  }
  throw new Error("Timed out: " + document.querySelector("#root")?.textContent);
}
const click = (label: string) => {
  const button = [...document.querySelectorAll("button")].find(
    (button) => button.getAttribute("aria-label") === label || button.textContent?.trim() === label,
  );
  assert(button, `Missing ${label}`);
  button!.click();
};
const current = () => document.querySelector("[data-map-name]")?.getAttribute("data-map-name");
const warns = () => {
  const event = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(event);
  return event.defaultPrevented;
};
async function main() {
  await checkHttpLibrary();
  const root = await navigator.storage.getDirectory();
  const storage = await root.getDirectoryHandle("map-selection-test", { create: true });
  const originalFetch = window.fetch;
  const originalConfirm = window.confirm;
  const originalPrompt = window.prompt;
  let approve = false,
    confirmations = 0;
  window.confirm = () => {
    confirmations++;
    return approve;
  };
  window.prompt = () => "Renamed map";
  const blank = {
    version: 1,
    map: "York",
    size: null,
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    sceneAssets: [],
    groups: [],
    objects: [],
  };
  const remote = new Map([
    ["/fixture/3d-assets/index.json", JSON.stringify({ version: 1, assets: [] })],
    ["/fixture/scenes/index.json", JSON.stringify(["York.rhlos-map.json"])],
    ["/fixture/scenes/York.rhlos-map.json", JSON.stringify(blank)],
  ]);
  let failRead = false;
  let blockedRead: Promise<void> | null = null;
  window.fetch = async (url) => {
    if (String(url).endsWith("York.rhlos-map.json") && blockedRead) await blockedRead;
    if (failRead && String(url).endsWith("York.rhlos-map.json"))
      return new Response("Failed", { status: 500 });
    return new Response(remote.get(String(url)) ?? "", {
      status: remote.has(String(url)) ? 200 : 404,
    });
  };
  const library = await openHttpLibrary("/fixture/", storage);
  const errors: string[] = [];
  const [errorMessage, setErrorMessage] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<{ message: string; busy: boolean } | null>(null);
  const dispose = render(
    () => (
      <>
        <Editor3D
          toolbarStart={() => (
            <>
              <RobinMascot />
              <h1>Editor</h1>
            </>
          )}
          index={() => null}
          library={() => library}
          onError={(error) => {
            errors.push(error);
            setErrorMessage(error);
          }}
          onStatus={(message, busy = false) => setStatus(message ? { message, busy } : null)}
        />
        <Show when={status()}>
          {(current) => (
            <StatusDialog
              message={current().message}
              busy={current().busy}
              onClose={() => setStatus(null)}
            />
          )}
        </Show>
        <Show when={errorMessage()}>
          {(message) => <ErrorDialog message={message()} onClose={() => setErrorMessage(null)} />}
        </Show>
      </>
    ),
    document.querySelector("#root")!,
  );
  async function open(name: string) {
    await until(() => !!document.querySelector(`[data-map="${name}"]`));
    (document.querySelector(`[data-map="${name}"]`) as HTMLButtonElement).click();
    await until(() => current() === name);
  }
  function edit() {
    const transfer = new DataTransfer();
    transfer.items.add(
      new File([JSON.stringify(blank)], "York.rhlos-map.json", { type: "application/json" }),
    );
    document
      .querySelector(".editor-canvas")!
      .dispatchEvent(new DragEvent("drop", { bubbles: true, dataTransfer: transfer }));
  }
  try {
    await until(() => !!document.querySelector('[data-map="York"]'));
    assert(!current(), "Single map unexpectedly auto-opened");
    assert(!document.querySelector(".editor-modes"), "Editor modes visible before map load");
    assert(
      ![...document.querySelectorAll("button")].some(
        (button) => button.textContent?.trim() === "View settings",
      ),
      "View settings visible before map load",
    );
    assert(!document.querySelector('select[aria-label="Map"]'), "Header map selector remains");
    assert(
      !document.querySelector(`[aria-label="Delete ${publishedMapLabel("York", false)}"]`),
      "Built-in delete button shown",
    );
    assert(!warns(), "Clean screen warns on unload");
    const headerHeight = document.querySelector("header")!.getBoundingClientRect().height;
    setStatus({ message: "Compiling map and sprite occlusion…", busy: true });
    await until(() => !!document.querySelector(".status-dialog:modal"));
    assert(
      !document.querySelector("header")!.textContent?.includes("Compiling"),
      "Progress appeared in header",
    );
    assert(
      document.querySelector("header")!.getBoundingClientRect().height === headerHeight,
      "Progress resized header",
    );
    const operationDialog = document.querySelector<HTMLDialogElement>(".status-dialog")!;
    const bounds = operationDialog.getBoundingClientRect();
    assert(
      Math.abs(bounds.left + bounds.width / 2 - document.documentElement.clientWidth / 2) < 1 &&
        Math.abs(bounds.top + bounds.height / 2 - document.documentElement.clientHeight / 2) < 1,
      "Progress modal is not centered",
    );
    const cancel = new Event("cancel", { cancelable: true });
    operationDialog.dispatchEvent(cancel);
    assert(
      cancel.defaultPrevented && operationDialog.open,
      "Busy operation dismissed without cancellation support",
    );
    setStatus({ message: "Packaging mod ZIP…", busy: true });
    await until(() => operationDialog.textContent.includes("Packaging"));
    setStatus({ message: "Exported map.zip", busy: false });
    await until(() => !!operationDialog.querySelector("button"));
    operationDialog.querySelector("button")!.click();
    await until(() => !document.querySelector(".status-dialog"));

    let release!: () => void;
    blockedRead = new Promise<void>((resolve) => {
      release = resolve;
    });
    const before = document.querySelector(".editor-body")!.getBoundingClientRect().height;
    document.querySelector<HTMLButtonElement>('[data-map="York"]')!.click();
    await until(() => !!document.querySelector(".map-load-dialog:modal"));
    assert(
      document.querySelector(".editor-body")!.getBoundingClientRect().height === before,
      "Loading resized workspace",
    );
    const modalBounds = document.querySelector(".map-load-dialog")!.getBoundingClientRect();
    assert(
      Math.abs(
        modalBounds.left + modalBounds.width / 2 - document.documentElement.clientWidth / 2,
      ) < 1,
      `Loading modal is not horizontally centered: ${modalBounds.left}, ${modalBounds.width}, viewport ${document.documentElement.clientWidth}`,
    );
    assert(
      Math.abs(
        modalBounds.top + modalBounds.height / 2 - document.documentElement.clientHeight / 2,
      ) < 1,
      `Loading modal is not vertically centered: ${modalBounds.top}, ${modalBounds.height}, viewport ${document.documentElement.clientHeight}`,
    );
    document.querySelector<HTMLButtonElement>(".map-load-dialog button")!.click();
    await until(() => !document.querySelector(".map-load-dialog"));
    release();
    blockedRead = null;
    await new Promise((resolve) => setTimeout(resolve, 80));
    assert(!current(), "Cancelled load still published");
    failRead = true;
    document.querySelector<HTMLButtonElement>('[data-map="York"]')!.click();
    await until(() => !!document.querySelector(".error-dialog:modal"));
    assert(!document.querySelector(".map-load-dialog"), "Loading dialog remained over error");
    const errorBounds = document.querySelector(".error-dialog")!.getBoundingClientRect();
    assert(
      Math.abs(
        errorBounds.left + errorBounds.width / 2 - document.documentElement.clientWidth / 2,
      ) < 1 &&
        Math.abs(
          errorBounds.top + errorBounds.height / 2 - document.documentElement.clientHeight / 2,
        ) < 1,
      "Error modal is not centered",
    );
    assert(
      document.querySelector(".editor-body")!.getBoundingClientRect().height === before,
      "Error resized workspace",
    );
    assert(
      document.querySelector(".error-dialog")!.textContent?.includes("500"),
      "Error details missing",
    );
    document.querySelector<HTMLButtonElement>(".error-dialog button")!.click();
    await until(() => !document.querySelector(".error-dialog"));
    errors.length = 0;
    failRead = false;
    assert(
      !document.querySelector("header")!.textContent?.includes("New map"),
      "New map remains in editor header",
    );
    const mascot = document.querySelector<HTMLElement>(".robin-mascot")!;
    await until(() =>
      [...mascot.querySelectorAll("img")].every(
        (image) => image.complete && image.naturalWidth > 0,
      ),
    );
    mascot.focus();
    await until(() => !!mascot.querySelector(".robin-to-dance"));
    await until(() => !!mascot.querySelector(".robin-dancing"));
    assert(
      getComputedStyle(mascot.querySelector(".robin-dancing")!).visibility === "visible",
      "Robin did not dance on focus",
    );
    mascot.blur();
    await until(() => !!mascot.querySelector(".robin-from-dance"));
    await until(() => !!mascot.querySelector(".robin-bored"));
    assert(
      getComputedStyle(mascot.querySelector(".robin-bored")!).visibility === "visible",
      "Robin did not return to bored pose",
    );
    await open("York");
    edit();
    await until(() => current() === "York (Modified)");
    assert(warns(), "Dirty map did not prevent unload");
    click("Close map");
    await new Promise((resolve) => setTimeout(resolve, 50));
    assert(current() === "York (Modified)" && confirmations === 1, "Cancel discarded dirty map");
    click("Save *");
    await until(() => !warns());
    click("Close map");
    await until(() => !current());
    await until(() => !!document.querySelector('[data-map="York (Modified)"] img'));
    click(`Rename ${publishedMapLabel("York", true)}`);
    await until(() => !!document.querySelector('[data-map="Renamed map"]'));
    await open("Renamed map");
    click("Close map");
    await until(() => !current());
    click("Delete Renamed map");
    assert(document.querySelector('[data-map="Renamed map"]'), "Cancel deleted map");
    approve = true;
    click("Delete Renamed map");
    await until(() => !document.querySelector('[data-map="Renamed map"]'));
    await open("York");
    edit();
    await until(() => warns());
    click("Close map");
    await until(() => !current());
    assert(!warns(), "Discard left unload warning active");
    assert(
      !document.querySelector('[data-map="York (Modified)"]'),
      "Discard left transient map card",
    );
    const canvas = document.createElement("canvas");
    canvas.width = 480;
    canvas.height = 300;
    canvas.getContext("2d")!.fillRect(0, 0, 480, 300);
    const thumbnail = await encodeMapThumbnail(canvas);
    const first = await library.saveMap("York", blank, thumbnail);
    const second = await library.saveMap("York", blank, thumbnail);
    assert(
      first === "York (Modified)" && second === "York (Modified 2)",
      "Built-in saves overwrite copies",
    );
    assert(
      (await library.saveMap(second, blank, thumbnail)) === second,
      "Saving copy creates another copy",
    );
    const maps = await library.handle.getDirectoryHandle("scenes");
    assert(
      JSON.parse(
        await (await (await maps.getFileHandle(second + ".rhlos-map.json")).getFile()).text(),
      ).map === library.documentMap(second),
      "Copy identity cannot reload",
    );
    let rejected = false;
    try {
      await library.renameMap(first, "York");
    } catch {
      rejected = true;
    }
    assert(rejected, "Rename overwrote built-in map");
    rejected = false;
    try {
      await library.deleteMap("York");
    } catch {
      rejected = true;
    }
    assert(rejected, "Deleted built-in map");
    assert(errors.length === 0, errors.join("; "));
    result.textContent = `PASS map selection, native ${thumbnail.type} thumbnails, dirty close/cancel/discard/unload/save, rename/delete, collision-free built-in copies`;
  } finally {
    dispose();
    window.fetch = originalFetch;
    window.confirm = originalConfirm;
    window.prompt = originalPrompt;
    await root.removeEntry("map-selection-test", { recursive: true });
  }
}
main().catch((error) => {
  result.textContent = "FAIL " + (error.stack ?? error);
});
