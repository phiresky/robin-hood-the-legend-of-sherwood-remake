/** Lossless geometry packaging. The EXT adapter bridges glTF Transform 4.x to KHR/v1. */
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { NodeIO, Logger, type GLTF } from "@gltf-transform/core";
import { ALL_EXTENSIONS } from "@gltf-transform/extensions";
import { MeshoptEncoder, MeshoptDecoder } from "meshoptimizer";

const EXT = "EXT_meshopt_compression",
  KHR = "KHR_meshopt_compression";

function renameMeshoptJson(json: GLTF.IGLTF, from: string, to: string): void {
  for (const record of [...(json.buffers ?? []), ...(json.bufferViews ?? [])]) {
    if (record.extensions?.[from]) {
      record.extensions[to] = record.extensions[from];
      delete record.extensions[from];
    }
  }
  for (const key of ["extensionsUsed", "extensionsRequired"] as const) {
    if (json[key]) json[key] = json[key].map((name: string) => (name === from ? to : name));
  }
}

/** Resolve external authoring resources before adapting the KHR extension for glTF Transform. */
export async function readMeshoptDocument(io: NodeIO, input: string) {
  const document = await io.readAsJSON(input);
  renameMeshoptJson(document.json, KHR, EXT);
  return io.readJSON(document);
}

/** Rename only extension slots, never user extras. Binary offsets are relative to the BIN chunk. */
export function meshoptExtension(bytes: Uint8Array, from: string, to: string): Uint8Array {
  const b = Buffer.from(bytes);
  assert.equal(b.readUInt32LE(0), 0x46546c67);
  assert.equal(b.readUInt32LE(4), 2);
  assert.equal(b.readUInt32LE(8), b.length);
  assert.equal(b.readUInt32LE(16), 0x4e4f534a);
  const length = b.readUInt32LE(12);
  const doc = JSON.parse(b.toString("utf8", 20, 20 + length));
  if (!doc.extensionsUsed?.includes(from)) return bytes;
  renameMeshoptJson(doc, from, to);
  const json = Buffer.from(JSON.stringify(doc));
  const padded = Buffer.alloc(Math.ceil(json.length / 4) * 4, 32);
  json.copy(padded);
  const tail = b.subarray(20 + length),
    header = Buffer.from(b.subarray(0, 20));
  header.writeUInt32LE(20 + padded.length + tail.length, 8);
  header.writeUInt32LE(padded.length, 12);
  return Buffer.concat([header, padded, tail]);
}

export const MeshoptV1Encoder: typeof MeshoptEncoder = {
  ...MeshoptEncoder,
  encodeGltfBuffer(source, count, size, mode) {
    const encoded = MeshoptEncoder.encodeGltfBuffer(source, count, size, mode, 1);
    const decoded = new Uint8Array(count * size);
    MeshoptDecoder.decodeGltfBuffer(decoded, count, size, encoded, mode);
    if (mode === "TRIANGLES") {
      // The triangle codec may cyclically rotate corners, preserving winding and vertex data.
      const Type = size === 2 ? Uint16Array : Uint32Array;
      const before = new Type(new Uint8Array(source).buffer, 0, count);
      const after = new Type(decoded.buffer, decoded.byteOffset, count);
      for (let i = 0; i < count; i += 3) {
        assert.ok(
          [0, 1, 2].some((r) => [0, 1, 2].every((k) => before[i + k] === after[i + ((k + r) % 3)])),
          "Meshopt changed a triangle",
        );
      }
    } else {
      assert.ok(Buffer.from(decoded).equals(Buffer.from(source)), "Meshopt changed vertex data");
    }
    return encoded;
  },
};

export async function meshoptIO(): Promise<NodeIO> {
  await Promise.all([MeshoptEncoder.ready, MeshoptDecoder.ready]);
  return new NodeIO()
    .registerExtensions(ALL_EXTENSIONS)
    .registerDependencies({
      "meshopt.encoder": MeshoptV1Encoder,
      "meshopt.decoder": MeshoptDecoder,
    })
    .setLogger(new Logger(Logger.Verbosity.SILENT));
}

export const meshoptReadable = (bytes: Uint8Array): Uint8Array => meshoptExtension(bytes, KHR, EXT);
export const meshoptV1 = (bytes: Uint8Array): Uint8Array => meshoptExtension(bytes, EXT, KHR);

type CompressedView = {
  buffer: number;
  byteOffset?: number;
  byteLength: number;
  byteStride: number;
  count: number;
  mode: "ATTRIBUTES" | "TRIANGLES" | "INDICES";
  filter?: string;
};

export function parseGlb(bytes: Uint8Array): { json: GLTF.IGLTF; bin: Uint8Array } {
  const b = Buffer.from(bytes);
  assert.equal(b.readUInt32LE(0), 0x46546c67);
  assert.equal(b.readUInt32LE(4), 2);
  assert.equal(b.readUInt32LE(8), b.length);
  assert.equal(b.readUInt32LE(16), 0x4e4f534a);
  const length = b.readUInt32LE(12),
    end = 20 + length;
  assert.ok(end <= b.length);
  const json = JSON.parse(b.toString("utf8", 20, end)) as GLTF.IGLTF;
  if (end === b.length) return { json, bin: new Uint8Array() };
  assert.equal(b.readUInt32LE(end + 4), 0x004e4942);
  assert.equal(end + 8 + b.readUInt32LE(end), b.length);
  return { json, bin: b.subarray(end + 8) };
}

function writeGlb(json: GLTF.IGLTF, bin: Uint8Array): Uint8Array {
  const text = Buffer.from(JSON.stringify(json)),
    padded = Buffer.alloc(Math.ceil(text.length / 4) * 4, 32);
  text.copy(padded);
  const body = Buffer.alloc(Math.ceil(bin.length / 4) * 4);
  body.set(bin);
  const header = Buffer.alloc(20),
    binHeader = Buffer.alloc(8);
  header.writeUInt32LE(0x46546c67, 0);
  header.writeUInt32LE(2, 4);
  header.writeUInt32LE(28 + padded.length + body.length, 8);
  header.writeUInt32LE(padded.length, 12);
  header.writeUInt32LE(0x4e4f534a, 16);
  binHeader.writeUInt32LE(body.length, 0);
  binHeader.writeUInt32LE(0x004e4942, 4);
  return Buffer.concat([header, padded, binHeader, body]);
}

function chunks() {
  const parts: Uint8Array[] = [];
  let length = 0;
  return {
    add(data: Uint8Array) {
      const offset = length;
      parts.push(data);
      length += data.length;
      const pad = (4 - (length % 4)) % 4;
      parts.push(new Uint8Array(pad));
      length += pad;
      return offset;
    },
    finish: () => Buffer.concat(parts),
  };
}

/** Repack buffer views, preserving all scene metadata, accessor bytes and texture payloads. */
export async function decodeGlb(bytes: Uint8Array): Promise<Uint8Array> {
  const { json, bin } = parseGlb(bytes);
  if (!json.bufferViews?.some((v) => v.extensions?.[KHR] || v.extensions?.[EXT])) return bytes;
  await MeshoptDecoder.ready;
  const body = chunks();
  for (const view of json.bufferViews) {
    const ext = (view.extensions?.[KHR] ?? view.extensions?.[EXT]) as CompressedView | undefined;
    let data: Uint8Array;
    if (ext) {
      assert.equal(ext.buffer, 0, "External meshopt buffers are unsupported");
      data = new Uint8Array(ext.count * ext.byteStride);
      assert.equal(data.length, view.byteLength);
      const offset = ext.byteOffset ?? 0;
      assert.ok(offset + ext.byteLength <= bin.length);
      MeshoptDecoder.decodeGltfBuffer(
        data,
        ext.count,
        ext.byteStride,
        bin.subarray(offset, offset + ext.byteLength),
        ext.mode,
        ext.filter,
      );
      delete view.extensions![KHR];
      delete view.extensions![EXT];
      if (!Object.keys(view.extensions!).length) delete view.extensions;
    } else {
      assert.equal(view.buffer, 0, "External buffers are unsupported");
      const offset = view.byteOffset ?? 0;
      assert.ok(offset + view.byteLength <= bin.length);
      data = bin.subarray(offset, offset + view.byteLength);
    }
    view.buffer = 0;
    view.byteOffset = body.add(data);
  }
  const result = body.finish();
  json.buffers = [{ byteLength: result.length }];
  for (const key of ["extensionsUsed", "extensionsRequired"] as const) {
    if (json[key]) json[key] = json[key].filter((name) => name !== EXT && name !== KHR);
  }
  return writeGlb(json, result);
}

/** Upgrade existing compressed previews without applying their lossy filters a second time. */
export async function upgradeMeshopt(bytes: Uint8Array): Promise<Uint8Array> {
  await Promise.all([MeshoptEncoder.ready, MeshoptDecoder.ready]);
  const { json, bin } = parseGlb(bytes);
  if (!json.bufferViews?.some((v) => v.extensions?.[EXT])) return bytes;
  const body = chunks();
  for (const view of json.bufferViews) {
    const ext = (view.extensions?.[EXT] ?? view.extensions?.[KHR]) as CompressedView | undefined;
    if (ext) {
      assert.equal(ext.buffer, 0);
      const offset = ext.byteOffset ?? 0;
      assert.ok(offset + ext.byteLength <= bin.length);
      const raw = new Uint8Array(ext.count * ext.byteStride);
      // Decode only the codec here: filters remain in the output extension unchanged.
      MeshoptDecoder.decodeGltfBuffer(
        raw,
        ext.count,
        ext.byteStride,
        bin.subarray(offset, offset + ext.byteLength),
        ext.mode,
      );
      const encoded = MeshoptV1Encoder.encodeGltfBuffer(raw, ext.count, ext.byteStride, ext.mode);
      ext.byteOffset = body.add(encoded);
      ext.byteLength = encoded.length;
    } else {
      assert.equal(view.buffer, 0);
      const offset = view.byteOffset ?? 0;
      assert.ok(offset + view.byteLength <= bin.length);
      view.byteOffset = body.add(bin.subarray(offset, offset + view.byteLength));
    }
  }
  const result = body.finish();
  json.buffers![0]!.byteLength = result.length;
  return meshoptV1(writeGlb(json, result));
}

/** No quantization or simplification. Tiny models retain their original bytes when smaller. */
export async function compressGlb(bytes: Uint8Array): Promise<Uint8Array> {
  await Promise.all([MeshoptEncoder.ready, MeshoptDecoder.ready]);
  const decoded = await decodeGlb(bytes);
  const { json, bin } = parseGlb(decoded);
  if (!json.bufferViews?.length) return bytes;
  assert.ok(json.buffers?.length === 1 && !json.buffers[0]!.uri, "Expected embedded geometry");
  const imageViews = new Set((json.images ?? []).map((image) => image.bufferView));
  const body = chunks();
  let fallbackSize = 0,
    compressed = 0;
  for (const [index, view] of json.bufferViews.entries()) {
    assert.equal(view.buffer, 0);
    const offset = view.byteOffset ?? 0;
    assert.ok(offset + view.byteLength <= bin.length);
    const raw = bin.subarray(offset, offset + view.byteLength);
    const accessors = (json.accessors ?? []).filter((a) => a.bufferView === index);
    let stride = view.byteStride;
    let mode: CompressedView["mode"] = "ATTRIBUTES";
    if (accessors.length === 1 && view.target === 34963) {
      const accessor = accessors[0]!;
      if (accessor.componentType === 5123 || accessor.componentType === 5125) {
        stride = accessor.componentType === 5123 ? 2 : 4;
        // Sequence encoding preserves even corner order, useful to editing/inspection tools.
        mode = "INDICES";
      }
    } else if (!stride && accessors.length === 1) {
      const a = accessors[0]!;
      const components = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4, MAT2: 4, MAT3: 9, MAT4: 16 }[
        a.type
      ];
      const width = (
        { 5120: 1, 5121: 1, 5122: 2, 5123: 2, 5125: 4, 5126: 4 } as Record<number, number>
      )[a.componentType];
      if (components && width) stride = components * width;
    }
    const eligible =
      !imageViews.has(index) &&
      accessors.length > 0 &&
      stride &&
      stride <= 256 &&
      raw.length > 0 &&
      raw.length % stride === 0 &&
      (mode === "INDICES" || stride % 4 === 0) &&
      !view.extensions;
    if (!eligible || !stride) {
      view.byteOffset = body.add(raw);
      continue;
    }
    const count = raw.length / stride;
    const encoded = MeshoptV1Encoder.encodeGltfBuffer(raw, count, stride, mode);
    const ext: CompressedView = {
      buffer: 0,
      byteOffset: body.add(encoded),
      byteLength: encoded.length,
      byteStride: stride,
      count,
      mode,
    };
    view.buffer = 1;
    view.byteOffset = fallbackSize;
    fallbackSize += Math.ceil(view.byteLength / 4) * 4;
    view.extensions = { [KHR]: ext };
    compressed++;
  }
  if (!compressed) return bytes;
  const result = body.finish();
  json.buffers = [
    { byteLength: result.length },
    { byteLength: fallbackSize, extensions: { [KHR]: { fallback: true } } },
  ];
  for (const key of ["extensionsUsed", "extensionsRequired"] as const) {
    json[key] = [...new Set([...(json[key] ?? []), KHR])];
  }
  const candidate = writeGlb(json, result);
  // Replacing an old EXT payload must not retain v0: compare against its decoded form instead.
  const baseline = decoded === bytes ? bytes : decoded;
  return candidate.length < baseline.length ? candidate : baseline;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [mode, input, output] = process.argv.slice(2);
  if (mode === "--batch") {
    if (!input) throw new Error("Missing job manifest");
    const jobs = JSON.parse(await fs.readFile(input, "utf8")) as {
      input: string;
      output: string;
      preview: boolean;
      sha256: string;
    }[];
    for (const [i, job] of jobs.entries()) {
      const bytes = await fs.readFile(job.input);
      assert.equal(
        createHash("sha256").update(bytes).digest("hex"),
        job.sha256,
        `Asset changed: ${job.input}`,
      );
      const result = await (job.preview ? upgradeMeshopt(bytes) : compressGlb(bytes));
      await fs.mkdir(path.dirname(job.output), { recursive: true });
      await fs.writeFile(job.output, result);
      if (i % 50 === 0) console.error(`Meshopt: ${i + 1}/${jobs.length}`);
    }
  } else {
    if (mode !== "--encode" && mode !== "--decode")
      throw new Error("Expected --encode or --decode [input output], otherwise stdin/stdout");
    const parts: Buffer[] = [];
    if (!input) for await (const part of process.stdin) parts.push(Buffer.from(part));
    const bytes = input ? await fs.readFile(input) : Buffer.concat(parts);
    const result = await (mode === "--encode" ? compressGlb(bytes) : decodeGlb(bytes));
    if (output) await fs.writeFile(output, result);
    else process.stdout.write(result);
  }
}
