import LibraryBrowser from "./LibraryBrowser";
import { For, Show, createSignal, onCleanup } from "solid-js";
import type { ProjectionAssetEntry } from "@rle/shared";
import AssetPreview, { AssetPreviewRenderer } from "./AssetPreview";

/** The same preview cards for wall strips and matching corner models. */
export default function AssetPickerDialog(props: {
  title: string;
  renderer?: AssetPreviewRenderer;
  root: FileSystemDirectoryHandle;
  entries: ProjectionAssetEntry[];
  selected?: string;
  emptyLabel?: string;
  onSelect(id: string): void;
  onClose(): void;
}) {
  const renderer = props.renderer ?? new AssetPreviewRenderer();
  onCleanup(() => {
    if (!props.renderer) renderer.dispose();
  });
  const [query, setQuery] = createSignal("");
  const [map, setMap] = createSignal("");
  const entries = () =>
    props.entries.filter(
      (entry) =>
        (!map() || entry.source_map === map()) &&
        `${entry.name} ${entry.source_map}`.toLowerCase().includes(query().toLowerCase()),
    );
  return (
    <dialog
      class="asset-picker-dialog"
      aria-label={props.title}
      ref={(dialog) =>
        queueMicrotask(() => {
          if (dialog.isConnected) dialog.showModal();
        })
      }
      onCancel={(event) => {
        event.preventDefault();
        props.onClose();
      }}
    >
      <div class="asset-picker-heading">
        <h2>{props.title}</h2>
        <button aria-label="Close asset picker" onClick={() => props.onClose()}>
          ×
        </button>
      </div>
      <LibraryBrowser
        search={query()}
        onSearch={setQuery}
        searchLabel="Search assets"
        label={props.title}
        summary={`${entries().length} of ${props.entries.length} assets`}
        empty={!entries().length}
        emptyMessage="No matching assets."
        filters={
          <label>
            Source map
            <select
              aria-label="Source map"
              value={map()}
              onChange={(event) => setMap(event.currentTarget.value)}
            >
              <option value="">All maps</option>
              <For each={[...new Set(props.entries.map((entry) => entry.source_map))].sort()}>
                {(name) => <option value={name}>{name}</option>}
              </For>
            </select>
          </label>
        }
        beforeGrid={
          <Show when={props.emptyLabel}>
            <button class="asset-picker-none" onClick={() => props.onSelect("")}>
              {props.emptyLabel}
            </button>
          </Show>
        }
      >
        <For each={entries()}>
          {(entry) => (
            <button
              class="asset-card"
              aria-pressed={props.selected === entry.id ? "true" : "false"}
              onClick={() => props.onSelect(entry.id)}
            >
              <AssetPreview entry={entry} root={props.root} renderer={renderer} />
              <span class="asset-card-info">
                <strong>{entry.name}</strong>
                <small>{entry.source_map}</small>
              </span>
            </button>
          )}
        </For>
      </LibraryBrowser>
    </dialog>
  );
}
