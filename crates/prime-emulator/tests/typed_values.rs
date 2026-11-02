//! Retained real firmware observations, plus deliberately malformed codec inputs.
use prime_emulator::{memory::GuestMemory, runtime_value};
fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/runtime-values/m17-typed-captures-v1.json"
    ))
    .unwrap()
}
#[test]
fn actual_firmware_mixed_types_replay_exactly() {
    let fixture = fixture();
    assert_eq!(fixture["origin"], "local-firmware");
    assert_eq!(fixture["physical_verification"], false);
    for case in fixture["cases"].as_array().unwrap() {
        let graph = serde_json::from_value(case["representation"].clone()).unwrap();
        let evidence = runtime_value::decode_representation(graph);
        assert!(
            evidence.unavailable.is_none(),
            "{} {:?}",
            case["id"],
            evidence.unavailable
        );
        assert_eq!(
            serde_json::to_value(evidence.value).unwrap(),
            case["expected"]
        );
    }
}
fn invalid(mut change: impl FnMut(&mut runtime_value::RepresentationGraph)) {
    let fixture = fixture();
    let mut graph = serde_json::from_value(fixture["cases"][0]["representation"].clone()).unwrap();
    change(&mut graph);
    let result = runtime_value::decode_representation(graph);
    assert!(
        result.value.is_none(),
        "malformed representation decoded as {:?}",
        result.value
    );
    assert!(result.unavailable.is_some());
}
#[test]
fn matrix_layout_numeric_flags_and_shared_item_budget_fail_closed() {
    // The first matrix is node 5 in this captured graph: find it by its path.
    for bad in [0, 3, u32::MAX] {
        invalid(|g| {
            let m = g.nodes.iter_mut().find(|n| n.path == [2]).unwrap();
            m.bytes[4..8].copy_from_slice(&bad.to_le_bytes());
        });
    }
    invalid(|g| {
        let m = g.nodes.iter_mut().find(|n| n.path == [2]).unwrap();
        m.bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    });
    invalid(|g| {
        let m = g.nodes.iter_mut().find(|n| n.path == [2]).unwrap();
        m.bytes[23] = 0xa1;
    });
    invalid(|g| {
        let m = g.nodes.iter_mut().find(|n| n.path == [2]).unwrap();
        m.bytes[22] = 0xfa;
    });
    invalid(|g| g.logical_items -= 1);
    invalid(|g| {
        g.nodes
            .iter_mut()
            .find(|n| n.path == [2])
            .unwrap()
            .children
            .push(0)
    });
    invalid(|g| {
        g.nodes
            .iter_mut()
            .find(|n| n.path == [2])
            .unwrap()
            .bytes
            .pop();
    });
}
#[test]
fn complex_components_and_integer_metadata_are_checked() {
    invalid(|g| g.nodes.iter_mut().find(|n| n.path == [4]).unwrap().bytes[18] = 2);
    for width in [0, 65, 255] {
        invalid(|g| g.nodes.iter_mut().find(|n| n.path == [5]).unwrap().bytes[3] = width);
    }
    invalid(|g| g.nodes.iter_mut().find(|n| n.path == [5]).unwrap().bytes[9] = 1);
    invalid(|g| g.nodes.iter_mut().find(|n| n.path == [5]).unwrap().bytes[2] = 0xb1);
}
#[test]
fn unknown_objects_capture_only_bounded_exact_allocation_without_following_words() {
    let mut mem = GuestMemory::new();
    mem.map("heap", 0x10000, 0x10000).unwrap();
    mem.set_heap(0x10000, 0x10000);
    let address = mem.heap_alloc(8192).unwrap();
    mem.write_u32(address, 0x0009_0001).unwrap();
    mem.write_u32(address + 4, 0xdead_beec).unwrap();
    let evidence = runtime_value::decode_return(&mem, address);
    let graph = evidence.representation.unwrap();
    assert!(evidence.value.is_none());
    assert!(!graph.complete);
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.nodes[0].bytes.len(), 4096);
    assert_eq!(graph.captured_bytes, 4096);
}

#[test]
fn known_matrix_allocation_cannot_read_neighbor_objects() {
    let mut mem = GuestMemory::new();
    mem.map("heap", 0x10000, 0x10000).unwrap();
    mem.set_heap(0x10000, 0x10000);
    let matrix = mem.heap_alloc(16).unwrap();
    let neighbor = mem.heap_alloc(64).unwrap();
    assert_eq!(neighbor, matrix + 16);
    mem.write_u32(matrix, 0x0014_0001).unwrap();
    mem.write_u32(matrix + 4, 2).unwrap();
    mem.write_u32(matrix + 8, 2).unwrap();
    mem.write_u32(matrix + 12, 2).unwrap();
    let evidence = runtime_value::decode_return(&mem, matrix);
    assert!(
        evidence.value.is_none(),
        "neighbor bytes became a fabricated matrix"
    );
    assert!(
        evidence
            .unavailable
            .as_deref()
            .unwrap()
            .contains("allocation")
    );
    assert_eq!(evidence.representation.unwrap().nodes[0].bytes.len(), 16);
}

#[test]
fn mapped_matrix_constants_without_heap_allocation_remain_supported() {
    let mut mem = GuestMemory::new();
    mem.map("firmware constants", 0x3060_0000, 4096).unwrap();
    let matrix = 0x3060_0000;
    mem.write_u32(matrix, 0x0014_0001).unwrap();
    mem.write_u32(matrix + 4, 2).unwrap();
    mem.write_u32(matrix + 8, 1).unwrap();
    mem.write_u32(matrix + 12, 1).unwrap();
    let evidence = runtime_value::decode_return(&mem, matrix);
    assert_eq!(
        serde_json::to_value(evidence.value).unwrap(),
        serde_json::json!({"type":"matrix","rows":1,"columns":1,"vector":false,"value":[{"type":"real","value":"0"}]})
    );
}
