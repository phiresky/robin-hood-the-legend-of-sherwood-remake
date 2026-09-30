import LibraryPortal from "./LibraryPortal";
import { For, Show, createEffect, createSignal, onCleanup, untrack } from "solid-js";
import { parseLevel3D, type Level3D, type Vec3 } from "@rle/shared";
import type { EditorViewport } from "./editor-viewport.ts";
import ScrubNumber from "./ScrubNumber";
import MissionCharacterChoices from "./MissionCharacterChoices";
import { followMissionTerrain } from "./terrain-follow.ts";
import {
  DEFAULT_CHARACTER_DIRECTION,
  loadMissionCharacterCatalog,
  type MissionCharacterProfile,
} from "./mission-character-catalog.ts";

export default function MissionPanel(props: {
  document(): Level3D | null;
  commit(document: Level3D): void;
  onError(message: string): void;
  active: boolean;
  libraryMount?: HTMLElement;
  viewport: EditorViewport;
  library?(): FileSystemDirectoryHandle | null;
}) {
  const [catalog, setCatalog] = createSignal<{
    root: FileSystemDirectoryHandle;
    profiles: MissionCharacterProfile[];
  } | null>(null);
  const [catalogStatus, setCatalogStatus] = createSignal("");
  const [spriteStatus, setSpriteStatus] = createSignal("");
  let catalogGeneration = 0;
  createEffect(
    () => ({ library: props.library?.() ?? null }),
    ({ library }) => {
      const generation = ++catalogGeneration;
      setCatalog(null);
      props.viewport.setMissionSpriteLibrary(null, [], () => {});
      setCatalogStatus(
        library
          ? "Loading library characters…"
          : "Open the asset library to choose character sprites.",
      );
      if (library)
        void loadMissionCharacterCatalog(library)
          .then((catalog) => {
            if (generation !== catalogGeneration) return;
            setCatalog(catalog);
            setCatalogStatus("");
            props.viewport.setMissionSpriteLibrary(
              catalog.root,
              catalog.profiles,
              (loading, warnings) => {
                if (generation === catalogGeneration)
                  setSpriteStatus(
                    loading ? "Loading placed character sprites…" : warnings.join("\n"),
                  );
              },
            );
          })
          .catch((error) => {
            if (generation === catalogGeneration) setCatalogStatus(String(error));
          });
    },
  );
  onCleanup(() => {
    catalogGeneration++;
    props.viewport.setMissionSpriteLibrary(null, [], () => {});
  });
  const [selected, setSelected] = createSignal("");
  const [visible, setVisible] = createSignal(true);
  createEffect(visible, (value) => props.viewport.setMissionVisible(value));
  const [category, setCategory] = createSignal<"pc" | "npc">("pc");
  const entries = () => [
    ...(props.document()?.mission?.spawnPoints ?? []).map((entry) => ({
      ...entry,
      kind: "pc" as const,
    })),
    ...(props.document()?.mission?.soldiers ?? []).map((entry) => ({
      ...entry,
      kind: "npc" as const,
    })),
  ];
  const current = () => entries().find((entry) => entry.id === selected());
  function chooseCharacter(profile: MissionCharacterProfile) {
    const value = mission();
    const entry = current();
    if (!entry) return;
    const previous = catalog()?.profiles.find(
      (p) => p.kind === entry.kind && p.profile === entry.profile,
    );
    const name =
      entry.name === previous?.name ||
      entry.name === "Soldier" ||
      entry.name === "PC spawn" ||
      (entry.kind === "pc" &&
        entry.profile === undefined &&
        /^Campaign spawn(?: \d+)?$/.test(entry.name))
        ? profile.name
        : entry.name;
    if (profile.kind === "pc" && typeof profile.profile === "number") {
      const id = profile.profile;
      publish({
        ...value,
        spawnPoints: value.spawnPoints.map((spawn) =>
          spawn.id === selected() ? { ...spawn, profile: id, name } : spawn,
        ),
      });
    } else if (profile.kind === "npc" && typeof profile.profile === "string") {
      const id = profile.profile;
      publish({
        ...value,
        soldiers: value.soldiers.map((soldier) =>
          soldier.id === selected() ? { ...soldier, profile: id, name } : soldier,
        ),
      });
    }
  }
  const mission = () =>
    props.document()?.mission ?? { version: 1 as const, spawnPoints: [], soldiers: [] };
  function publish(next: NonNullable<Level3D["mission"]>) {
    const document = props.document();
    if (!document) return;
    try {
      const updated = { ...document, mission: next };
      parseLevel3D(updated);
      props.commit(updated);
    } catch (error) {
      props.onError(String(error));
    }
  }
  function cancelPreview() {
    const document = props.document();
    if (document) props.viewport.syncViews(document);
  }
  function preview(patch: { position?: Vec3; direction?: number }) {
    const document = props.document();
    if (!document) return;
    const entry = current();
    if (patch.position && entry)
      patch = {
        ...patch,
        position: followMissionTerrain(document, entry.position, patch.position),
      };
    const value = mission();
    props.viewport.syncViews({
      ...document,
      mission: {
        ...value,
        spawnPoints: value.spawnPoints.map((entry) =>
          entry.id === selected() ? { ...entry, ...patch } : entry,
        ),
        soldiers: value.soldiers.map((entry) =>
          entry.id === selected() ? { ...entry, ...patch } : entry,
        ),
      },
    });
  }
  function change(patch: { name?: string; position?: Vec3; direction?: number }) {
    const document = props.document();
    const entry = current();
    if (patch.position && entry && document)
      patch = {
        ...patch,
        position: followMissionTerrain(document, entry.position, patch.position),
      };
    const value = mission();
    publish({
      ...value,
      spawnPoints: value.spawnPoints.map((entry) =>
        entry.id === selected() ? { ...entry, ...patch } : entry,
      ),
      soldiers: value.soldiers.map((entry) =>
        entry.id === selected() ? { ...entry, ...patch } : entry,
      ),
    });
  }
  function addCharacter(key: string, position: Vec3, id: string, preview = false) {
    const profile = catalog()?.profiles.find(
      (profile) => `${profile.kind}:${profile.profile}` === key,
    );
    if (!profile) return;
    const base = {
      id,
      name: profile.name,
      position,
      direction: DEFAULT_CHARACTER_DIRECTION,
    };
    const value = mission();
    const update = (next: NonNullable<Level3D["mission"]>) => {
      const document = props.document();
      if (preview && document) props.viewport.syncViews({ ...document, mission: next });
      else publish(next);
    };
    if (profile.kind === "pc" && typeof profile.profile === "number")
      update({
        ...value,
        spawnPoints: [...value.spawnPoints, { ...base, profile: profile.profile }],
      });
    else if (profile.kind === "npc" && typeof profile.profile === "string")
      update({
        ...value,
        soldiers: [...value.soldiers, { ...base, profile: profile.profile, allegiance: 1 }],
      });
    if (!preview) setSelected(id);
  }
  createEffect(
    () => ({
      active: props.active && visible(),
      document: props.document(),
      selected: selected(),
    }),
    ({ active, document, selected }) => {
      untrack(() =>
        props.viewport.setMissionEdit(
          active && document
            ? {
                selected,
                select: setSelected,
                preview: (position: Vec3) => preview({ position }),
                move: (position: Vec3) => change({ position }),
                cancel: cancelPreview,
                add: addCharacter,
                previewAdd: (key: string, position: Vec3, id: string) =>
                  addCharacter(key, position, id, true),
              }
            : null,
        ),
      );
    },
  );
  onCleanup(() => props.viewport.setMissionEdit(null));
  function remove() {
    const value = mission();
    publish({
      ...value,
      spawnPoints: value.spawnPoints.filter((entry) => entry.id !== selected()),
      soldiers: value.soldiers.filter((entry) => entry.id !== selected()),
    });
    setSelected("");
  }
  return (
    <section class="view-settings mission-settings">
      <h2>Mission</h2>
      <label>
        <input
          type="checkbox"
          checked={visible()}
          onChange={(event) => setVisible(event.currentTarget.checked)}
        />
        Show characters
      </label>
      <p class="hint">
        Drag a character onto the map to add it. Click a placed character or its list entry to
        select it, then drag it to move it. Blue outlines are PCs; red outlines are NPCs.
      </p>
      <Show when={mission().importedFrom}>
        <p class="hint">
          Imported from {mission().importedFrom}. Campaign spawn slots use blue outlines until
          assigned a character.
        </p>
        <Show when={mission().importWarnings?.length}>
          <details>
            <summary>Mission import limitations ({mission().importWarnings?.length})</summary>
            <ul>
              <For each={mission().importWarnings}>{(warning) => <li>{warning}</li>}</For>
            </ul>
          </details>
        </Show>
      </Show>
      <LibraryPortal mount={props.libraryMount} active={props.active}>
        <Show when={catalogStatus()}>
          <p role="status">{catalogStatus()}</p>
        </Show>
        <fieldset disabled={!props.document() || !visible()}>
          <label>
            Character category
            <select
              aria-label="Character category"
              value={category()}
              onChange={(event) => setCategory(event.currentTarget.value === "npc" ? "npc" : "pc")}
            >
              <option value="pc">PCs</option>
              <option value="npc">NPCs</option>
            </select>
          </label>
          <Show when={!!catalog() && !!props.document()}>
            <MissionCharacterChoices
              root={catalog()!.root}
              camera={props.document()!.camera}
              profiles={catalog()!.profiles.filter((profile) => profile.kind === category())}
              onDragStart={(key) => props.viewport.startMissionPaletteDrag(key)}
              onDragEnd={() => props.viewport.endMissionPaletteDrag()}
            />
          </Show>
          <Show when={spriteStatus()}>
            <p role="status">{spriteStatus()}</p>
          </Show>
        </fieldset>
      </LibraryPortal>
      <fieldset disabled={!props.document() || !visible()}>
        <section class="object-list mission-element-list">
          <h3>Mission elements ({entries().length})</h3>
          <ul aria-label="Mission elements">
            <For each={entries()}>
              {(entry) => (
                <li
                  class={entry.id === selected() ? "selected" : ""}
                  data-mission-element={entry.id}
                >
                  <button
                    type="button"
                    aria-pressed={entry.id === selected() ? "true" : "false"}
                    onClick={() => setSelected(entry.id)}
                  >
                    <span class="kind">{entry.kind === "pc" ? "PC" : "NPC"}</span>
                    {entry.name}
                  </button>
                </li>
              )}
            </For>
          </ul>
          <Show when={!entries().length}>
            <p class="hint">No mission elements yet.</p>
          </Show>
        </section>
        <Show when={current()}>
          {(entry) => (
            <>
              <label>
                Name
                <input
                  value={entry().name}
                  onChange={(event) => change({ name: event.currentTarget.value })}
                />
              </label>
              <label>
                Character
                <select
                  value={entry().profile === undefined ? "" : String(entry().profile)}
                  onChange={(event) => {
                    if (entry().kind === "pc" && event.currentTarget.value === "") {
                      const next = mission();
                      publish({
                        ...next,
                        spawnPoints: next.spawnPoints.map((spawn) => {
                          if (spawn.id !== selected()) return spawn;
                          const { profile: _profile, ...generic } = spawn;
                          const previous = catalog()?.profiles.find(
                            (profile) => profile.kind === "pc" && profile.profile === spawn.profile,
                          );
                          return {
                            ...generic,
                            name: spawn.name === previous?.name ? "Campaign spawn" : spawn.name,
                          };
                        }),
                      });
                      return;
                    }
                    const profile = catalog()?.profiles.find(
                      (profile) =>
                        profile.kind === entry().kind &&
                        String(profile.profile) === event.currentTarget.value,
                    );
                    if (profile) chooseCharacter(profile);
                  }}
                >
                  <Show when={entry().kind === "pc"}>
                    <option value="">Campaign character</option>
                  </Show>
                  <For
                    each={
                      catalog()?.profiles.filter((profile) => profile.kind === entry().kind) ?? []
                    }
                  >
                    {(profile) => <option value={String(profile.profile)}>{profile.name}</option>}
                  </For>
                </select>
              </label>
              <Show when={entry().kind === "npc"}>
                <label>
                  Allegiance
                  <select
                    value={
                      mission().soldiers.find((soldier) => soldier.id === selected())?.allegiance ??
                      1
                    }
                    onChange={(event) => {
                      const value = Number(event.currentTarget.value);
                      const next = mission();
                      publish({
                        ...next,
                        soldiers: next.soldiers.map((soldier) =>
                          soldier.id === selected() ? { ...soldier, allegiance: value } : soldier,
                        ),
                      });
                    }}
                  >
                    <option value={0}>Royalists</option>
                    <option value={1}>Lacklandists</option>
                    <For
                      each={[...new Set(mission().soldiers.map((soldier) => soldier.allegiance))]
                        .filter((allegiance) => allegiance > 1)
                        .sort((a, b) => a - b)}
                    >
                      {(allegiance) => <option value={allegiance}>Faction {allegiance}</option>}
                    </For>
                  </select>
                </label>
              </Show>
              <For each={["X", "Y", "Height"]}>
                {(label, index) => (
                  <ScrubNumber
                    label={label}
                    value={entry().position[index()]!}
                    step={1}
                    onPreview={(value) => {
                      const position: Vec3 = [...entry().position];
                      position[index()] = value;
                      preview({ position });
                    }}
                    onCancel={cancelPreview}
                    onCommit={(value) => {
                      cancelPreview();
                      const position: Vec3 = [...entry().position];
                      position[index()] = value;
                      change({ position });
                    }}
                  />
                )}
              </For>
              <ScrubNumber
                label="Direction (0–15)"
                value={entry().direction}
                step={1}
                min={0}
                max={15}
                onPreview={(value) => preview({ direction: Math.round(value) })}
                onCancel={cancelPreview}
                onCommit={(value) => {
                  cancelPreview();
                  change({ direction: Math.round(value) });
                }}
              />
              <div class="actions">
                <button onClick={remove}>Delete placement</button>
              </div>
            </>
          )}
        </Show>
      </fieldset>
    </section>
  );
}
