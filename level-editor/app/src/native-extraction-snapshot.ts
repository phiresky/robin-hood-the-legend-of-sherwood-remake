import * as THREE from "three";

type Attribute = THREE.BufferAttribute | THREE.InterleavedBufferAttribute;

/** One independent backing copy per session, shared only by its private geometry snapshots. */
export function createNativeExtractionGeometrySnapshot() {
  const buffers = new WeakMap<ArrayBufferLike, ArrayBuffer>();
  const attributes = new WeakMap<Attribute, Attribute>();
  const interleaved = new WeakMap<THREE.InterleavedBuffer, THREE.InterleavedBuffer>();
  const array = (source: THREE.TypedArray): THREE.TypedArray => {
    let backing = buffers.get(source.buffer);
    if (!backing) {
      backing = new Uint8Array(source.buffer).slice().buffer;
      buffers.set(source.buffer, backing);
    }
    const Constructor = source.constructor as new (
      buffer: ArrayBuffer,
      byteOffset: number,
      length: number,
    ) => THREE.TypedArray;
    return new Constructor(backing, source.byteOffset, source.length);
  };
  const attribute = (source: Attribute): Attribute => {
    const existing = attributes.get(source);
    if (existing) return existing;
    let copy: Attribute;
    if (source instanceof THREE.InterleavedBufferAttribute) {
      if (source.data instanceof THREE.InstancedInterleavedBuffer)
        throw new Error("Extraction snapshot does not support instanced attributes");
      let data = interleaved.get(source.data);
      if (!data) {
        data = new THREE.InterleavedBuffer(array(source.data.array), source.data.stride);
        data.setUsage(source.data.usage);
        interleaved.set(source.data, data);
      }
      copy = new THREE.InterleavedBufferAttribute(
        data,
        source.itemSize,
        source.offset,
        source.normalized,
      );
    } else {
      if (source instanceof THREE.InstancedBufferAttribute)
        throw new Error("Extraction snapshot does not support instanced attributes");
      copy = new THREE.BufferAttribute(array(source.array), source.itemSize, source.normalized);
      if (source instanceof THREE.Float16BufferAttribute) {
        Object.setPrototypeOf(copy, THREE.Float16BufferAttribute.prototype);
        Object.defineProperty(copy, "isFloat16BufferAttribute", { value: true });
      }
      copy.setUsage(source.usage);
      copy.gpuType = source.gpuType;
    }
    copy.name = source.name;
    attributes.set(source, copy);
    return copy;
  };
  return (source: THREE.BufferGeometry): THREE.BufferGeometry => {
    const copy = new THREE.BufferGeometry();
    copy.name = source.name;
    if (source.index) copy.setIndex(attribute(source.index) as THREE.BufferAttribute);
    for (const [name, value] of Object.entries(source.attributes))
      copy.setAttribute(name, attribute(value));
    for (const [name, values] of Object.entries(source.morphAttributes))
      copy.morphAttributes[name as keyof typeof copy.morphAttributes] = values!.map(attribute);
    copy.morphTargetsRelative = source.morphTargetsRelative;
    for (const group of source.groups) copy.addGroup(group.start, group.count, group.materialIndex);
    copy.boundingBox = source.boundingBox?.clone() ?? null;
    copy.boundingSphere = source.boundingSphere?.clone() ?? null;
    copy.setDrawRange(source.drawRange.start, source.drawRange.count);
    copy.userData = structuredClone(source.userData);
    return copy;
  };
}
