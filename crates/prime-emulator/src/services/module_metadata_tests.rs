use super::*;
#[path = "../../tests/support/module_fixture.rs"]
mod fixture;
struct Bytes(Vec<u8>);
impl ReadAt for Bytes {
    fn len(&self) -> u32 {
        self.0.len() as u32
    }
    fn read(&mut self, at: u32, len: usize) -> Option<Vec<u8>> {
        Some(
            self.0
                .get(at as usize..(at as usize).checked_add(len)?)?
                .to_vec(),
        )
    }
}
#[test]
fn original_rom_records_match_rom_headers_and_pe_named_resource() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/module-metadata-controls.json"
    ))
    .unwrap();
    let mut fm = vec![0; 128];
    fm[..8].copy_from_slice(&[0x4d, 0x46, 0x34, 0x12, 0x78, 0x56, 0xbc, 0x9a]);
    let mut fi = fm.clone();
    fi[0] = 0x49;
    for (name, input) in [
        ("other", b"not FM/FI".to_vec()),
        ("FM", fm),
        ("FI", fi),
        ("PE", fixture::pe()),
    ] {
        let expected = reference["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["case"] == name)
            .unwrap()["bytes"]
            .as_str()
            .unwrap();
        let actual = metadata(&mut Bytes(input));
        let hex = actual
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(hex, expected[..OUTPUT_BYTES * 2], "{name}");
    }
}
#[test]
fn resource_names_bounds_machine_and_truncated_rom_headers_are_respected() {
    for (at, value) in [(514 + 0xb0, b'X'), (514 + 0xc0, b'X')] {
        let mut bytes = fixture::pe();
        bytes[at] = value;
        let output = metadata(&mut Bytes(bytes));
        assert_eq!(&output[..8], &[0; 8]);
        assert_eq!(word(&output, 8), Some(0x11223344));
        assert_eq!(word(&output, 12), Some(1024));
    }
    let mut wrong_machine = fixture::pe();
    wrong_machine[0x84..0x86].copy_from_slice(&0x14cu16.to_le_bytes());
    assert_eq!(metadata(&mut Bytes(wrong_machine)), [0; OUTPUT_BYTES]);
    let mut truncated = vec![0; 127];
    truncated[..2].copy_from_slice(b"MF");
    assert_eq!(metadata(&mut Bytes(truncated)), [0; OUTPUT_BYTES]);
    let mut outside = fixture::pe();
    outside[512 + 0x90..512 + 0x94].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(&metadata(&mut Bytes(outside))[..8], &[0; 8]);
}
