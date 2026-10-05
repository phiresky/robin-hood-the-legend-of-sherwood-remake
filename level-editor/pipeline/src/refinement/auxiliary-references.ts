import fs from "node:fs/promises";
import path from "node:path";
import crypto from "node:crypto";
import sharp from "sharp";

const sha = (bytes: Buffer) => crypto.createHash("sha256").update(bytes).digest("hex");
type CropReference = {
  file: string;
  sha256: string;
  source: "input" | "lighting";
  crop: { left: number; top: number; width: number; height: number };
  scale: number;
};
type MaterialReference = {
  file: string;
  sha256: string;
  source: "material";
  asset_id: string;
  role: string;
};
type RegionGuide = {
  file: string;
  sha256: string;
  source: "region-guide";
  role: string;
};
type Reference = CropReference | MaterialReference | RegionGuide;

/** Keep target-bound crops distinct from supplementary material examples. */
export async function auxiliaryReferences(
  file: string | null,
  input: Buffer,
  lighting: Buffer | null,
) {
  if (!file) return { images: [] as Buffer[], evidence: null, instructions: "" };
  if (!lighting)
    throw new Error("Auxiliary references require the calibrated lighting reference second");
  const bytes = await fs.readFile(file);
  const manifest = JSON.parse(bytes.toString()) as {
    input_sha256: string;
    lighting_sha256: string;
    references: Reference[];
  };
  if (manifest.input_sha256 !== sha(input) || manifest.lighting_sha256 !== sha(lighting))
    throw new Error("Auxiliary references do not bind the approved input and lighting");
  if (
    !Array.isArray(manifest.references) ||
    !manifest.references.length ||
    manifest.references.filter(reference => reference.source !== "region-guide").length > 4 ||
    manifest.references.filter(reference => reference.source === "region-guide").length > 1
  )
    throw new Error("Supply up to four supplementary references and at most one aligned region guide");
  const images: Buffer[] = [];
  const records = [];
  const descriptions: string[] = [];
  for (const reference of manifest.references) {
    if (reference.source === "region-guide") {
      if (!reference.role?.trim()) throw new Error("Region guide requires an explicit region legend");
      const image = await fs.readFile(path.resolve(path.dirname(file), reference.file));
      if (sha(image) !== reference.sha256) throw new Error("Region guide hash changed");
      const [metadata, target] = await Promise.all([sharp(image).metadata(), sharp(input).metadata()]);
      if (metadata.format !== "png" || metadata.width !== target.width || metadata.height !== target.height)
        throw new Error("Region guide must match the exact input canvas");
      images.push(image);
      records.push(reference);
      descriptions.push(`Image ${images.length + 2} is an aligned region guide sent as an ordinary image, not a provider edit mask. ${reference.role} Use only its region locations; never copy its diagnostic colors or markings into the output. Local compositing enforces protected pixels independently.`);
      continue;
    }
    if (reference.source === "material") {
      if (!reference.asset_id?.trim() || !reference.role?.trim())
        throw new Error("Material examples require an asset ID and material role");
      const image = await fs.readFile(path.resolve(path.dirname(file), reference.file));
      if (sha(image) !== reference.sha256) throw new Error("Material reference hash changed");
      const metadata = await sharp(image).metadata();
      if (metadata.format !== "png" || !metadata.width || !metadata.height ||
          Math.max(metadata.width, metadata.height) > 3840)
        throw new Error("Material reference must be a PNG no larger than 3840 pixels per edge");
      images.push(image);
      records.push(reference);
      descriptions.push(`Image ${images.length + 2} is a supplementary material example from ${reference.asset_id}: ${reference.role}. Use its texture character and material detail only; do not copy its shape, proportions, camera, lighting, background, or gray unknown patches.`);
      continue;
    }
    if (reference.source !== "input" && reference.source !== "lighting")
      throw new Error("Auxiliary crop source must be input or lighting");
    const { left, top, width, height } = reference.crop;
    if (
      ![left, top, width, height, reference.scale].every(Number.isInteger) ||
      left < 0 ||
      top < 0 ||
      width < 1 ||
      height < 1 ||
      reference.scale < 1 ||
      reference.scale > 16
    )
      throw new Error("Invalid auxiliary crop geometry");
    const source = reference.source === "input" ? input : lighting;
    const metadata = await sharp(source).metadata();
    if (
      left + width > metadata.width ||
      top + height > metadata.height ||
      Math.max(width, height) * reference.scale > 3840
    )
      throw new Error("Auxiliary crop exceeds approved source or reference size");
    const image = await fs.readFile(path.resolve(path.dirname(file), reference.file));
    if (sha(image) !== reference.sha256) throw new Error("Auxiliary reference hash changed");
    const expected = await sharp(source)
      .extract(reference.crop)
      .resize(width * reference.scale, height * reference.scale, { kernel: "nearest" })
      .ensureAlpha()
      .raw()
      .toBuffer();
    const actual = await sharp(image).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
    if (
      actual.info.width !== width * reference.scale ||
      actual.info.height !== height * reference.scale ||
      !actual.data.equals(expected)
    )
      throw new Error("Auxiliary reference is not an exact magnified approved crop");
    images.push(image);
    records.push({ ...reference, source_sha256: sha(source) });
    descriptions.push(`Image ${images.length + 2} is a ${reference.scale}x crop of the ${reference.source === "input" ? "first" : "second"} image at full-sheet pixel box (${reference.crop.left},${reference.crop.top},${reference.crop.width},${reference.crop.height}).`);
  }
  return {
    images,
    evidence: { manifest_sha256: sha(bytes), references: records },
    instructions:
      " Additional references are explanatory examples, not replacement views. " +
      descriptions.join(" ") +
      " Preserve target geometry, protected source pixels, and the second image's calibrated lighting. Return only the complete first image at its original dimensions and original layout.",
  };
}
