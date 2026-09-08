//! Fixed bytes emitted from the unchanged layouts at commit
//! 53e36bdc46542b51a5358674e663961de9f25d1b using bitcode 0.6.9.
//! Fixed bytes were generated independently with bitcode 0.6.9 from declarations
//! at pre-refactor commit 53e36bdc46542b51a5358674e663961de9f25d1b, not from the
//! decoder under test. The generator preserved transitive wire fields/variants
//! and omitted only serde-specific annotations. Assertions below document the
//! nonzero fixture values. Do not regenerate these bytes to accommodate a schema
//! edit; the full construction record remains in Git history at d4fc606a7,
//! docs/AUDIT2_PARITY_SCHEMA.md.
//! Do not regenerate these from the decoder under test.
use super::*;

const V66: &str = "04421466726f7a656e2d6265666f72652d617564697432066865616465720474657374047465737400f0debc9a7856341204100401000441010419011f6c6962635f72616e645f7261775f676c6f62616c5f647261775f6f72646572136f70617175655f69735f726561636861626c65011172657461696e65642d7636362d6f6e6c79010098badcfe010100000000000000040100000000000000000000040004000000000000000000010700011f00000000803f00000000c000000040400000008040023412f9ff010102045d6b00d2042e16000a726e675f7072656669780600030209005100d902000000";
const V67: &str = "04431466726f7a656e2d6265666f72652d617564697432066865616465720474657374047465737400f0debc9a7856341204100401000441010419011f6c6962635f72616e645f7261775f676c6f62616c5f647261775f6f72646572136f70617175655f69735f726561636861626c65010098badcfe010100000000000000040100000000000000000000040004000000000000000000010700011f00000000803f00000000c000000040400000008040023412f9ff0102045d6b00d2042e16000a726e675f7072656669780600030209005100d902000000";
const V67_LATE: &str = "04431466726f7a656e2d6265666f72652d617564697432066865616465720474657374047465737400f0debc9a7856341204100401000441010419011f6c6962635f72616e645f7261775f676c6f62616c5f647261775f6f72646572136f70617175655f69735f726561636861626c65010098badcfe010100000000000000040100000000000000000000040004000000000000000000010700011f00000000803f00000000c000000040400000008040023412f9ff010102045d6b00d2042e16000a726e675f7072656669780600030209005100d902000000";
const COMMANDS_V66: &str =
    "0515181706060004111111045d5d041818050573776f726473776f7264030200006040007856341201";

fn bytes(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0);
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("fixed fixture hex"))
        .collect()
}

fn framed(payload: &[u8]) -> std::io::Cursor<Vec<u8>> {
    let mut record = (payload.len() as u64).to_le_bytes().to_vec();
    record.extend_from_slice(payload);
    std::io::Cursor::new(record)
}

#[test]
fn historical_header_golden_bytes_decode_through_production_dispatch() {
    for (hex, version, late) in [(V66, 66, false), (V67, 67, false), (V67_LATE, 67, true)] {
        let (header, decoded_late) =
            read_binary_trace_header_record_with_layout(&mut framed(&bytes(hex)), version)
                .expect("fixed historical header must decode");
        assert_eq!(decoded_late, late);
        assert_eq!(header.version, version);
        assert_eq!(header.source_fingerprint, "frozen-before-audit2");
        let trace = header.trace;
        assert_eq!(trace.record_type, "header");
        assert_eq!(trace.rng_seed, 0x123456789abcdef0);
        assert_eq!(trace.initial_frame, 321);
        assert_eq!(trace.random_input_seed, Some(0xfedcba98));
        let transients = trace.initial_npc_transients.unwrap();
        assert_eq!(transients.len(), 2);
        assert_eq!(
            (
                transients[0].creation_order,
                transients[0].maximal_visibility
            ),
            (93, 1234)
        );
        assert_eq!(
            (
                transients[1].creation_order,
                transients[1].maximal_visibility
            ),
            (107, 5678)
        );
        let layer = &trace.motion_grid.layers[0];
        assert_eq!(layer.layer, 7);
        let line = &layer.lines[0];
        assert_eq!(line.index, 31);
        assert_eq!((line.a.x.value(), line.a.y.value()), (1.0, -2.0));
        assert_eq!((line.b.x.value(), line.b.y.value()), (3.0, 4.0));
        assert_eq!(
            (line.type_mask, line.associated_sector, line.active),
            (0x1234, -7, true)
        );
        assert_eq!(header.rng_prefix.draws.values, [9, 81, 729]);
    }
}

#[test]
fn historical_header_golden_bytes_preserve_frozen_layouts() {
    let payload = bytes(V66);
    let header: BinaryTraceHeaderV66 = bitcode::decode(&payload).unwrap();
    assert_eq!(
        header.trace.authoritative_state.as_deref(),
        Some("retained-v66-only")
    );
    assert_eq!(bitcode::encode(&header), payload);
    let payload = bytes(V67);
    let header: BinaryTraceHeaderV67 = bitcode::decode(&payload).unwrap();
    assert_eq!(bitcode::encode(&header), payload);
    let payload = bytes(V67_LATE);
    let header: BinaryTraceHeaderV67Late = bitcode::decode(&payload).unwrap();
    assert_eq!(bitcode::encode(&header), payload);
}

#[test]
fn historical_command_golden_preserves_variant_order_and_optional_seek() {
    let payload = bytes(COMMANDS_V66);
    let commands: Vec<v66::TraceCommandV66> = bitcode::decode(&payload).unwrap();
    assert_eq!(bitcode::encode(&commands), payload);
    let mut commands = commands.into_iter().map(v66::TraceCommandV66::into_current);
    assert!(matches!(commands.next(), Some(TraceCommand::SelectAllPcs)));
    assert!(matches!(
        commands.next(),
        Some(TraceCommand::SetLockAlt { on: true })
    ));
    assert!(matches!(
        commands.next(),
        Some(TraceCommand::SelectActionIndex { index: 0x12345678 })
    ));
    for expected_distance in [None, Some(3.5)] {
        let Some(TraceCommand::SwordStrike {
            actor,
            target,
            original_command,
            original_command_name,
            with_seek,
            seek_distance,
        }) = commands.next()
        else {
            panic!("golden sword command changed variant");
        };
        assert_eq!((actor.kind, actor.index), (TraceEntityKind::Pc, 17));
        assert_eq!((target.kind, target.index), (TraceEntityKind::Soldier, 93));
        assert_eq!(original_command, 24);
        assert_eq!(original_command_name, "sword");
        assert!(with_seek);
        match expected_distance {
            Some(value) => assert_eq!(seek_distance, value),
            None => assert!(seek_distance.is_nan()),
        }
    }
    assert!(commands.next().is_none());
}

#[test]
fn historical_header_golden_rejects_truncation_and_unknown_version() {
    for (hex, version) in [(V66, 66), (V67, 67), (V67_LATE, 67)] {
        let mut payload = bytes(hex);
        payload.pop();
        assert!(read_binary_trace_header_record(&mut framed(&payload), version).is_err());
    }
    assert!(read_binary_trace_header_record(&mut framed(&bytes(V67)), 65).is_err());
}
