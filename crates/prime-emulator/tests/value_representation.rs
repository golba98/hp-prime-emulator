//! Returned-object evidence is bounded and independent of physical hardware.
use prime_emulator::{memory::GuestMemory, runtime_value};

fn memory() -> GuestMemory {
    let mut mem = GuestMemory::new();
    mem.map("objects", 0x1000, 0x40000).unwrap();
    mem
}

fn list(mem: &GuestMemory, address: u32, pointers: &[u32]) {
    mem.write_u32(address, 0x2016_0001).unwrap();
    mem.write_u32(address + 4, pointers.len() as u32).unwrap();
    for (i, pointer) in pointers.iter().enumerate() {
        mem.write_u32(address + 8 + 4 * i as u32, *pointer).unwrap();
    }
}

fn rational(mem: &GuestMemory, address: u32, numerator: u64) {
    mem.write_u32(address, 0x010f_0001).unwrap();
    mem.write_u32(address + 4, 0).unwrap();
    mem.write_bytes(address + 8, &numerator.to_le_bytes())
        .unwrap();
    mem.write_bytes(address + 16, &1u64.to_le_bytes()).unwrap();
}

#[test]
fn rooted_capture_retains_nested_objects_and_exact_element_paths() {
    let mem = memory();
    list(&mem, 0x1000, &[0x1100, 0x1200]);
    list(&mem, 0x1100, &[0x1300]);
    rational(&mem, 0x1300, 1);
    mem.write_u32(0x1200, 0x0012_0001).unwrap();
    mem.write_u32(0x1204, 2).unwrap();
    mem.write_bytes(0x1208, &[0xe9, 0, 0xa3, 3, 0, 0]).unwrap();
    let evidence = runtime_value::decode_return(&mem, 0x1000);
    let graph = evidence.representation.unwrap();
    assert!(graph.coherent && graph.complete);
    assert_eq!(graph.captured_bytes, 66);
    assert_eq!(graph.nodes.len(), 4);
    assert_eq!(graph.nodes[0].children, [1, 3]);
    assert_eq!(graph.nodes[2].path, [1, 1]);
    assert_eq!(graph.nodes[3].path, [2]);
    assert_eq!(&graph.nodes[3].bytes[8..], &[0xe9, 0, 0xa3, 3, 0, 0]);
}

#[test]
fn unknown_and_cyclic_leaves_keep_partial_evidence_without_hiding_siblings() {
    let mem = memory();
    list(&mem, 0x1000, &[0x1000, 0x2000, 0x2100]);
    mem.write_u32(0x2000, 0x0009_0001).unwrap();
    rational(&mem, 0x2100, 7);
    let evidence = runtime_value::decode_return(&mem, 0x1000);
    assert!(evidence.value.is_none());
    let graph = evidence.representation.unwrap();
    assert!(!graph.complete);
    assert_eq!(graph.nodes.len(), 4);
    assert!(
        graph.nodes[1]
            .unavailable
            .as_ref()
            .unwrap()
            .contains("cyclic")
    );
    assert_eq!(graph.nodes[2].bytes, 0x0009_0001u32.to_le_bytes());
    assert_eq!(graph.nodes[2].path, [2]);
    assert_eq!(graph.nodes[3].bytes.len(), 24);
}

#[test]
fn shared_noncyclic_objects_are_captured_per_logical_occurrence() {
    let mem = memory();
    list(&mem, 0x1000, &[0x1100, 0x1100]);
    list(&mem, 0x1100, &[0x1200]);
    rational(&mem, 0x1200, 3);
    let graph = runtime_value::decode_return(&mem, 0x1000)
        .representation
        .unwrap();
    assert!(graph.complete);
    assert_eq!(graph.nodes.len(), 5);
    assert_eq!(graph.captured_bytes, 16 + 2 * (12 + 24));
    assert_eq!(graph.nodes[2].path, [1, 1]);
    assert_eq!(graph.nodes[4].path, [2, 1]);
}

#[test]
fn pointer_and_item_limits_fail_closed_with_the_readable_prefix() {
    let mem = memory();
    for address in [0, 0x1001, 0xffff_fffc] {
        let value = runtime_value::decode_return(&mem, address);
        assert!(value.value.is_none());
        assert!(!value.representation.unwrap().complete);
    }
    mem.write_u32(0x1000, 0x2016_0001).unwrap();
    mem.write_u32(0x1004, u32::MAX).unwrap();
    let graph = runtime_value::decode_return(&mem, 0x1000)
        .representation
        .unwrap();
    assert!(!graph.complete);
    assert_eq!(graph.nodes[0].bytes.len(), 8);
    assert!(
        graph.nodes[0]
            .unavailable
            .as_ref()
            .unwrap()
            .contains("item")
    );
}

#[test]
fn nesting_and_repeated_large_strings_are_bounded() {
    let mem = memory();
    for level in 0..9 {
        list(&mem, 0x1000 + level * 0x100, &[0x1100 + level * 0x100]);
    }
    let graph = runtime_value::decode_return(&mem, 0x1000)
        .representation
        .unwrap();
    assert!(!graph.complete);
    assert_eq!(
        graph.nodes.last().unwrap().path.len(),
        runtime_value::MAX_LIST_DEPTH
    );
    assert!(
        graph
            .nodes
            .last()
            .unwrap()
            .unavailable
            .as_ref()
            .unwrap()
            .contains("nesting")
    );

    list(&mem, 0x1000, &[0x10000; 16]);
    mem.write_u32(0x10000, 0x0012_0001).unwrap();
    mem.write_u32(0x10004, 65536).unwrap();
    let graph = runtime_value::decode_return(&mem, 0x1000)
        .representation
        .unwrap();
    assert!(!graph.complete);
    assert!(graph.captured_bytes <= runtime_value::MAX_VALUE_BYTES);
    assert!(
        graph
            .nodes
            .iter()
            .any(|n| n.unavailable.as_ref().is_some_and(|e| e.contains("1 MiB")))
    );
}

#[test]
fn busy_snapshot_cannot_produce_a_decoded_return() {
    let mem = memory();
    rational(&mem, 0x1000, 49);
    let _snapshot = mem.snapshot_gate.try_snapshot().unwrap();
    let value = runtime_value::decode_return(&mem, 0x1000);
    assert!(value.value.is_none());
    let graph = value.representation.unwrap();
    assert!(!graph.coherent && !graph.complete && graph.nodes.is_empty());
}

#[test]
fn fresh_firmware_captures_replay_to_exact_typed_values() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/runtime-values/list-captures-v1.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let graph = serde_json::from_value(case["representation"].clone()).unwrap();
        let evidence = runtime_value::decode_representation(graph);
        assert!(
            evidence.unavailable.is_none(),
            "{}: {:?}",
            case["id"],
            evidence.unavailable
        );
        assert_eq!(
            serde_json::to_value(evidence.value).unwrap(),
            case["expected"],
            "{}",
            case["id"]
        );
    }
}

#[test]
fn managed_lists_ignore_only_the_unused_byte_and_keep_payload_admission() {
    let mem = memory();
    rational(&mem, 0x1100, 7);
    for unused in 0..=255u32 {
        list(&mem, 0x1000, &[0x1100]);
        mem.write_u32(0x1000, (unused << 24) | 0x0016_0001).unwrap();
        let evidence = runtime_value::decode_return(&mem, 0x1000);
        assert_eq!(evidence.value.as_ref().unwrap().display(), "{7}");
        let graph = evidence.representation.unwrap();
        assert!(graph.complete && graph.coherent);
        assert_eq!(
            runtime_value::decode_representation(graph).value,
            evidence.value
        );
        mem.write_u32(0x1008, 0xffff_fffc).unwrap();
        assert!(runtime_value::decode_return(&mem, 0x1000).value.is_none());
        mem.write_u32(0x1004, u32::MAX).unwrap();
        assert!(runtime_value::decode_return(&mem, 0x1000).value.is_none());
    }
}

#[test]
fn unseen_subtypes_and_unknown_children_cannot_become_partial_successes() {
    let mem = memory();
    rational(&mem, 0x1100, 1);
    list(&mem, 0x1000, &[0x1100]);
    // Managed tag16 has no byte3 subtype. An unestablished tag26/header
    // combination must still fail closed.
    mem.write_u32(0x1000, 0x1026_0001).unwrap();
    let value = runtime_value::decode_return(&mem, 0x1000);
    assert!(value.value.is_none());
    assert!(value.unavailable.unwrap().contains("0x10"));
    list(&mem, 0x1000, &[0x1100, 0x1200]);
    mem.write_u32(0x1200, 0x0009_0001).unwrap();
    let value = runtime_value::decode_return(&mem, 0x1000);
    assert!(value.value.is_none());
    assert!(value.unavailable.unwrap().contains("return[2]"));
    assert_eq!(value.representation.unwrap().nodes.len(), 3);
}

#[test]
fn shared_strings_charge_decoded_output_as_well_as_captured_bytes() {
    let mem = memory();
    list(&mem, 0x1000, &[0x10000; 200]);
    mem.write_u32(0x10000, 0x0012_0001).unwrap();
    mem.write_u32(0x10004, 1000).unwrap();
    let units: Vec<u8> = std::iter::repeat_n([1, 0], 1000)
        .flatten()
        .chain([0, 0])
        .collect();
    mem.write_bytes(0x10008, &units).unwrap();
    let value = runtime_value::decode_return(&mem, 0x1000);
    assert!(value.representation.as_ref().unwrap().complete);
    assert!(value.representation.as_ref().unwrap().captured_bytes < runtime_value::MAX_VALUE_BYTES);
    assert!(value.value.is_none());
    assert!(value.unavailable.unwrap().contains("decoded output"));
}

#[test]
fn declared_item_limit_and_mapped_rom_elements_are_supported() {
    let mut mem = memory();
    mem.map("firmware constants", 0x3060_0000, 0x1000).unwrap();
    rational(&mem, 0x3060_0000, 7);
    list(
        &mem,
        0x1000,
        &vec![0x3060_0000; runtime_value::MAX_LIST_ITEMS],
    );
    let value = runtime_value::decode_return(&mem, 0x1000);
    let decoded = serde_json::to_value(value.value).unwrap();
    assert_eq!(decoded["type"], "list");
    assert_eq!(
        decoded["value"].as_array().unwrap().len(),
        runtime_value::MAX_LIST_ITEMS
    );
    assert_eq!(
        decoded["value"][4095],
        serde_json::json!({"type":"real","value":"7"})
    );
}

#[test]
fn utf16_strings_keep_isolated_units_nul_and_valid_pairs_but_check_the_terminator() {
    let mem = memory();
    let address = 0x1000;
    for (units, expected) in [
        (
            vec![0xd800],
            serde_json::json!({"type":"utf16","value":[55296]}),
        ),
        (
            vec![0xdc00],
            serde_json::json!({"type":"utf16","value":[56320]}),
        ),
        (
            vec![65, 0, 66],
            serde_json::json!({"type":"string","value":"A\u{0}B"}),
        ),
        (
            vec![0xd83d, 0xde00],
            serde_json::json!({"type":"string","value":"😀"}),
        ),
    ] {
        mem.write_u32(address, 0x0012_0001).unwrap();
        mem.write_u32(address + 4, units.len() as u32).unwrap();
        let bytes: Vec<u8> = units
            .iter()
            .copied()
            .chain([0u16])
            .flat_map(u16::to_le_bytes)
            .collect();
        mem.write_bytes(address + 8, &bytes).unwrap();
        let evidence = runtime_value::decode_return(&mem, address);
        assert!(evidence.unavailable.is_none(), "{:?}", evidence.unavailable);
        assert_eq!(serde_json::to_value(evidence.value).unwrap(), expected);
        mem.write_bytes(address + 8 + units.len() as u32 * 2, &[1, 0])
            .unwrap();
        assert!(
            runtime_value::decode_return(&mem, address)
                .unavailable
                .unwrap()
                .contains("terminator")
        );
    }
}

#[test]
fn repeated_raw_utf16_strings_share_the_existing_capture_and_decoded_output_budget() {
    let mem = memory();
    list(&mem, 0x1000, &[0x10000; 200]);
    mem.write_u32(0x10000, 0x0012_0001).unwrap();
    mem.write_u32(0x10004, 1000).unwrap();
    mem.write_bytes(
        0x10008,
        &std::iter::repeat_n([0, 0xd8], 1000)
            .flatten()
            .chain([0, 0])
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let evidence = runtime_value::decode_return(&mem, 0x1000);
    assert!(evidence.representation.unwrap().complete);
    assert!(evidence.value.is_none());
    assert!(evidence.unavailable.unwrap().contains("decoded output"));
}

#[test]
fn signed_width_carriers_admit_only_coherent_positive_or_sign_extended_values() {
    // Constructed ABI-boundary inputs, independent of the actual firmware test.
    let mem = memory();
    for (raw, width, tag) in [
        (255u64, -8i8, 0x91u8),
        (u64::MAX, -8, 0x11),
        (u64::MAX - 255, -8, 0x11),
        (u64::MAX, -63, 0x11),
        (u64::MAX, 64, 0x91),
    ] {
        mem.write_u32(
            0x1000,
            ((width as u8 as u32) << 24) | ((tag as u32) << 16) | 1,
        )
        .unwrap();
        mem.write_u32(0x1004, 0xaabb_ccdd).unwrap();
        mem.write_bytes(0x1008, &raw.to_le_bytes()).unwrap();
        let decoded = runtime_value::decode_return(&mem, 0x1000);
        assert!(
            decoded.unavailable.is_none(),
            "{raw}/{width}: {:?}",
            decoded.unavailable
        );
        let encoded = serde_json::to_value(decoded.value.unwrap()).unwrap();
        assert_eq!(encoded["bits"], width.unsigned_abs());
        if width < 0 {
            assert_eq!(encoded["signed_word"], true);
            assert_eq!(
                encoded["numeric"],
                if raw == 255 {
                    "255".to_owned()
                } else {
                    (raw as i64).to_string()
                }
            );
        }
    }
    for (raw, width) in [
        (256u64, -8i8),
        (0xffff_ffff_0000_0001, -8),
        (0, -64),
        (0, 0),
    ] {
        mem.write_u32(0x1000, ((width as u8 as u32) << 24) | 0x0011_0001)
            .unwrap();
        mem.write_bytes(0x1008, &raw.to_le_bytes()).unwrap();
        assert!(runtime_value::decode_return(&mem, 0x1000).value.is_none());
    }
}

#[test]
fn observed_list_flag30_keeps_tag_pointer_cycle_and_allocation_admission() {
    let mut mem = GuestMemory::new();
    mem.map("heap", 0x10000, 0x10000).unwrap();
    mem.set_heap(0x10000, 0x10000);
    let root = mem.heap_alloc(12).unwrap();
    let child = mem.heap_alloc(24).unwrap();
    rational(&mem, child, 7);
    mem.write_u32(root, 0x3016_0001).unwrap();
    mem.write_u32(root + 4, 1).unwrap();
    mem.write_u32(root + 8, child).unwrap();
    let valid = runtime_value::decode_return(&mem, root);
    assert!(valid.unavailable.is_none());
    assert_eq!(
        serde_json::to_value(valid.value.unwrap()).unwrap(),
        serde_json::json!({"type":"list","value":[{"type":"real","value":"7"}]})
    );
    mem.write_u32(root, 0x3036_0001).unwrap();
    let wrong_tag = runtime_value::decode_return(&mem, root);
    assert!(wrong_tag.value.is_none());
    assert_eq!(wrong_tag.representation.unwrap().nodes.len(), 1);
    mem.write_u32(root, 0x3016_0001).unwrap();
    mem.write_u32(root + 8, root).unwrap();
    assert!(runtime_value::decode_return(&mem, root).value.is_none());
    mem.write_u32(root + 8, child).unwrap();
    mem.write_u32(root + 4, 8).unwrap();
    let oversized = runtime_value::decode_return(&mem, root);
    assert!(oversized.value.is_none());
    assert!(oversized.unavailable.unwrap().contains("allocation"));
    assert_eq!(oversized.representation.unwrap().nodes.len(), 1);
}
