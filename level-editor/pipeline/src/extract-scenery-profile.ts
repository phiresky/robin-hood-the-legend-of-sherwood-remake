import fs from "node:fs/promises";
import path from "node:path";
import { parseArgs } from "node:util";
import { pathToFileURL } from "node:url";
import sharp from "sharp";
import { safeLibraryPath } from "../../shared/src/projection-assets.ts";
import { spriteAtlasRect } from "../../shared/src/sprite-atlas-rect.ts";
import { validateSceneryManifest } from "../../app/src/scenery-manifest.ts";
import { sanitizedProfileName } from "../../app/src/sprite-profiles.ts";

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Expected atlas record");
  return value as Record<string, unknown>;
}

/** One-time authoring conversion: a selected sprite profile becomes a portable PNG bank. */
export async function extractSceneryProfile(
  input: unknown,
  profileName: string,
  read: (name: string) => Promise<Uint8Array>,
) {
  const source = record(input);
  if (
    source.atlas !== undefined &&
    (typeof source.atlas !== "string" || !safeLibraryPath(source.atlas))
  )
    throw new Error("Missing or invalid sprite atlas path");
  if (!Array.isArray(source.profiles)) throw new Error("Missing sprite profiles");
  const matching = source.profiles.map(record).filter((profile) => profile.name === profileName);
  if (matching.length !== 1) throw new Error(`Missing or duplicate atlas profile: ${profileName}`);
  const profile = matching[0]!;
  if (!Array.isArray(profile.rows) || !profile.rows.length) throw new Error("Missing sprite rows");
  const atlas =
    typeof source.atlas === "string"
      ? await sharp(await read(source.atlas))
          .ensureAlpha()
          .raw()
          .toBuffer({ resolveWithObject: true })
      : null;
  const files: Record<string, Uint8Array> = {};
  const rows = [];
  for (const [rowIndex, value] of profile.rows.entries()) {
    const row = record(value);
    if (!Array.isArray(row.frames) || !row.frames.length) throw new Error("Missing sprite frames");
    const frames = [];
    for (const [frameIndex, value] of row.frames.entries()) {
      const frame = record(value);
      const file = `${rowIndex}-${frameIndex}.png`;
      if (atlas) {
        if (frame.file !== source.atlas)
          throw new Error("Frame references a different sprite atlas");
        const [left, top, width, height] = spriteAtlasRect(
          frame.rect,
          atlas.info.width,
          atlas.info.height,
        );
        files[file] = await sharp(atlas.data, { raw: atlas.info })
          .extract({ left, top, width, height })
          .png()
          .toBuffer();
      } else {
        if (typeof row.path !== "string" || typeof frame.file !== "string")
          throw new Error("Missing sprite frame path");
        const sourceFile = `${row.path && row.path !== "." ? row.path + "/" : ""}${frame.file}`;
        if (!safeLibraryPath(sourceFile)) throw new Error("Invalid sprite frame path");
        try {
          files[file] = await read(sourceFile);
        } catch (error) {
          if (
            !(error instanceof Error) ||
            !("code" in error) ||
            error.code !== "ENOENT" ||
            source.profiles.length === 1
          )
            throw error;
          // Converted multi-profile banks put otherwise identical row paths in
          // profile folders; native authored banks use paths relative to the bank.
          const nested = `${sanitizedProfileName(profileName)}/${sourceFile}`;
          if (!safeLibraryPath(nested))
            throw new Error("Invalid sprite profile path", { cause: error });
          files[file] = await read(nested);
        }
      }
      frames.push({
        file,
        delay: frame.delay,
        distance: frame.distance,
        offset_x: frame.offset_x,
        offset_y: frame.offset_y,
        sound_id: frame.sound_id,
      });
    }
    rows.push({
      action_id: row.action_id,
      action_done: row.action_done,
      average_speed: row.average_speed,
      direction: row.direction,
      hotspot_x: row.hotspot_x,
      hotspot_y: row.hotspot_y,
      path: ".",
      frames,
    });
  }
  const manifest = {
    pixel_format: source.pixel_format ?? "legacy_color_keys",
    profiles: [
      {
        name: profile.name,
        width: profile.width,
        height: profile.height,
        center_x: profile.center_x,
        center_y: profile.center_y,
        rows,
      },
    ],
  };
  // Use the same admission rules as export before returning any authoring output.
  const validated = await validateSceneryManifest(manifest, files);
  files["manifest.json"] = new TextEncoder().encode(JSON.stringify(manifest, null, 2) + "\n");
  return { files, profile: validated.profiles[0]! };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const { values } = parseArgs({
    options: {
      bank: { type: "string" },
      profile: { type: "string" },
      out: { type: "string" },
    },
  });
  if (!values.bank || !values.profile || !values.out)
    throw new Error("Usage: --bank <source.rhs.d> --profile <profile-name> --out <new-bank.rhs.d>");
  const root = await fs.realpath(values.bank);
  const result = await extractSceneryProfile(
    JSON.parse(await fs.readFile(path.join(root, "manifest.json"), "utf8")),
    values.profile,
    async (name) => {
      const file = await fs.realpath(path.join(root, name));
      if (!file.startsWith(root + path.sep)) throw new Error("Sprite atlas escapes bank directory");
      return fs.readFile(file);
    },
  );
  await fs.mkdir(values.out);
  for (const [name, bytes] of Object.entries(result.files))
    await fs.writeFile(path.join(values.out, name), bytes, { flag: "wx" });
  console.log(JSON.stringify({ profile: result.profile, files: Object.keys(result.files).length }));
}
