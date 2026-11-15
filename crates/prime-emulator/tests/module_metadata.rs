//! Real SVC metadata/resource calls; self-authored inputs and original-ROM controls.
use prime_emulator::{
    engine::CpuThread,
    memory::GuestMemory,
    services::{Registry, SharedState, UnsupportedPolicy, handlers, names},
    trace::{TraceConfig, TraceHub},
};
use std::sync::Arc;
use unicorn_engine::RegisterARM as R;
#[path = "support/module_fixture.rs"]
mod fixture;
const CODE: u32 = 0x10000000;
const PATH: u32 = CODE + 0x1000;
const OUT: u32 = CODE + 0x2000;
struct Guest {
    cpu: CpuThread,
    mem: Arc<GuestMemory>,
    shared: Arc<SharedState>,
}
impl Guest {
    fn new(root: &std::path::Path) -> Self {
        let mut mem = GuestMemory::new();
        mem.map("code-data", CODE, 65536).unwrap();
        mem.map("heap", 0x20000000, 65536).unwrap();
        mem.map("fixed-dll", 0x21000000, 65536).unwrap();
        mem.set_heap(0x20000000, 65536);
        let mem = Arc::new(mem);
        let shared = Arc::new(SharedState::new(root.into(), "A:\\test.bin".into()));
        let mut registry = Registry::new();
        handlers::install(&mut registry);
        let cpu = CpuThread::new(
            mem.clone(),
            Arc::new(registry),
            shared.clone(),
            Arc::new(TraceHub::new(TraceConfig::default()).unwrap()),
            UnsupportedPolicy::Stop,
            0,
            vec![],
        )
        .unwrap();
        Self { cpu, mem, shared }
    }
    fn call(&mut self, name: &str, args: [u32; 4]) -> u32 {
        let id = names::service_id_by_name(name).unwrap();
        let bytes = [
            0xe92d0001u32,
            0xe92d4000,
            0xef000000 | id,
            0xe8bd8000,
            0xeafffffe,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
        self.mem.write_bytes(CODE, &bytes).unwrap();
        self.cpu.set_entry(CODE, CODE + 0x8000, args[0]).unwrap();
        for (reg, value) in [(R::R1, args[1]), (R::R2, args[2]), (R::R3, args[3])] {
            self.cpu.set_reg(reg, value).unwrap();
        }
        self.cpu.set_lr(CODE + 16).unwrap();
        self.cpu.run_slice(64).unwrap();
        self.cpu.reg(R::R0)
    }
    fn query(&mut self, name: &str, wide: bool) -> (u32, Vec<u8>) {
        if wide {
            let bytes = name
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            self.mem.write_bytes(PATH, &bytes).unwrap();
        } else {
            self.mem.write_cstr(PATH, name).unwrap();
        }
        self.mem.write_bytes(OUT, &[0xa5; 136]).unwrap();
        let ret = self.call(
            if wide {
                "GetApplicationHeadInfoW"
            } else {
                "GetApplicationHeadInfoA"
            },
            [PATH, OUT, 0, 0],
        );
        (ret, self.mem.read_bytes_alloc(OUT, 136).unwrap())
    }
}
fn cases() -> Vec<(&'static str, Vec<u8>)> {
    let mut fm = vec![0; 128];
    fm[..8].copy_from_slice(&[0x4d, 0x46, 0x34, 0x12, 0x78, 0x56, 0xbc, 0x9a]);
    let mut fi = fm.clone();
    fi[0] = 0x49;
    let mut elf = vec![0; 52];
    elf[..4].copy_from_slice(b"\x7fELF");
    vec![
        ("other", b"not FM/FI".to_vec()),
        ("FM", fm),
        ("FI", fi),
        ("PE", fixture::pe()),
        ("ELF", elf),
    ]
}
#[test]
fn ansi_wide_metadata_and_legacy_resources_execute_through_arm_svc() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("C")).unwrap();
    let mut guest = Guest::new(root.path());
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/module-metadata-controls.json")).unwrap();
    let heap = guest.mem.heap_stats();
    let files = guest.shared.vfs.inspection();
    let (ret, output) = guest.query("C:\\missing", false);
    assert_eq!(ret, u32::MAX);
    assert_eq!(output, [0xa5; 136]);
    for (case, bytes) in cases() {
        for wide in [false, true] {
            let filename = if wide {
                format!("λ{case}")
            } else {
                case.into()
            };
            std::fs::write(root.path().join("C").join(&filename), &bytes).unwrap();
            let (ret, output) = guest.query(&format!("C:\\{filename}"), wide);
            assert_eq!(ret, 0);
            let expected = reference["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["case"] == case)
                .unwrap()["bytes"]
                .as_str()
                .unwrap();
            let actual = output
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            assert_eq!(actual, expected, "{case}, wide={wide}");
        }
    }
    for name in [
        "_FindResourceA",
        "_FindResourceExA",
        "_FindResourceW",
        "_FindResourceExW",
        "GetResourceCfg",
    ] {
        guest.mem.write_bytes(OUT, &[0xa5; 136]).unwrap();
        assert_eq!(
            guest.call(name, [0xfffffff1, OUT, 0xfffffff3, 0xfffffff4]),
            0
        );
        assert_eq!(guest.mem.read_bytes_alloc(OUT, 136).unwrap(), [0xa5; 136]);
    }
    assert_eq!(guest.mem.heap_stats(), heap);
    assert_eq!(guest.shared.vfs.inspection(), files);
}

#[cfg(test)]
#[path = "support/metadata_rom.rs"]
mod metadata_rom;
#[test]
fn original_rom_module_parsers_and_hle_return_identical_complete_records() {
    let Some(rom_path) = std::env::var_os("PRIME_TEST_OS_ROM") else {
        return;
    };
    let rom = std::fs::read(rom_path).unwrap();
    assert_eq!(
        prime_emulator::sha256::hex(&rom),
        "f6aeba161189a5aa1acb57be428dc5730cfa5ff7cd534494e59225debd5a8999"
    );
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("C")).unwrap();
    let mut guest = Guest::new(root.path());
    for (label, payload, found) in std::iter::once(("missing", vec![], false))
        .chain(cases().into_iter().map(|(name, bytes)| (name, bytes, true)))
    {
        if found {
            std::fs::write(root.path().join("C/test.bin"), &payload).unwrap();
        }
        for wide in [false, true] {
            let actual = metadata_rom::invoke(&rom, &payload, found, wide);
            let hle = guest.query("C:\\test.bin", wide);
            assert_eq!(
                hle, actual,
                "original module callbacks: {label},wide={wide}"
            );
        }
    }
    metadata_rom::legacy_resources(&rom);
}

fn importing_pe() -> Vec<u8> {
    let mut bytes = fixture::pe();
    fn word(bytes: &mut [u8], at: usize, v: u32) {
        bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    word(&mut bytes, 0x98 + 28, 0x21000000);
    word(&mut bytes, 0x98 + 104, 0x1180);
    word(&mut bytes, 0x98 + 108, 40);
    word(&mut bytes, 896 + 12, 0x11b0);
    word(&mut bytes, 896 + 16, 0x11c0);
    bytes[944..953].copy_from_slice(b"LATE.DLL\0");
    word(&mut bytes, 960, 0x11d0);
    bytes[978..993].copy_from_slice(b"ActualFunction\0");
    bytes
}
#[test]
fn unresolved_pe_import_stops_with_its_symbol_instead_of_returning_zero() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("C")).unwrap();
    std::fs::write(root.path().join("C/importer.dll"), importing_pe()).unwrap();
    let mut guest = Guest::new(root.path());
    guest.mem.write_cstr(PATH, "C:\\importer.dll").unwrap();
    let base = guest.call("_LoadLibraryA", [PATH, 0, 0, 0]);
    assert_ne!(base, 0);
    let trap = guest.mem.read_u32(base + 0x11c0).unwrap();
    assert_eq!(
        guest.mem.read_u32(trap).unwrap(),
        prime_emulator::services::pe::IMPORT_TRAP
    );
    guest.cpu.set_entry(trap, CODE + 0x8000, 99).unwrap();
    guest.cpu.set_lr(CODE + 16).unwrap();
    assert!(matches!(
        guest.cpu.run_slice(64).unwrap(),
        prime_emulator::engine::RunSliceOutcome::Stopped(prime_emulator::engine::StopReason::Fault)
    ));
    let fault = guest.cpu.fault_report().unwrap();
    assert!(
        fault.kind.contains("LATE.DLL!ActualFunction"),
        "{}",
        fault.kind
    );
    assert!(fault.kind.contains("importer.dll"));
    assert_eq!(
        guest.cpu.reg(R::R0),
        99,
        "a missing function must not return fabricated zero"
    );
}
#[test]
fn a_late_loaded_dependency_rebinds_the_real_export_and_preserves_call_arguments() {
    use std::collections::HashMap;
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("C")).unwrap();
    std::fs::write(root.path().join("C/importer.dll"), importing_pe()).unwrap();
    let mut guest = Guest::new(root.path());
    guest.mem.write_cstr(PATH, "C:\\importer.dll").unwrap();
    let base = guest.call("_LoadLibraryA", [PATH, 0, 0, 0]);
    assert_ne!(base, 0);
    let trap = guest.mem.read_u32(base + 0x11c0).unwrap();
    let target = CODE + 0x400;
    guest
        .mem
        .write_bytes(
            target,
            &[0xe2800007u32, 0xe12fff1e]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    guest
        .shared
        .add_pe_module(prime_emulator::services::pe::PeModule {
            name: "late.dll".into(),
            guest_path: "C:\\late.dll".into(),
            base: target,
            size: 8,
            entry_rva: 0,
            exports_by_name: HashMap::from([("ActualFunction".into(), target)]),
            exports_by_ordinal: HashMap::new(),
            unresolved_imports: HashMap::new(),
        });
    guest.cpu.set_entry(trap, CODE + 0x8000, 35).unwrap();
    guest.cpu.set_lr(CODE + 16).unwrap();
    assert!(matches!(
        guest.cpu.run_slice(64).unwrap(),
        prime_emulator::engine::RunSliceOutcome::Progress
    ));
    assert_eq!(guest.cpu.pc(), target);
    guest.cpu.run_slice(64).unwrap();
    assert_eq!(guest.cpu.reg(R::R0), 42);
    assert_eq!(guest.mem.read_u32(base + 0x11c0).unwrap(), target);
    assert!(guest.cpu.fault_report().is_none());
    let module = guest.shared.pe_module_by_base(base).unwrap();
    let json = serde_json::to_vec(&module).unwrap();
    let decoded: prime_emulator::services::pe::PeModule = serde_json::from_slice(&json).unwrap();
    assert_eq!(
        decoded.unresolved_imports[&trap].description(),
        "LATE.DLL!ActualFunction"
    );
}
