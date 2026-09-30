import LibraryBrowser from "./LibraryBrowser";
import { For, Show, createEffect, createSignal, onCleanup } from "solid-js";
import type { ProjectionAssetEntry } from "@rle/shared";
import AssetPreview, { AssetPreviewRenderer } from "./AssetPreview";
import { listProjectionAppearances } from "./projection-library";
import {
  ASSET_DRAG_TYPE,
  REFINED_LEVELS_FILTER,
  assetType,
  assetTags,
  filterAssets,
  isGameplayHelper,
} from "./asset-library";

export default function AssetLibrary(props: {
  root: FileSystemDirectoryHandle | null;
  renderer?: AssetPreviewRenderer;
  entries: ProjectionAssetEntry[];
  loading: boolean;
  error: string;
  canInsert: boolean;
  collapsed: boolean;
  onToggle: () => void;
  onPreload: (entry: ProjectionAssetEntry) => void;
  onDragStart: (entry: ProjectionAssetEntry) => void;
  onDragReturn: () => void;
  onAdd: (entry: ProjectionAssetEntry) => void;
  onDragEnd: () => void;
}) {
  const [search, setSearch] = createSignal("");
  const [type, setType] = createSignal("");
  const [source, setSource] = createSignal(REFINED_LEVELS_FILTER);
  const [showHelpers, setShowHelpers] = createSignal(false);
  const renderer = props.renderer ?? new AssetPreviewRenderer();
  onCleanup(() => {
    if (!props.renderer) renderer.dispose();
  });
  createEffect(
    () => props.root,
    () => {
      renderer.cache.clear();
      setType("");
      setSource(REFINED_LEVELS_FILTER);
      setSearch("");
      setShowHelpers(false);
    },
  );
  const visibleEntries = () =>
    props.entries.filter((entry) => showHelpers() || !isGameplayHelper(entry));
  const filtered = () => filterAssets(props.entries, search(), type(), source(), showHelpers());
  return (
    <aside
      class="shared-library"
      onDragEnter={props.onDragReturn}
      onDragOver={(event) => {
        if (event.dataTransfer?.types.includes(ASSET_DRAG_TYPE)) {
          event.preventDefault();
          event.dataTransfer.dropEffect = "none";
        }
      }}
      onDrop={(event) => {
        if (event.dataTransfer?.types.includes(ASSET_DRAG_TYPE)) {
          event.preventDefault();
          props.onDragEnd();
        }
      }}
    >
      <header class="library-heading">
        <button
          onClick={props.onToggle}
          aria-expanded={props.collapsed ? "false" : "true"}
          aria-controls="asset-library-content"
          aria-label={props.collapsed ? "Show asset library" : "Hide asset library"}
          title={props.collapsed ? "Show asset library" : "Hide asset library"}
        >
          {props.collapsed ? "→" : "←"}
        </button>
        <h2>Asset library</h2>
      </header>
      <div id="asset-library-content" class="library-content" hidden={props.collapsed}>
        <LibraryBrowser
          search={search()}
          onSearch={setSearch}
          searchLabel="Find assets"
          placeholder="Search assets or tags…"
          label="Asset library"
          summary={`${filtered().length} of ${props.entries.length} assets · Hover to rotate`}
          filters={
            <>
              <label>
                Asset type
                <select
                  aria-label="Asset type"
                  value={type()}
                  onChange={(event) => setType(event.currentTarget.value)}
                >
                  <option value="">All types</option>
                  <For each={[...new Set(visibleEntries().map(assetType))].sort()}>
                    {(value) => <option value={value}>{value}</option>}
                  </For>
                </select>
              </label>
              <label>
                Source level
                <select
                  aria-label="Source level"
                  value={source()}
                  onChange={(event) => setSource(event.currentTarget.value)}
                >
                  <option value={REFINED_LEVELS_FILTER}>All refined levels</option>
                  <option value="">All levels</option>
                  <For each={[...new Set(props.entries.map((entry) => entry.source_map))].sort()}>
                    {(value) => <option value={value}>{value}</option>}
                  </For>
                </select>
              </label>
              <label class="library-helper-toggle">
                <input
                  type="checkbox"
                  checked={showHelpers()}
                  onChange={(event) => {
                    setShowHelpers(event.currentTarget.checked);
                    setType("");
                  }}
                />
                Show gameplay helpers
              </label>
            </>
          }
          beforeGrid={
            <>
              <Show when={!props.root}>
                <p class="hint">Waiting for assets…</p>
              </Show>
              <Show when={props.loading}>
                <p class="hint">Loading shared library…</p>
              </Show>
              <Show when={props.error}>
                <p class="library-error" role="alert">
                  {props.error}
                </p>
              </Show>
              <Show when={!props.loading && props.root && !props.error && !filtered().length}>
                <p class="hint">
                  {props.entries.length
                    ? "No assets match these filters."
                    : "No published 3D assets in this library."}
                </p>
              </Show>
            </>
          }
        >
          <For each={filtered()}>
            {(entry) => {
              const [appearances, setAppearances] = createSignal<ProjectionAssetEntry[]>([entry]);
              const [chosen, setChosen] = createSignal(0);
              const [ready, setReady] = createSignal(false);
              const [appearanceError, setAppearanceError] = createSignal("");
              let pending: Promise<ProjectionAssetEntry[]> | undefined;
              const selected = () => appearances()[chosen()] ?? entry;
              const loadAppearances = () =>
                (pending ??= listProjectionAppearances(props.root!, entry)
                  .then((items) => {
                    setAppearances(items);
                    setReady(true);
                    return items;
                  })
                  .catch((error) => {
                    setAppearanceError(String(error));
                    throw error;
                  }));
              const preload = () => {
                void loadAppearances().then(
                  () => {
                    if (props.canInsert && entry.editor_usage !== "map-background")
                      props.onPreload(selected());
                  },
                  () => {},
                );
              };
              return (
                <article
                  class="asset-card"
                  draggable={
                    props.canInsert && entry.editor_usage !== "map-background" ? "true" : "false"
                  }
                  onPointerEnter={preload}
                  onFocus={preload}
                  onDragStart={(event) => {
                    if (!props.canInsert || entry.editor_usage === "map-background" || !ready()) {
                      event.preventDefault();
                      return;
                    }
                    const appearance = selected();
                    event.dataTransfer!.setData(ASSET_DRAG_TYPE, appearance.id);
                    event.dataTransfer!.effectAllowed = "copy";
                    const image = document.createElement("canvas");
                    image.width = image.height = 1;
                    event.dataTransfer!.setDragImage(image, 0, 0);
                    props.onDragStart(appearance);
                  }}
                  onDragEnd={props.onDragEnd}
                >
                  <AssetPreview entry={selected()} root={props.root!} renderer={renderer} />
                  <div class="asset-card-info">
                    <strong title={entry.name}>{entry.name}</strong>
                    <div class="asset-tags">
                      <For each={assetTags(entry)}>{(tag) => <span>{tag}</span>}</For>
                    </div>
                    <Show when={appearances().length > 1}>
                      <label>
                        Appearance
                        <select
                          aria-label={`${entry.name} appearance`}
                          value={chosen()}
                          onChange={(event) => {
                            setChosen(Number(event.currentTarget.value));
                            props.onPreload(selected());
                          }}
                        >
                          <For each={appearances()}>
                            {(appearance, index) => (
                              <option value={index()}>{appearance.name}</option>
                            )}
                          </For>
                        </select>
                      </label>
                    </Show>
                    <Show when={appearanceError()}>
                      <span class="library-error">{appearanceError()}</span>
                    </Show>
                    <button
                      disabled={!props.canInsert || entry.editor_usage === "map-background"}
                      onClick={() =>
                        void loadAppearances().then(
                          () => props.onAdd(selected()),
                          () => {},
                        )
                      }
                      aria-label={`Add ${entry.name}`}
                    >
                      {entry.editor_usage === "map-background" ? "Map background" : "Add to scene"}
                    </button>
                  </div>
                </article>
              );
            }}
          </For>
        </LibraryBrowser>
        <footer>
          {props.canInsert
            ? "Drag an asset into the scene to place it."
            : "Open a level to place assets."}
        </footer>
      </div>
    </aside>
  );
}
