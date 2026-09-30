import { Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import "./library-browser.css";

/** Shared search, filters and preview-card layout for editor libraries. */
export default function LibraryBrowser(props: {
  search: string;
  onSearch(value: string): void;
  searchLabel: string;
  placeholder?: string;
  filters?: JSX.Element;
  summary?: JSX.Element;
  beforeGrid?: JSX.Element;
  children: JSX.Element;
  label: string;
  empty?: boolean;
  emptyMessage?: string;
  maxHeight?: string;
}) {
  return (
    <div class="library-browser">
      <div class="library-browser-filters">
        <input
          class="search"
          type="search"
          aria-label={props.searchLabel}
          placeholder={props.placeholder ?? props.searchLabel + "…"}
          value={props.search}
          onInput={(event) => props.onSearch(event.currentTarget.value)}
        />
        {props.filters}
      </div>
      <Show when={props.summary}>
        <p class="library-browser-summary" aria-live="polite">
          {props.summary}
        </p>
      </Show>
      {props.beforeGrid}
      <div
        class="asset-grid"
        role="group"
        aria-label={props.label}
        style={{ "max-height": props.maxHeight }}
      >
        {props.children}
      </div>
      <Show when={props.empty}>
        <p class="hint" role="status">
          {props.emptyMessage ?? "No matches. Try another search or filter."}
        </p>
      </Show>
    </div>
  );
}
