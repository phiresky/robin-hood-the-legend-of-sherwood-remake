import { For } from "solid-js";

const controls: Record<string, [string, string][]> = {
  Assets: [
    ["Place an asset", "Drag from the library onto the map"],
    ["Select / move", "Click / drag an object"],
    ["Select a part", "Alt-click"],
    ["Rotate", "Q / E"],
    ["Duplicate / delete", "D / Delete"],
    ["Pan", "Left-drag empty ground"],
  ],
  Paths: [
    ["Start a path", "Choose a footpath, river, wall or fence in the library"],
    ["Add points", "Click empty ground"],
    ["Move a point", "Drag a path point"],
    ["Surface design", "Select a point, then choose a material on the left"],
    ["Finish / exit", "Enter finishes a new path; Escape exits editing"],
    ["Remove a point", "Delete / Backspace"],
    ["Pan", "Left-drag empty ground"],
  ],
  Terrain: [
    ["Select", "Click a vertex, edge or cell; hover highlights what will move"],
    ["Extend selection", "Shift-click or Shift-drag a selection box"],
    ["Raise / lower", "Drag selected terrain vertically"],
    ["Move horizontally", "Alt-drag"],
    ["Subdivide", "Double-click a cell, edge or vertex"],
    ["Remove vertices", "Delete reconnects the surrounding ground"],
    ["Flatten", "Select multiple vertices, then click Flatten"],
    ["Material", "Choose from the library on the left"],
    ["Cancel a drag", "Escape"],
    ["Pan", "Left-drag empty ground"],
  ],
  Mission: [
    ["Place a character", "Choose PCs or NPCs, then drag a character onto the map"],
    ["Select / move", "Click / drag a placed character"],
    ["Edit / delete", "Use the selected character’s details on the right"],
    ["Pan", "Left-drag empty ground"],
  ],
};
const common: [string, string][] = [
  ["Orbit / zoom", "Right-drag / mouse wheel"],
  ["Frame / game camera", "F / G"],
  ["Save / undo", "Ctrl or ⌘ + S / Z"],
];

export default function EditorHelp(props: {
  mode: string;
  hasMissionLoader?: boolean;
  onClose: () => void;
}) {
  return (
    <div id="editor-help" class="viewport-help" role="region" aria-label={`${props.mode} help`}>
      <div class="detail-head">
        <h2>{props.mode} controls</h2>
        <button aria-label="Close help" onClick={props.onClose}>
          ×
        </button>
      </div>
      <dl>
        <For
          each={[
            ...(props.mode === "Mission" && props.hasMissionLoader
              ? [["Load a mission", "Choose a mission above the character library"]]
              : []),
            ...(controls[props.mode] ?? []),
            ...common,
          ]}
        >
          {([action, gesture]) => (
            <>
              <dt>{action}</dt>
              <dd>{gesture}</dd>
            </>
          )}
        </For>
      </dl>
    </div>
  );
}
