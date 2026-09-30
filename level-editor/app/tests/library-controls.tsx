import LibraryPortal from "../src/LibraryPortal";
import { render } from "@solidjs/web";
import { createSignal } from "solid-js";
import { terrainMaterials, type CustomTerrainMaterial } from "@rle/shared";
import MaterialPicker from "../src/MaterialPicker";
import AssetLibrary from "../src/AssetLibrary";
import MissionCharacterChoices from "../src/MissionCharacterChoices";
import "../src/styles.css";

const [defaultCategory, setDefaultCategory] = createSignal<string>();
const [value, setValue] = createSignal("grass_short");
const [custom, setCustom] = createSignal<CustomTerrainMaterial[]>([]);
const [disabled, setDisabled] = createSignal(false);
const [selectionDisabled, setSelectionDisabled] = createSignal(false);
let changes = 0;
const portalMount = document.createElement("div");
portalMount.id = "library-test-sidebar";
portalMount.style.width = "340px";
if (location.search.includes("portal")) document.body.prepend(portalMount);
const mountStart = performance.now();
const dispose = render(
  () => (
    <div style={{ display: "flex", gap: "20px", height: "650px" }}>
      <div class="editor-panel" style={{ width: "340px", overflow: "auto" }}>
        <LibraryPortal mount={location.search.includes("portal") ? portalMount : undefined}>
          <MaterialPicker
            defaultCategory={defaultCategory()}
            value={value()}
            onChange={(id) => {
              changes++;
              setValue(id);
            }}
            customMaterials={custom()}
            onCustomMaterialsChange={setCustom}
            disabled={disabled()}
            selectionDisabled={selectionDisabled()}
            label="Vertex material"
          />
        </LibraryPortal>
        <MissionCharacterChoices
          root={{} as FileSystemDirectoryHandle}
          camera={{ kind: "oblique-orthographic", elevation_deg: 35 }}
          profiles={[]}
          onDragStart={() => {}}
          onDragEnd={() => {}}
        />
      </div>
      <AssetLibrary
        root={null}
        entries={[]}
        loading={false}
        error=""
        canInsert={false}
        collapsed={false}
        onToggle={() => {}}
        onPreload={() => {}}
        onDragStart={() => {}}
        onDragReturn={() => {}}
        onAdd={() => {}}
        onDragEnd={() => {}}
      />
    </div>
  ),
  document.querySelector("#root")!,
);
const firstFrame = new Promise<number>((resolve) =>
  requestAnimationFrame(() => resolve(performance.now() - mountStart)),
);
const pause = () => new Promise((resolve) => setTimeout(resolve, 50));
function check(value: unknown, message: string) {
  if (!value) throw new Error(message);
}
function input(selector: string, value: string, event = "input") {
  const element = document.querySelector<HTMLInputElement | HTMLSelectElement>(selector);
  check(element, `Missing ${selector}`);
  element!.value = value;
  element!.dispatchEvent(new Event(event, { bubbles: true }));
}
async function run() {
  await pause();
  if (location.search.includes("portal"))
    check(
      portalMount.querySelectorAll(".material-picker").length === 1,
      "Portal did not mount exactly one material picker",
    );
  const cards = () => [
    ...document.querySelectorAll<HTMLButtonElement>(".material-picker .asset-card"),
  ];
  check(cards().length === terrainMaterials.length, "Full material catalog is missing");
  const canvas = document.querySelector<HTMLCanvasElement>(".material-preview")!;
  for (let i = 0; i < 100 && canvas.dataset.previewReady !== "true"; i++) await pause();
  check(canvas.dataset.previewReady === "true", "Visible material thumbnail never loaded");
  check(
    canvas.width === 128 && canvas.height === 128,
    "Library previews must not allocate full terrain textures",
  );
  const ready = document.querySelectorAll('.material-preview[data-preview-ready="true"]').length;
  check(ready < terrainMaterials.length, "Offscreen material previews should stay lazy");
  const pixels = canvas.getContext("2d")!.getImageData(0, 0, canvas.width, canvas.height).data;
  check(
    pixels.some((value, i) => i % 4 === 3 && value > 0),
    "Material thumbnail is blank",
  );
  const searches = [
    ...document.querySelectorAll<HTMLInputElement>(".library-browser input[type=search]"),
  ];
  check(
    searches.length === 3,
    "Materials, characters and assets must share library search controls",
  );
  const styles = searches.map((search) => {
    const s = getComputedStyle(search);
    return [s.padding, s.borderRadius, s.backgroundColor, s.fontSize].join("|");
  });
  check(
    styles.every((style) => style === styles[0]),
    "Library searches have inconsistent styling",
  );
  setDefaultCategory("path");
  await pause();
  check(
    cards().length === terrainMaterials.filter((m) => m.category === "path").length,
    "Road default must show path materials",
  );
  input('[aria-label="Material category"]', "", "change");
  await pause();
  check(
    cards().length === terrainMaterials.length,
    "Default category must allow browsing other materials",
  );
  setDefaultCategory("river");
  await pause();
  check(value() === "grass_short", "Changing default category must not edit the selected material");
  input('[aria-label="Material category"]', "river", "change");
  await pause();
  check(
    cards().length === terrainMaterials.filter((m) => m.category === "river").length,
    "Category filter failed",
  );
  input('[aria-label="Search materials"]', "water_white_stone");
  await pause();
  check(cards().length === 1, "Material ID search failed");
  cards()[0]!.click();
  await pause();
  check(
    value() === "water_white_stone" && changes === 1,
    "Card did not apply material exactly once",
  );
  check(cards()[0]!.getAttribute("aria-pressed") === "true", "Selected card is not marked");
  input('[aria-label="Search materials"]', "no-matching-material");
  await pause();
  check(
    cards().length === 0 && value() === "water_white_stone",
    "Search changed the current material",
  );
  document.querySelector<HTMLDetailsElement>(".material-picker details")!.open = true;
  input('.material-picker input[type="text"]', "Custom copper");
  input('.material-picker input[type="color"]', "#b36b37");
  await pause();
  [...document.querySelectorAll<HTMLButtonElement>(".material-picker button")]
    .find((b) => b.textContent === "Add to map materials")!
    .click();
  await pause();
  check(
    custom().length === 1 && cards().length === 1,
    `New custom material is missing from library: ${JSON.stringify(custom())}, ${cards().length} cards, ${document.querySelector(".material-picker")?.textContent}`,
  );
  check(value() === "water_white_stone", "Adding a custom material unexpectedly applied it");
  cards()[0]!.click();
  await pause();
  check(value() === "custom_custom_copper" && changes === 2, "Custom material card did not apply");
  setSelectionDisabled(true);
  await pause();
  cards()[0]!.click();
  check(changes === 2, "Selection-disabled picker applied material");
  check(
    !document
      .querySelector<HTMLInputElement>('[aria-label="Search materials"]')!
      .matches(":disabled"),
    "Selection-disabled picker disabled search",
  );
  setSelectionDisabled(false);
  setDisabled(true);
  await pause();
  cards()[0]!.click();
  check(changes === 2, "Disabled picker applied material");
  setDisabled(false);
  input('[aria-label="Search materials"]', "");
  input('[aria-label="Material category"]', "", "change");
  await pause();
  check(cards().length === terrainMaterials.length + 1, "Clearing filters did not restore catalog");
  if (!location.search.includes("keep")) dispose();
  document.querySelector("#result")!.textContent =
    `PASS shared libraries, lazy 128px previews, category defaults, filtering and application; first frame ${Math.round(await firstFrame)}ms; initially rendered ${ready}/${terrainMaterials.length} thumbnails`;
}
run().catch((error) => {
  document.querySelector("#result")!.textContent = "FAIL " + (error.stack ?? error);
});
