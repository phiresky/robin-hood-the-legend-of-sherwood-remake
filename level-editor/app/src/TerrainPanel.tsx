import LibraryPortal from "./LibraryPortal";
import MaterialPicker from "./MaterialPicker";
import ScrubNumber from "./ScrubNumber";
import { alignTerrainDiagonals, flattenTerrainVertices } from "./terrain-selection";
import type { EditorViewport } from "./editor-viewport";
import { For, Show, createSignal, createEffect, onCleanup, untrack } from "solid-js";
import {
  parseLevel3D,
  createTerrainGrid,
  subdivideTerrainCells,
  deleteTerrainVertices,
  type CustomTerrainMaterial,
  type TerrainGrid,
  type Level3D,
  type Vec3,
} from "@rle/shared";

export default function TerrainPanel(props: {
  document: () => Level3D | null;
  commit(document: Level3D): void;
  onError(message: string): void;
  disabled?: boolean;
  active?: boolean;
  viewport: EditorViewport;
  libraryMount?: HTMLElement;
}) {
  const [selected, setSelected] = createSignal<string[]>([]);
  const [cells, setCells] = createSignal<string[]>([]);
  const grid = () => props.document()?.terrain;
  const vertices = () => {
    const ids = new Set(selected());
    return grid()?.vertices.filter((vertex) => ids.has(vertex.id)) ?? [];
  };
  const selectedCells = () => {
    const ids = new Set(cells());
    return grid()?.cells.filter((cell) => ids.has(cell.id)) ?? [];
  };
  const center = (): Vec3 => {
    const selection = vertices();
    return [0, 1, 2].map(
      (axis) =>
        selection.reduce((total, vertex) => total + vertex.position[axis]!, 0) /
        Math.max(1, selection.length),
    ) as Vec3;
  };
  const vertexMaterial = (vertex: TerrainGrid["vertices"][number]) =>
    vertex.material ??
    Object.entries(vertex.materialMix ?? {}).sort((a, b) => b[1] - a[1])[0]?.[0] ??
    "grass_short";
  const walkability = () => {
    const selection = selectedCells();
    return selection.some((cell) => cell.walkable !== selection[0]?.walkable)
      ? "mixed"
      : selection[0]?.walkable === undefined
        ? "auto"
        : String(selection[0]?.walkable);
  };
  function clearSelection() {
    setSelected([]);
    setCells([]);
  }
  function customMaterials(materials: CustomTerrainMaterial[]) {
    const document = props.document();
    if (!document) return;
    try {
      const next = { ...document, customMaterials: materials };
      parseLevel3D(next);
      props.commit(next);
    } catch (error) {
      props.onError(String(error));
    }
  }
  function publish(terrain: TerrainGrid) {
    const document = props.document();
    if (!document) return;
    try {
      const next = { ...document, terrain };
      parseLevel3D(next);
      props.commit(next);
    } catch (error) {
      props.onError(String(error));
    }
  }
  function subdivide(ids: string[]) {
    const g = grid();
    if (!g || !ids.length) return;
    try {
      publish(subdivideTerrainCells(g, ids));
      setCells([]);
    } catch (error) {
      props.onError(String(error));
    }
  }
  function deleteVertices(ids: string[]) {
    const g = grid();
    if (!g || !ids.length) return;
    try {
      publish(deleteTerrainVertices(g, ids));
      clearSelection();
    } catch (error) {
      props.onError(String(error));
    }
  }
  createEffect(
    () => ({
      grid: grid(),
      camera: props.document()?.camera,
      selected: vertices().map((vertex) => vertex.id),
      cells: selectedCells().map((cell) => cell.id),
      enabled: props.active !== false && !props.disabled,
    }),
    ({ grid, camera, selected, cells, enabled }) =>
      untrack(() =>
        props.viewport.setTerrainEdit(
          grid && camera && enabled
            ? {
                grid,
                camera,
                selectedVertices: selected,
                selectVertices: setSelected,
                selectedCells: cells,
                selectCells: setCells,
                commit: publish,
                subdivideCells: subdivide,
                deleteVertices,
                deselect: clearSelection,
              }
            : null,
        ),
      ),
  );
  onCleanup(() => props.viewport.setTerrainEdit(null));
  const cancelPreview = () => props.viewport.previewTerrain(null);
  function position(axis: number, value: number, commit: boolean) {
    const g = grid(),
      selection = vertices();
    if (!g || !selection.length) return;
    const ids = new Set(selection.map((vertex) => vertex.id));
    const delta = value - center()[axis]!;
    const moved = {
      ...g,
      vertices: g.vertices.map((item) => {
        if (!ids.has(item.id)) return item;
        const position = [...item.position] as Vec3;
        position[axis] = position[axis]! + delta;
        return { ...item, position };
      }),
    };
    const next = axis === 2 && delta !== 0 ? alignTerrainDiagonals(moved, [...ids]) : moved;
    if (commit) {
      cancelPreview();
      publish(next);
    } else {
      try {
        parseLevel3D({ ...props.document()!, terrain: next });
        props.viewport.previewTerrain(next);
      } catch {
        cancelPreview();
      }
    }
  }
  function changeCell(patch: Partial<NonNullable<Level3D["terrain"]>["cells"][number]>) {
    const g = grid();
    const ids = new Set(selectedCells().map((cell) => cell.id));
    const corners = new Set(selectedCells().flatMap((cell) => cell.vertices));
    if (g && ids.size)
      publish({
        ...g,
        vertices: patch.material
          ? g.vertices.map((v, i) =>
              corners.has(i) ? { ...v, material: patch.material, materialMix: undefined } : v,
            )
          : g.vertices,
        cells: g.cells.map((c) => (ids.has(c.id) ? { ...c, ...patch } : c)),
      });
  }
  return (
    <section class="view-settings terrain-settings">
      <LibraryPortal mount={props.libraryMount} active={props.active !== false}>
        <p class="hint">
          {vertices().length
            ? `Apply a material to ${vertices().length} selected vertices.`
            : "Select terrain vertices, edges or cells to apply a material."}
        </p>
        <MaterialPicker
          label="Vertex material"
          value={vertices().length ? vertexMaterial(vertices()[0]!) : ""}
          customMaterials={props.document()?.customMaterials ?? []}
          onCustomMaterialsChange={customMaterials}
          disabled={!props.document() || props.disabled}
          selectionDisabled={!vertices().length}
          onChange={(material) => {
            const g = grid(),
              ids = new Set(selected()),
              cellIds = new Set(cells());
            if (g && ids.size)
              publish({
                ...g,
                vertices: g.vertices.map((item) =>
                  ids.has(item.id) ? { ...item, material, materialMix: undefined } : item,
                ),
                cells: g.cells.map((cell) => (cellIds.has(cell.id) ? { ...cell, material } : cell)),
              });
          }}
        />
      </LibraryPortal>
      <h2>Terrain grid</h2>
      <fieldset disabled={props.disabled || !props.document()}>
        <Show
          when={grid()}
          fallback={
            <button
              onClick={() => {
                const d = props.document();
                if (d)
                  publish(
                    createTerrainGrid(
                      d.exportBounds ?? [0, 0, ...(d.size ?? [1200, 900])],
                      128,
                      0,
                      "grass_short",
                      128 * Math.sin((d.camera.elevation_deg * Math.PI) / 180),
                    ),
                  );
              }}
            >
              Create terrain grid
            </button>
          }
        >
          <p class="hint">
            {grid()?.vertices.length} vertices · {grid()?.cells.length} cells. Click the ground to
            select a cell.
          </p>
          <Show when={vertices().length > 0}>
            <strong>
              {vertices().length === 1
                ? "Selected vertex"
                : `${vertices().length} selected vertices`}
            </strong>
            <Show when={vertices().length > 1}>
              <p class="hint">
                Coordinates show the selection center. Editing one translates every selected vertex
                by the same amount.
              </p>
              <button
                title={`Set selected vertices to their average Z (${center()[2]}), keeping X and Y unchanged`}
                onClick={() => {
                  const terrain = grid();
                  if (!terrain) return;
                  cancelPreview();
                  publish(
                    flattenTerrainVertices(
                      terrain,
                      vertices().map((vertex) => vertex.id),
                    ),
                  );
                }}
              >
                Flatten
              </button>
            </Show>
            <Show when={new Set(vertices().map(vertexMaterial)).size > 1}>
              <p class="hint">
                Mixed vertex materials. Choosing a material applies it to every selected vertex.
              </p>
            </Show>
            <For each={["X", "Y", "Z"]}>
              {(label, i) => (
                <ScrubNumber
                  label={`${vertices().length === 1 ? "Vertex" : "Selection center"} ${label}`}
                  value={center()[i()]!}
                  step={1}
                  onPreview={(value) => position(i(), value, false)}
                  onCommit={(value) => position(i(), value, true)}
                  onCancel={cancelPreview}
                />
              )}
            </For>
          </Show>
          <Show when={selectedCells().length > 0}>
            <strong>
              {selectedCells().length === 1
                ? "Selected cell"
                : `${selectedCells().length} selected cells`}
            </strong>
            <Show when={new Set(selectedCells().map((cell) => cell.material)).size > 1}>
              <p class="hint">
                Mixed cell materials. Choosing a material applies it to every selected cell and its
                corners.
              </p>
            </Show>
            <label>
              Walkability
              <select
                aria-label="Terrain walkability"
                value={walkability()}
                onChange={(e) =>
                  changeCell({
                    walkable:
                      e.currentTarget.value === "auto"
                        ? undefined
                        : e.currentTarget.value === "true",
                  })
                }
              >
                <Show when={walkability() === "mixed"}>
                  <option value="mixed" disabled>
                    Mixed
                  </option>
                </Show>
                <option value="auto">Use material</option>
                <option value="true">Walkable</option>
                <option value="false">Blocked</option>
              </select>
            </label>
            <button onClick={() => subdivide(selectedCells().map((cell) => cell.id))}>
              {selectedCells().length === 1
                ? "Subdivide selected cell"
                : "Subdivide selected cells"}
            </button>
          </Show>
        </Show>
      </fieldset>
    </section>
  );
}
