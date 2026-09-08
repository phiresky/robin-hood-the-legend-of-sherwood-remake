// Minimal modules for artifact verifier tests; never packaged as game artifacts.
export function admissionFixture({ max = 6144, shared = false, imported = false, exported = true } = {}) {
    const leb = value => { const result = []; do { const byte = value & 127; value >>>= 7; result.push(byte | (value ? 128 : 0)); } while (value); return result; };
    const section = (id, data) => [id, ...leb(data.length), ...data];
    const memory = [max === null ? 0 : shared ? 3 : 1, 0, ...(max === null ? [] : leb(max))];
    const name = Buffer.from('validate_compact_replay');
    return Buffer.from([
        0, 97, 115, 109, 1, 0, 0, 0,
        ...section(1, [1, 96, 0, 0]),
        ...(imported ? section(2, [1, 1, 109, 1, 109, 2, ...memory]) : []),
        ...section(3, [1, 0]),
        ...(!imported ? section(5, [1, ...memory]) : []),
        ...(exported ? section(7, [1, name.length, ...name, 0, 0]) : []),
        ...section(10, [1, 2, 0, 11]),
    ]);
}

