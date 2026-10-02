import { For, Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";
import {
  connectInteriors,
  interiorEndpointId,
  placedInteriorOptions,
  type GameplayAssetDescriptor,
  type Level3D,
} from "@rle/shared";
import { readPinnedAssetDescriptors } from "./projection-library";

export default function InteriorConnectionsPanel(props: {
  document(): Level3D | null;
  library(): FileSystemDirectoryHandle | null;
  commit(document: Level3D): void;
  onError(message: string): void;
}) {
  const [assets, setAssets] = createSignal(new Map<string, GameplayAssetDescriptor>());
  const [status, setStatus] = createSignal("");
  const [from, setFrom] = createSignal("");
  const [to, setTo] = createSignal("");
  // Pin changes reload descriptors; dragging parts and connecting rooms do not.
  const pins = createMemo(() => JSON.stringify(props.document()?.assetSources ?? []));
  let generation = 0;
  createEffect(
    () => ({ root: props.library(), pins: pins() }),
    ({ root, pins }) => {
      const current = ++generation;
      setAssets(new Map());
      setStatus(root ? "Loading room definitions…" : "Open the asset library to connect rooms.");
      if (root)
        void readPinnedAssetDescriptors(root, JSON.parse(pins))
          .then((descriptors) => {
            if (current !== generation) return;
            setAssets(descriptors);
            setStatus("");
          })
          .catch((error) => {
            if (current === generation) setStatus(String(error));
          });
    },
  );
  onCleanup(() => {
    generation++;
  });
  const rooms = createMemo(() => {
    const document = props.document();
    return document ? placedInteriorOptions(document, assets()) : [];
  });
  const source = createMemo(() =>
    rooms().find((room) => interiorEndpointId(room.endpoint) === from()),
  );
  const targets = createMemo(() =>
    rooms().filter((room) => {
      const first = source()?.endpoint;
      return (
        first &&
        (room.endpoint.placement !== first.placement || room.endpoint.asset !== first.asset)
      );
    }),
  );
  const target = createMemo(() =>
    targets().find((room) => interiorEndpointId(room.endpoint) === to()),
  );
  const label = (id: string) =>
    rooms().find((room) => interiorEndpointId(room.endpoint) === id)?.label ?? id;
  return (
    <section aria-label="Interior connections">
      <h3>Interior connections</h3>
      <p>
        Doors in the same asset room connect automatically. Link rooms below to connect separate
        assets.
      </p>
      <Show when={status()}>
        <p role="status">{status()}</p>
      </Show>
      <Show when={!status()}>
        <Show when={rooms().length > 0} fallback={<p>No placed assets have interior rooms.</p>}>
          <label>
            From room
            <select
              value={source() ? from() : ""}
              onChange={(event) => setFrom(event.currentTarget.value)}
            >
              <option value="">Choose a room…</option>
              <For each={rooms()}>
                {(room) => <option value={interiorEndpointId(room.endpoint)}>{room.label}</option>}
              </For>
            </select>
          </label>
          <label>
            To room
            <select
              value={target() ? to() : ""}
              onChange={(event) => setTo(event.currentTarget.value)}
            >
              <option value="">Choose another asset’s room…</option>
              <For each={targets()}>
                {(room) => <option value={interiorEndpointId(room.endpoint)}>{room.label}</option>}
              </For>
            </select>
          </label>
          <button
            disabled={!source() || !target()}
            onClick={() => {
              const document = props.document(),
                first = source(),
                second = target();
              if (!document || !first || !second) return;
              try {
                props.commit(connectInteriors(document, first.endpoint, second.endpoint));
              } catch (error) {
                props.onError(String(error));
              }
            }}
          >
            Connect rooms
          </button>
        </Show>
      </Show>
      <For each={props.document()?.interiorConnections ?? []}>
        {(connection) => (
          <div>
            <span>
              {label(interiorEndpointId(connection.from))} ↔{" "}
              {label(interiorEndpointId(connection.to))}
            </span>
            <button
              aria-label={`Remove connection ${connection.id}`}
              onClick={() => {
                const document = props.document();
                if (document)
                  props.commit({
                    ...document,
                    interiorConnections: document.interiorConnections?.filter(
                      (link) => link.id !== connection.id,
                    ),
                  });
              }}
            >
              Remove
            </button>
          </div>
        )}
      </For>
    </section>
  );
}
