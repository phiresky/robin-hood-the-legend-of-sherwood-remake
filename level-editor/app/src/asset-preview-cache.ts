import * as THREE from "three";
import type { ProjectionAssetEntry } from "@rle/shared";
import { loadProjectionAssetPreview } from "./projection-library.ts";
import { disposeObjectResources } from "./resources.ts";

export interface AssetPreviewLease {
  /** Independent transform hierarchy; geometry, materials and textures belong to the cache. */
  asset: THREE.Object3D;
  release(): void;
}

export interface AssetPreviewCacheOptions {
  maxEntries?: number;
  maxBytes?: number;
  load?: typeof loadProjectionAssetPreview;
}

interface CachedPreview {
  key: string;
  pending: Promise<THREE.Object3D>;
  asset?: THREE.Object3D;
  bytes: number;
  leases: number;
  retired: boolean;
}

/** Approximate CPU plus GPU storage, including texture mipmaps, without uploading anything. */
export function estimatePreviewBytes(root: THREE.Object3D): number {
  const buffers = new Set<ArrayBufferLike>();
  const textures = new Set<THREE.Texture>();
  const sources = new Set<THREE.Texture["source"]>();
  let bytes = 0;
  const addAttribute = (attribute: THREE.BufferAttribute | THREE.InterleavedBufferAttribute) => {
    const array =
      attribute instanceof THREE.InterleavedBufferAttribute
        ? attribute.data.array
        : attribute.array;
    if (!buffers.has(array.buffer)) {
      buffers.add(array.buffer);
      bytes += array.buffer.byteLength * 2;
    }
  };
  root.traverse((node) => {
    const mesh = node as THREE.Mesh;
    if (mesh.geometry) {
      if (mesh.geometry.index) addAttribute(mesh.geometry.index);
      for (const attribute of Object.values(mesh.geometry.attributes)) addAttribute(attribute);
      for (const attributes of Object.values(mesh.geometry.morphAttributes))
        for (const attribute of attributes ?? []) addAttribute(attribute);
    }
    if (mesh.material)
      for (const material of Array.isArray(mesh.material) ? mesh.material : [mesh.material])
        for (const value of Object.values(material))
          if (value instanceof THREE.Texture) textures.add(value);
  });
  for (const texture of textures) {
    const images = Array.isArray(texture.source.data) ? texture.source.data : [texture.source.data];
    let decodedBytes = 0;
    for (const image of images) {
      if (!image) continue;
      decodedBytes += image.data?.byteLength ?? (image.width ?? 0) * (image.height ?? 0) * 4;
    }
    bytes += decodedBytes * (texture.generateMipmaps ? 4 / 3 : 1);
    if (!sources.has(texture.source)) {
      sources.add(texture.source);
      bytes += decodedBytes;
    }
  }
  return Math.ceil(bytes);
}

/** Bounded parsed-preview LRU. Active cards pin their resources until their lease is released. */
export class AssetPreviewCache {
  private readonly roots = new WeakMap<FileSystemDirectoryHandle, number>();
  private nextRoot = 0;
  private readonly entries = new Map<string, CachedPreview>();
  private readonly maxEntries: number;
  private readonly maxBytes: number;
  private readonly load: typeof loadProjectionAssetPreview;
  private bytes = 0;
  private disposed = false;

  constructor(options: AssetPreviewCacheOptions = {}) {
    this.maxEntries = options.maxEntries ?? 64;
    this.maxBytes = options.maxBytes ?? 128 * 1024 * 1024;
    this.load = options.load ?? loadProjectionAssetPreview;
    if (
      !Number.isInteger(this.maxEntries) ||
      this.maxEntries < 0 ||
      !Number.isFinite(this.maxBytes) ||
      this.maxBytes < 0
    )
      throw new Error("Invalid asset preview cache budget");
  }

  async acquire(
    root: FileSystemDirectoryHandle,
    entry: ProjectionAssetEntry,
  ): Promise<AssetPreviewLease> {
    if (this.disposed) throw new Error("Asset preview cache is disposed");
    let rootId = this.roots.get(root);
    if (rootId === undefined) this.roots.set(root, (rootId = ++this.nextRoot));
    const key = `${rootId}:${JSON.stringify(entry)}`;
    let record = this.entries.get(key);
    if (!record) {
      const created: CachedPreview = {
        key,
        pending: undefined!,
        bytes: 0,
        leases: 0,
        retired: false,
      };
      record = created;
      // Defer the loader so synchronous failures take the same cleanup path as rejected loads.
      created.pending = Promise.resolve()
        .then(() => this.load(root, entry))
        .then((asset) => {
          if (created.retired) {
            disposeObjectResources([asset]);
            throw new Error("Asset preview cache was cleared during loading");
          }
          created.asset = asset;
          created.bytes = estimatePreviewBytes(asset);
          this.bytes += created.bytes;
          this.trim();
          return asset;
        })
        .catch((error) => {
          this.retire(created);
          throw error;
        });
      this.entries.set(key, created);
    } else {
      this.entries.delete(key);
      this.entries.set(key, record);
    }
    record.leases++;
    let asset: THREE.Object3D;
    try {
      asset = (await record.pending).clone(true);
    } catch (error) {
      this.release(record);
      throw error;
    }
    let released = false;
    return {
      asset,
      release: () => {
        if (released) return;
        released = true;
        asset.removeFromParent();
        this.release(record);
      },
    };
  }

  /** Retire the current library generation; existing leases remain valid until released. */
  clear(): void {
    for (const record of this.entries.values()) this.retire(record);
  }

  dispose(): void {
    this.disposed = true;
    this.clear();
  }

  private release(record: CachedPreview): void {
    record.leases--;
    if (record.retired) {
      if (!record.leases && record.asset) {
        disposeObjectResources([record.asset]);
        record.asset = undefined;
      }
    } else {
      this.entries.delete(record.key);
      this.entries.set(record.key, record);
      this.trim();
    }
  }

  private retire(record: CachedPreview): void {
    if (record.retired) return;
    record.retired = true;
    this.entries.delete(record.key);
    this.bytes -= record.bytes;
    if (!record.leases && record.asset) {
      disposeObjectResources([record.asset]);
      record.asset = undefined;
    }
  }

  private trim(): void {
    for (const record of this.entries.values()) {
      if (this.entries.size <= this.maxEntries && this.bytes <= this.maxBytes) break;
      if (!record.leases) this.retire(record);
    }
  }
}
