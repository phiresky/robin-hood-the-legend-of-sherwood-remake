import fs from "node:fs/promises";
import { configureAssetGameplay, type GameplayEdit } from "./configure-surface-jumps.ts";

const [library, configuration, output, mode] = process.argv.slice(2);
if (!library || !configuration || !output || (mode !== undefined && mode !== "--apply"))
  throw new Error(
    "Usage: configure-spline-gameplay.ts library edits.json new-backup-directory [--apply]",
  );
const edits = JSON.parse(await fs.readFile(configuration, "utf8")) as GameplayEdit[];
console.log(
  JSON.stringify(await configureAssetGameplay(library, edits, output, mode === "--apply")),
);
