import { config } from "dotenv";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
export const editorRoot = path.resolve(here, "../..");
export const repoRoot = path.resolve(editorRoot, "..");
export const libraryDir = path.join(editorRoot, "library");
export const workDir = path.join(editorRoot, "work");

/** Explicit/lazy initialization: importing geometry/provider modules never mutates process.env. */
export function loadEnvironment(): void {
  config({ path: path.join(editorRoot, ".env"), quiet: true });
}

export function datadirPath(): string {
  loadEnvironment();
  return (
    process.env.HACKABLE_DATADIR ??
    path.join(repoRoot, "datadirs", "fullgame_gog_hackable")
  );
}

export function requireEnv(name: string): string {
  loadEnvironment();
  const v = process.env[name];
  if (!v) throw new Error(`missing ${name} (set it in level-editor/.env)`);
  return v;
}
