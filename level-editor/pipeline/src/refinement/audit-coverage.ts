import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import sharp from "sharp";

export type ObjectCoverage = {
  file: string;
  sha256: string;
  channel: "alpha" | "red";
};

/** Read a pinned renderer coverage buffer, independent of the displayed background. */
export async function auditCoverage(
  directory: string,
  coverage: ObjectCoverage | undefined,
  width: number,
  height: number,
): Promise<Uint8Array> {
  if (!coverage || !coverage.file || !/^[a-f0-9]{64}$/.test(coverage.sha256) ||
      (coverage.channel !== "alpha" && coverage.channel !== "red")) {
    throw new Error("--audit-input requires explicit pinned object_coverage (file, sha256, channel). Prepare a new coverage-bound manifest; RGB background guessing is unsupported.");
  }
  const bytes = await fs.readFile(path.resolve(directory, coverage.file));
  if (createHash("sha256").update(bytes).digest("hex") !== coverage.sha256)
    throw new Error("Object coverage hash changed");
  const metadata = await sharp(bytes).metadata();
  if (metadata.format !== "png" || metadata.width !== width || metadata.height !== height)
    throw new Error("Object coverage must be a PNG matching the source-view dimensions");
  if (coverage.channel === "alpha" && !metadata.hasAlpha)
    throw new Error("Object coverage requests alpha but the PNG has no alpha channel");
  const pixels = await sharp(bytes).toColourspace("srgb").ensureAlpha().raw().toBuffer();
  const result = new Uint8Array(width * height);
  for (let i = 0; i < result.length; i++) {
    if (coverage.channel === "red" &&
        (pixels[i * 4] !== pixels[i * 4 + 1] || pixels[i * 4] !== pixels[i * 4 + 2]))
      throw new Error("Red-channel object coverage must be a grayscale mask");
    result[i] = Number(pixels[i * 4 + (coverage.channel === "alpha" ? 3 : 0)]! > 0);
  }
  return result;
}
