import fs from "node:fs/promises";
import { isMissing } from "./provider-cache.ts";

/** Only an absent optional default means no document. Explicit paths, bad JSON,
 * and permission/I/O failures must stop before reconstruction or export. */
export async function readDocument(
  file: string,
  required: boolean,
): Promise<unknown | undefined> {
  let text: string;
  try {
    text = await fs.readFile(file, "utf8");
  } catch (error) {
    if (!required && isMissing(error)) return undefined;
    throw new Error(`cannot read document ${file}`, { cause: error });
  }
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`invalid JSON document ${file}`, { cause: error });
  }
}

export function pathComponent(value: string, label: string): string {
  if (!/^[a-zA-Z0-9_-]+$/.test(value))
    throw new Error(`invalid ${label}: ${value}`);
  return value;
}
