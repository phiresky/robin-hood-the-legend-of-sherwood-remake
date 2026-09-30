import { Show } from "solid-js";
import { Portal, type JSX } from "@solidjs/web";

/** Keep a tool's library connected to its editor state while displaying it in the sidebar. */
export default function LibraryPortal(props: {
  mount?: HTMLElement;
  active?: boolean;
  children: JSX.Element;
}) {
  return (
    <Show when={props.mount} fallback={props.children}>
      {(mount) => (
        <Portal mount={mount()}>
          <div class="mode-library-section" hidden={props.active === false}>
            {props.children}
          </div>
        </Portal>
      )}
    </Show>
  );
}
