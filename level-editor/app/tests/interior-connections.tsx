import { render } from "@solidjs/web";
import { createSignal } from "solid-js";
import InteriorConnectionsPanel from "../src/InteriorConnectionsPanel";
import { connectedInteriorCompilerFixture } from "../../shared/test-fixtures/asset-gameplay";

export async function checkInteriorConnections() {
  const fixture = connectedInteriorCompilerFixture();
  fixture.document.interiorConnections = [];
  const root = await navigator.storage.getDirectory();
  const library = await root.getDirectoryHandle("interior-connection-test", { create: true });
  const catalog = await library.getDirectoryHandle("3d-assets", { create: true });
  const file = await catalog.getFileHandle("index.json", { create: true });
  const writer = await file.createWritable();
  const entries = fixture.document.assetSources!.map((source) => {
    const asset = fixture.assets.get(source.id)!;
    const descriptor = `${source.id}.json`;
    source.descriptor = `3d-assets/${descriptor}`;
    return {
      id: asset.id,
      name: asset.name,
      source_map: asset.source_map,
      descriptor,
      model: asset.model,
      descriptor_sha256: source.descriptor_sha256,
      editor: asset,
    };
  });
  await writer.write(JSON.stringify({ version: 1, assets: entries }));
  await writer.close();
  const host = document.createElement("div");
  document.body.append(host);
  const [map, setMap] = createSignal(fixture.document);
  let error = "";
  const dispose = render(
    () => (
      <InteriorConnectionsPanel
        document={map}
        library={() => library}
        commit={setMap}
        onError={(message) => {
          error = message;
        }}
      />
    ),
    host,
  );
  const assert = (condition: unknown, message: string) => {
    if (!condition) throw new Error(message);
  };
  const tick = () => new Promise((resolve) => setTimeout(resolve, 20));
  try {
    for (let n = 0; n < 100 && host.querySelectorAll("select option").length < 4; n++) await tick();
    const selects = host.querySelectorAll("select");
    assert(selects.length === 2, `Room selectors missing: ${host.textContent}`);
    assert(
      selects[0]!.options.length === 3,
      "Each local room must have one choice, not one per door",
    );
    selects[0]!.value = "hut-a/hut/room";
    selects[0]!.dispatchEvent(new Event("change", { bubbles: true }));
    await tick();
    selects[1]!.value = "annex/annex/room";
    selects[1]!.dispatchEvent(new Event("change", { bubbles: true }));
    await tick();
    host.querySelector("button")!.click();
    await tick();
    assert(!error && map().interiorConnections?.length === 1, `Connect failed: ${error}`);
    const linked = map();
    setMap({
      ...linked,
      groups: linked.groups.map((group) => ({
        ...group,
        transform: { ...group.transform, dx: group.transform.dx + 10 },
      })),
    });
    await tick();
    assert(!host.textContent?.includes("Loading"), "Moving assets reloaded room definitions");
    host
      .querySelector<HTMLButtonElement>('button[aria-label="Remove connection interior-link-1"]')!
      .click();
    await tick();
    assert(map().interiorConnections?.length === 0, "Remove did not update the document");
    setMap(linked);
    await tick();
    assert(
      host.querySelector('button[aria-label="Remove connection interior-link-1"]'),
      "Restoring a history revision did not restore the connection",
    );
  } finally {
    dispose();
    host.remove();
    await root.removeEntry("interior-connection-test", { recursive: true });
  }
}
