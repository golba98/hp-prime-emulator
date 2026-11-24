//! Complete independent original-firmware frames through actual PPL execution.
mod common;
use common::Run;
use sha2::{Digest, Sha256};

#[test]
fn installed_menu_resources_and_ppl_labels_match_complete_original_frames() {
    let Some(resource) = std::env::var_os("PRIME_TEST_FONT_RESOURCE") else {
        eprintln!("Complete menu controls require PRIME_TEST_FONT_RESOURCE");
        return;
    };
    let fonts =
        ppl_graphics::text::prime::PrimeFonts::load_resource(std::path::Path::new(&resource))
            .unwrap();
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-menu-controls.json"
    ))
    .unwrap();
    let evidence = std::env::var_os("PRIME_TEST_MENU_EVIDENCE").map(std::path::PathBuf::from);
    if let Some(root) = &evidence {
        std::fs::create_dir_all(root).unwrap();
    }
    for case in controls["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source.as_bytes())),
            case["source_sha256"]
        );
        let mut run = Run::new(case["name"].as_str().unwrap(), source);
        run.host
            .borrow_mut()
            .keys
            .push_back(ppl_vm::ScriptEvent::Key(30));
        let value =
            ppl_graphics::text::with_prime_fonts(fonts.clone(), || run.call("MENUREF", &[]))
                .unwrap()
                .unwrap();
        assert_eq!(common::text(&value), "{1,0}");
        let screen = run.machine.screen();
        if let Some(root) = &evidence {
            screen
                .save_png(&root.join(format!("{}.png", case["name"].as_str().unwrap())))
                .unwrap();
        }
        let rgb = screen
            .pixels()
            .iter()
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
            .collect::<Vec<_>>();
        assert_eq!(
            format!("{:x}", Sha256::digest(rgb)),
            case["rgb_sha256"],
            "{}",
            case["name"]
        );
    }
}

#[test]
fn malformed_selected_menu_assets_fail_before_modifying_the_display() {
    let Some(resource) = std::env::var_os("PRIME_TEST_FONT_RESOURCE") else {
        return;
    };
    let original = std::fs::read(resource).unwrap();
    let png_start =
        28 + u32::from_le_bytes(original[28 + 6 * 8..28 + 6 * 8 + 4].try_into().unwrap()) as usize;
    for (offset, bytes) in [
        (28 + 6 * 8, u32::MAX.to_le_bytes()),
        (28 + 6 * 8 + 4, 0u32.to_le_bytes()),
        (png_start + 16, u32::MAX.to_be_bytes()),
    ] {
        let mut data = original.clone();
        data[offset..offset + 4].copy_from_slice(&bytes);
        let fonts = ppl_graphics::text::prime::PrimeFonts::from_resource(&data).unwrap();
        let mut screen = ppl_graphics::Grob::new(320, 240, 0x123456);
        let before = screen.pixels().to_vec();
        let result = ppl_graphics::text::with_prime_fonts(fonts, || {
            ppl_graphics::softkeys::draw(&mut screen, &["ABC"])
        })
        .unwrap();
        assert!(result.is_err());
        assert_eq!(screen.pixels(), before);
    }
}
