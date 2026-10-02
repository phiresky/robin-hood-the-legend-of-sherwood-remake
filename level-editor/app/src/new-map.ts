import { createTerrainGrid, parseLevel3D, DEFAULT_LIGHTING } from "@rle/shared";
import { listFiles, writeText } from "./fs.ts";
import { defaultNewMapOptions, validateNewMapOptions, type NewMapOptions } from "./workspace.ts";
export { defaultNewMapOptions, type NewMapOptions } from "./workspace.ts";

export function validateNewMap(name: string) {
  name = name.trim();
  if (!/^[A-Za-z0-9][A-Za-z0-9 _-]{0,63}$/.test(name))
    throw new Error(
      "Use 1–64 letters, numbers, spaces, hyphens or underscores; start with a letter or number.",
    );
  return name;
}

/** A new map is one document; geometry is referenced only after assets are placed. */
export async function createNewMap(
  library: FileSystemDirectoryHandle,
  rawName: string,
  options: NewMapOptions = defaultNewMapOptions(),
) {
  const name = validateNewMap(rawName);
  validateNewMapOptions(options);
  const directory = await library.getDirectoryHandle("scenes", { create: true });
  const documentName = `${name}.rhlos-map.json`;
  const names = [documentName];
  const existing = new Set((await listFiles(directory)).map((file) => file.toLowerCase()));
  if (names.some((file) => existing.has(file.toLowerCase())))
    throw new Error(`A map named “${name}” already exists. Choose a different name.`);

  const document = parseLevel3D({
    version: 1,
    lighting: { ...DEFAULT_LIGHTING, enabled: true },
    map: name,
    size: [...options.size],
    terrain: createTerrainGrid(
      [0, 0, ...options.size],
      options.spacing,
      options.height,
      "grass_short",
      options.spacing * Math.sin((35 * Math.PI) / 180),
    ),
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    sceneAssets: [],
    groups: [],
    objects: [],
  });
  const created: string[] = [];
  try {
    created.push(documentName);
    await writeText(directory, documentName, JSON.stringify(document, null, 2));
  } catch (error) {
    const cleanup = await Promise.allSettled(created.map((file) => directory.removeEntry(file)));
    const failures = cleanup.filter((result) => result.status === "rejected");
    if (failures.length)
      throw new AggregateError(
        [error, ...failures.map((result) => result.reason)],
        "Map creation failed and incomplete files could not be removed",
        { cause: error },
      );
    throw error;
  }
  return name;
}
