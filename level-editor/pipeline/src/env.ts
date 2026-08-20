import { config } from "dotenv";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
export const editorRoot = path.resolve(here, "../..");
export const repoRoot = path.resolve(editorRoot, "..");
export const libraryDir = path.join(editorRoot, "library");
export const workDir = path.join(editorRoot, "work");

config({ path: path.join(editorRoot, ".env"), quiet: true });

export function datadirPath(): string {
  return process.env.HACKABLE_DATADIR ?? path.join(repoRoot, "datadirs", "fullgame_gog_hackable");
}

export function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`missing ${name} (set it in level-editor/.env)`);
  return v;
}
