import { For, Show, createEffect, createMemo, createSignal, createUniqueId } from "solid-js";
import {
  terrainMaterials,
  validateCustomTerrainMaterials,
  type CustomTerrainMaterial,
} from "@rle/shared";

import LibraryBrowser from "./LibraryBrowser";
import { tintMaterialPreview } from "./material-preview";
import previewAtlasUrl from "./terrain-textures/material-previews.png";
import previewAtlas from "./terrain-textures/material-previews.json";

export interface MaterialPickerProps {
  value: string;
  onChange(id: string): void;
  customMaterials: readonly CustomTerrainMaterial[];
  onCustomMaterialsChange(materials: CustomTerrainMaterial[]): void;
  label?: string;
  disabled?: boolean;
  selectionDisabled?: boolean;
  defaultCategory?: string;
}

/** Shared catalog chooser. Creating a custom material adds it to the map catalog. */
export default function MaterialPicker(props: MaterialPickerProps) {
  const id = createUniqueId();
  const [search, setSearch] = createSignal("");
  const [category, setCategory] = createSignal(props.defaultCategory ?? "");
  createEffect(
    () => props.defaultCategory,
    (value) => {
      setCategory(value ?? "");
    },
  );
  const [name, setName] = createSignal("");
  const [color, setColor] = createSignal("#8c7853");
  const [feedback, setFeedback] = createSignal("");
  const [error, setError] = createSignal("");
  const materials = createMemo(() => [...terrainMaterials, ...props.customMaterials]);
  const matches = createMemo(() => {
    const query = search().trim().toLocaleLowerCase();
    return materials().filter(
      (material) =>
        (!category() || ("category" in material ? material.category : "custom") === category()) &&
        `${material.name} ${material.id} ${"category" in material ? material.category : "custom"}`
          .toLocaleLowerCase()
          .includes(query),
    );
  });
  const selected = createMemo(() => materials().find((material) => material.id === props.value));

  function addCustomMaterial() {
    setFeedback("");
    setError("");
    const displayName = name().trim();
    if (!displayName) {
      setError("Enter a material name.");
      return;
    }
    const stem = `custom_${
      displayName
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "_")
        .replace(/^_+|_+$/g, "") || "material"
    }`;
    const ids = new Set(materials().map((material) => material.id));
    let newId = stem;
    for (let suffix = 2; ids.has(newId); suffix++) newId = `${stem}_${suffix}`;
    const next = [...props.customMaterials, { id: newId, name: displayName, color: color() }];
    try {
      validateCustomTerrainMaterials(next);
      props.onCustomMaterialsChange(next);
      setSearch(displayName);
      setCategory("custom");
      setName("");
      setFeedback(`Added ${displayName}. Select it above to apply it.`);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  return (
    <fieldset class="material-picker" disabled={props.disabled}>
      <legend>{props.label ?? "Material"}</legend>
      <Show when={selected()}>
        {(material) => <p class="material-selection">Selected: {material().name}</p>}
      </Show>
      <LibraryBrowser
        search={search()}
        onSearch={setSearch}
        searchLabel="Search materials"
        placeholder="Search materials or IDs…"
        label={props.label ?? "Material library"}
        maxHeight="320px"
        summary={`${matches().length} of ${materials().length} materials`}
        empty={matches().length === 0}
        emptyMessage="No matching materials. Try another search or category."
        filters={
          <label>
            Category
            <select
              aria-label="Material category"
              value={category()}
              onChange={(event) => setCategory(event.currentTarget.value)}
            >
              <option value="">All categories</option>
              <For
                each={[
                  ["grass", "Grass"],
                  ["path", "Paths"],
                  ["river", "Rivers"],
                  ["other", "Other"],
                  ["custom", "Custom"],
                ]}
              >
                {(entry) => <option value={entry[0]}>{entry[1]}</option>}
              </For>
            </select>
          </label>
        }
      >
        <For each={matches()}>
          {(material) => (
            <button
              type="button"
              class="asset-card"
              disabled={props.selectionDisabled}
              aria-label={`Apply ${material.name}`}
              aria-pressed={props.value === material.id ? "true" : "false"}
              title={`${material.name} · ${material.id}`}
              onClick={() => props.onChange(material.id)}
            >
              <MaterialPreview material={material} />
              <span class="asset-card-info">
                <strong>{material.name}</strong>
              </span>
            </button>
          )}
        </For>
      </LibraryBrowser>
      <details>
        <summary>Add custom material</summary>
        <label for={`${id}-name`}>Material name</label>
        <input
          id={`${id}-name`}
          type="text"
          value={name()}
          onInput={(event) => setName(event.currentTarget.value)}
        />
        <label for={`${id}-color`}>Material color</label>
        <input
          id={`${id}-color`}
          type="color"
          value={color()}
          onInput={(event) => setColor(event.currentTarget.value)}
        />
        <button type="button" onClick={addCustomMaterial}>
          Add to map materials
        </button>
        <Show when={error()}>
          <p role="alert">{error()}</p>
        </Show>
        <Show when={feedback()}>
          <p role="status">{feedback()}</p>
        </Show>
      </details>
    </fieldset>
  );
}

let atlasImage: Promise<HTMLImageElement> | undefined;
function loadPreviewAtlas() {
  if (!atlasImage) {
    atlasImage = new Promise<HTMLImageElement>((resolve, reject) => {
      const image = new Image();
      image.onload = () => resolve(image);
      image.onerror = () => {
        atlasImage = undefined;
        reject(new Error("Could not load material preview atlas"));
      };
      image.src = previewAtlasUrl;
    });
  }
  return atlasImage;
}

/** Visible cards use prebuilt thumbnails; full terrain textures belong to the map renderer. */
function MaterialPreview(props: { material: CustomTerrainMaterial }) {
  let canvas!: HTMLCanvasElement;
  const [error, setError] = createSignal("");
  createEffect(
    () => props.material,
    (material) => {
      let active = true;
      setError("");
      canvas.dataset.previewReady = "false";
      const draw = async () => {
        try {
          const image = await loadPreviewAtlas();
          if (!active) return;
          const presetIndex = previewAtlas.materials.indexOf(material.id);
          const baseIndex = previewAtlas.bases.indexOf(material.textureBase ?? "dirt");
          const index = presetIndex >= 0 ? presetIndex : previewAtlas.materials.length + baseIndex;
          if (presetIndex < 0 && baseIndex < 0) throw new Error("Unknown material preview base");
          const context = canvas.getContext("2d");
          if (!context) throw new Error("Material previews need a 2D canvas context");
          const size = previewAtlas.tileSize;
          context.drawImage(
            image,
            (index % previewAtlas.columns) * size,
            Math.floor(index / previewAtlas.columns) * size,
            size,
            size,
            0,
            0,
            size,
            size,
          );
          if (presetIndex < 0) {
            const pixels = context.getImageData(0, 0, size, size);
            tintMaterialPreview(pixels.data, material.color);
            context.putImageData(pixels, 0, 0);
          }
          canvas.dataset.previewReady = "true";
        } catch (cause) {
          if (active) setError(cause instanceof Error ? cause.message : String(cause));
        }
      };
      const observer = new IntersectionObserver((entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          observer.disconnect();
          void draw();
        }
      });
      observer.observe(canvas);
      return () => {
        active = false;
        observer.disconnect();
      };
    },
  );
  return (
    <>
      <canvas
        ref={(element) => {
          canvas = element;
        }}
        width={previewAtlas.tileSize}
        height={previewAtlas.tileSize}
        class="material-preview"
        style={{ "background-color": props.material.color }}
        aria-hidden="true"
      />
      <Show when={error()}>
        <span class="hint" role="status">
          Preview unavailable: {error()}
        </span>
      </Show>
    </>
  );
}
