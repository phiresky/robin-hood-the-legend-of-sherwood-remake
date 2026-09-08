/** Check the published bytes, including retained-FD corpora, without instantiating. */
export function verifyReplayAdmissionWasm(bytes) {
    const module = new WebAssembly.Module(bytes);
    if (WebAssembly.Module.imports(module).some(entry => entry.kind === 'memory')) {
        throw new Error('replay admission must not import memory');
    }
    if (!WebAssembly.Module.exports(module).some(entry => entry.name === 'validate_compact_replay' && entry.kind === 'function')) {
        throw new Error('replay admission is missing validate_compact_replay');
    }
    let offset = 8;
    const u32 = () => {
        let value = 0;
        for (let shift = 0; shift < 35; shift += 7) {
            if (offset >= bytes.length) throw new Error('truncated replay admission memory declaration');
            const byte = bytes[offset++];
            if (shift === 28 && byte > 15) throw new Error('invalid replay admission memory declaration');
            value += (byte & 127) * 2 ** shift;
            if ((byte & 128) === 0) return value;
        }
        throw new Error('invalid replay admission memory declaration');
    };
    let found = false;
    while (offset < bytes.length) {
        const section = bytes[offset++];
        const length = u32();
        const end = offset + length;
        if (end > bytes.length) throw new Error('truncated replay admission section');
        if (section === 5) {
            // Flags=1 means a 32-bit non-shared memory with an explicit maximum.
            if (u32() !== 1 || u32() !== 1 || u32() > 6144 || u32() !== 6144 || offset !== end) {
                throw new Error('replay admission must own one non-shared memory capped at 384 MiB');
            }
            found = true;
        }
        offset = end;
    }
    if (!found) throw new Error('replay admission must declare its own capped memory');
}
