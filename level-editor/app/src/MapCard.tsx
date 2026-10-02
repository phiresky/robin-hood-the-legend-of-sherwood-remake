import { Show, createEffect, createSignal } from "solid-js";
import type { LibraryRef } from "./Editor3D";
import { isNotFound, subdir } from "./fs";
import { thumbnailExtensions } from "./map-thumbnail";

export default function MapCard(props: {
  name: string;
  label: string;
  library: LibraryRef;
  onOpen: () => void;
  onDownload: () => void;
  disabled?: boolean;
  onDelete?: () => void;
  onRename?: () => void;
}) {
  const [preview, setPreview] = createSignal<string | null>(null);
  createEffect(
    () => ({ library: props.library, name: props.name }),
    ({ library, name }) => {
      let cancelled = false;
      let url: string | undefined;
      setPreview(null);
      void (async () => {
        const directory = await subdir(library.handle, ["scenes"]);
        if (!directory) return;
        for (const extension of thumbnailExtensions) {
          try {
            const file = await (await directory.getFileHandle(`${name}.${extension}`)).getFile();
            if (cancelled) return;
            url = URL.createObjectURL(file);
            setPreview(url);
            return;
          } catch (error) {
            if (!isNotFound(error)) throw error;
          }
        }
      })().catch((error) => console.warn(`Cannot load thumbnail for ${name}`, error));
      return () => {
        cancelled = true;
        if (url) URL.revokeObjectURL(url);
      };
    },
  );
  return (
    <article class="map-card">
      <button
        class="map-card-open"
        data-map={props.name}
        onClick={props.onOpen}
        disabled={props.disabled}
      >
        <span class="map-thumbnail">
          <Show
            when={preview()}
            fallback={
              <span class="map-thumbnail-empty">
                No preview yet<span>Save this map to create one</span>
              </span>
            }
          >
            {(url) => <img src={url()} alt="" loading="lazy" />}
          </Show>
        </span>
        <span class="map-card-name">{props.label}</span>
      </button>
      <div class="map-card-actions">
        <button
          aria-label={`Download ${props.label}`}
          title="Download latest saved map"
          disabled={props.disabled}
          onClick={props.onDownload}
        >
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="M12 3v12m-5-5 5 5 5-5M5 16v5h14v-5" />
          </svg>
        </button>
        <Show when={props.onRename}>
          <button
            aria-label={`Rename ${props.label}`}
            title="Rename map"
            disabled={props.disabled}
            onClick={props.onRename}
          >
            ✎
          </button>
        </Show>
        <Show when={props.onDelete}>
          <button
            aria-label={`Delete ${props.label}`}
            title="Delete map"
            disabled={props.disabled}
            onClick={props.onDelete}
          >
            ×
          </button>
        </Show>
      </div>
    </article>
  );
}
