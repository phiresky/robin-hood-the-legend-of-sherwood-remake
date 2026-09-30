import LibraryBrowser from "./LibraryBrowser";
import { For, Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";
import type { MapCamera } from "@rle/shared";
import { MissionEntities } from "./mission.ts";
import {
  DEFAULT_CHARACTER_DIRECTION,
  CHARACTER_DRAG_TYPE,
  type MissionCharacterProfile,
} from "./mission-character-catalog.ts";

function CharacterThumbnail(props: {
  root: FileSystemDirectoryHandle;
  camera: MapCamera;
  profile: MissionCharacterProfile;
  onReady(ready: boolean): void;
}) {
  const [url, setUrl] = createSignal("");
  const [error, setError] = createSignal("");
  let element!: HTMLDivElement;
  let generation = 0;
  let objectUrl = "";
  const camera = createMemo(() => props.camera, {
    equals: (previous, next) => previous.elevation_deg === next.elevation_deg,
  });
  createEffect(
    () => ({ root: props.root, camera: camera(), profile: props.profile }),
    ({ root, camera, profile }) => {
      const token = ++generation;
      setUrl("");
      setError("");
      props.onReady(false);
      let started = false;
      const observer = new IntersectionObserver(
        (entries) => {
          if (started || !entries.some((entry) => entry.isIntersecting)) return;
          started = true;
          observer.disconnect();
          void (async () => {
            let sprite: MissionEntities | undefined;
            try {
              sprite = await MissionEntities.loadCharacter(
                root,
                profile,
                camera,
                () => token === generation,
                [DEFAULT_CHARACTER_DIRECTION],
              );
              if (token !== generation) return;
              const blob = await sprite.thumbnail(DEFAULT_CHARACTER_DIRECTION);
              if (token !== generation) return;
              if (objectUrl) URL.revokeObjectURL(objectUrl);
              objectUrl = URL.createObjectURL(blob);
              setUrl(objectUrl);
              props.onReady(true);
            } catch (error) {
              if (token === generation) setError(String(error));
            } finally {
              sprite?.dispose();
            }
          })();
        },
        { rootMargin: "80px" },
      );
      observer.observe(element);
      onCleanup(() => {
        generation++;
        observer.disconnect();
        if (objectUrl) URL.revokeObjectURL(objectUrl);
        objectUrl = "";
      });
    },
  );
  return (
    <div
      ref={(node) => {
        element = node;
      }}
      class="mission-character-thumb"
      style={{ height: "96px", display: "grid", "place-items": "center" }}
    >
      <Show
        when={url()}
        fallback={<span title={error()}>{error() ? "Sprite unavailable" : "Loading…"}</span>}
      >
        <img
          src={url()}
          alt={props.profile.name}
          style={{ "max-height": "96px", "max-width": "100%", "image-rendering": "pixelated" }}
        />
      </Show>
    </div>
  );
}

function CharacterChoice(props: {
  root: FileSystemDirectoryHandle;
  camera: MapCamera;
  profile: MissionCharacterProfile;
  onDragStart(key: string): void;
  onDragEnd(): void;
}) {
  const [ready, setReady] = createSignal(false);
  return (
    <article
      class="asset-card"
      draggable={ready() ? "true" : "false"}
      aria-disabled={ready() ? "false" : "true"}
      data-character-profile={props.profile.profile}
      onDragStart={(event) => {
        if (!ready() || !event.dataTransfer) {
          event.preventDefault();
          return;
        }
        event.dataTransfer.setData(
          CHARACTER_DRAG_TYPE,
          `${props.profile.kind}:${props.profile.profile}`,
        );
        event.dataTransfer.effectAllowed = "copy";
        const image = document.createElement("canvas");
        image.width = image.height = 1;
        event.dataTransfer.setDragImage(image, 0, 0);
        props.onDragStart(`${props.profile.kind}:${props.profile.profile}`);
      }}
      onDragEnd={() => props.onDragEnd()}
    >
      <CharacterThumbnail
        root={props.root}
        camera={props.camera}
        profile={props.profile}
        onReady={setReady}
      />
      <div class="asset-card-info">
        <strong>{props.profile.name}</strong>
        <small>{props.profile.filename}</small>
      </div>
    </article>
  );
}

export default function MissionCharacterChoices(props: {
  root: FileSystemDirectoryHandle;
  camera: MapCamera;
  profiles: MissionCharacterProfile[];
  onDragStart(key: string): void;
  onDragEnd(): void;
}) {
  const [search, setSearch] = createSignal("");
  const filtered = () =>
    props.profiles.filter((profile) =>
      `${profile.name} ${profile.filename}`.toLowerCase().includes(search().toLowerCase()),
    );
  return (
    <div class="mission-character-choices">
      <LibraryBrowser
        search={search()}
        onSearch={setSearch}
        searchLabel="Find character"
        placeholder="Search characters or filenames…"
        label="Character profiles"
        maxHeight="360px"
        summary={`${filtered().length} of ${props.profiles.length} characters`}
        empty={!filtered().length}
        emptyMessage="No matching characters."
      >
        <For each={filtered()}>
          {(profile) => (
            <CharacterChoice
              root={props.root}
              camera={props.camera}
              profile={profile}
              onDragStart={(key) => props.onDragStart(key)}
              onDragEnd={() => props.onDragEnd()}
            />
          )}
        </For>
      </LibraryBrowser>
    </div>
  );
}
