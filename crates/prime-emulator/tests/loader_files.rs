//! Loader file descriptors (`_OpenFile` / `_OpenSubFile` / `_ReadFile` /
//! `_FseekFile` / `_FileSize` / `_CloseFile`) through the real SVC path.
//!
//! The firmware reads its resources (fonts, images) out of the
//! `armfir.dat` container through sub-file windows. Closing a sub-file
//! must leave the container open: a sub-file's `parent_fd` is its parent
//! *descriptor* (PrimU2), never 0. M4 found sub-files inheriting the
//! parent's 0, so the first sub-file close closed the container and every
//! later resource read failed (no text was rendered on the firmware UI).

use std::sync::Arc;

use prime_emulator::engine::CpuThread;
use prime_emulator::memory::GuestMemory;
use prime_emulator::services::{Registry, SharedState, UnsupportedPolicy, handlers, names};
use prime_emulator::trace::{TraceConfig, TraceHub};
use unicorn_engine::RegisterARM;

const CODE: u32 = 0x1000_0000;
const DATA: u32 = 0x1000_0800;

struct Guest {
    cpu: CpuThread,
    mem: Arc<GuestMemory>,
}

impl Guest {
    fn new(data_dir: &std::path::Path) -> Guest {
        let mut mem = GuestMemory::new();
        mem.map("code", CODE, 0x10_000).unwrap();
        mem.map("heap", 0x2000_0000, 0x10_000).unwrap();
        mem.set_heap(0x2000_0000, 0x10_000);
        let mem = Arc::new(mem);
        let mut registry = Registry::new();
        handlers::install(&mut registry);
        let shared = Arc::new(SharedState::new(data_dir.to_path_buf(), "A:\\t".into()));
        let cpu = CpuThread::new(
            mem.clone(),
            Arc::new(registry),
            shared,
            Arc::new(TraceHub::new(TraceConfig::default()).unwrap()),
            UnsupportedPolicy::Stop,
            0,
            Vec::new(),
        )
        .unwrap();
        Guest { cpu, mem }
    }

    /// Calls service `name(r0, r1, r2)` through the firmware's stub idiom
    /// (`push {r0}; push {lr}; svc #id; pop {pc}`) and returns r0.
    fn call(&mut self, name: &str, args: [u32; 3]) -> u32 {
        let id = names::service_id_by_name(name).unwrap();
        let svc = 0xEF00_0000 | id;
        let stub: Vec<u8> = [0xE92D_0001u32, 0xE92D_4000, svc, 0xE8BD_8000, 0xEAFF_FFFE]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        self.mem.write_bytes(CODE, &stub).unwrap();
        self.cpu.set_entry(CODE, 0x1000_8000, args[0]).unwrap();
        self.cpu.set_reg(RegisterARM::R1, args[1]).unwrap();
        self.cpu.set_reg(RegisterARM::R2, args[2]).unwrap();
        self.cpu.set_lr(CODE + 16).unwrap(); // `b .` after the stub
        let _ = self.cpu.run_slice(64).unwrap();
        self.cpu.reg(RegisterARM::R0)
    }

    /// Like [`Guest::call`] but leaves r3 as the caller set it.
    fn call_keep_r3(&mut self, name: &str, args: [u32; 3]) -> u32 {
        let r3 = self.cpu.reg(RegisterARM::R3);
        let id = names::service_id_by_name(name).unwrap();
        let svc = 0xEF00_0000 | id;
        let stub: Vec<u8> = [0xE92D_0001u32, 0xE92D_4000, svc, 0xE8BD_8000, 0xEAFF_FFFE]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        self.mem.write_bytes(CODE, &stub).unwrap();
        self.cpu.set_entry(CODE, 0x1000_8000, args[0]).unwrap();
        self.cpu.set_reg(RegisterARM::R1, args[1]).unwrap();
        self.cpu.set_reg(RegisterARM::R2, args[2]).unwrap();
        self.cpu.set_reg(RegisterARM::R3, r3).unwrap();
        self.cpu.set_lr(CODE + 16).unwrap();
        let _ = self.cpu.run_slice(64).unwrap();
        self.cpu.reg(RegisterARM::R0)
    }

    fn cstr(&self, va: u32, s: &str) -> u32 {
        self.mem.write_cstr(va, s).unwrap();
        va
    }

    fn read(&mut self, fd: u32, n: u32) -> Vec<u8> {
        let got = self.call("_ReadFile", [fd, DATA + 0x400, n]);
        self.mem
            .read_bytes_alloc(DATA + 0x400, got as usize)
            .unwrap()
    }
}

#[test]
fn closing_a_sub_file_keeps_the_container_open() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("A")).unwrap();
    let content: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
    std::fs::write(dir.path().join("A/RES.DAT"), &content).unwrap();
    let mut g = Guest::new(dir.path());

    let path = g.cstr(DATA, "A:\\RES.DAT");
    let mode = g.cstr(DATA + 0x40, "rb");
    let root = g.call("_OpenFile", [path, mode, 0]);
    assert_ne!(root, 0, "container opens");
    assert_eq!(g.call("_FileSize", [root, 0, 0]), 4096);
    // Reference layout: parent_fd 0, subfile_base 0, size at +12.
    assert_eq!(g.mem.read_u32(root + 4).unwrap(), 0);
    assert_eq!(g.mem.read_u32(root + 8).unwrap(), 0);
    assert_eq!(g.mem.read_u32(root + 12).unwrap(), 4096);

    // Two resources as sub-file windows, like the firmware's loader.
    let a = g.call("_OpenSubFile", [root, 100, 16]);
    let b = g.call("_OpenSubFile", [root, 1000, 8]);
    assert!(a != 0 && b != 0);
    assert_eq!(g.mem.read_u32(a + 4).unwrap(), root, "parent descriptor");
    assert_eq!(g.mem.read_u32(a + 8).unwrap(), 100, "subfile_base");
    assert_eq!(g.mem.read_u32(a + 12).unwrap(), 16, "size");
    assert_eq!(g.call("_FileSize", [a, 0, 0]), 16);
    assert_eq!(
        g.read(a, 64),
        content[100..116].to_vec(),
        "bounded to the window"
    );

    // Closing one resource must not close the container.
    assert_eq!(g.call("_CloseFile", [a, 0, 0]), 0);
    assert_eq!(
        g.read(b, 8),
        content[1000..1008].to_vec(),
        "sibling still reads"
    );
    assert_eq!(g.call("_FseekFile", [root, 4000, 0]), 0);
    assert_eq!(
        g.read(root, 16),
        content[4000..4016].to_vec(),
        "container still reads"
    );

    // Nested window: base accumulates through the parent.
    let nested = g.call("_OpenSubFile", [b, 2, 4]);
    assert_eq!(g.read(nested, 4), content[1002..1006].to_vec());
    // Out-of-range windows are refused.
    assert_eq!(g.call("_OpenSubFile", [root, 4090, 16]), 0);

    // Closing the root closes the host file: reads then return nothing.
    assert_eq!(g.call("_CloseFile", [root, 0, 0]), 0);
    assert!(g.read(b, 8).is_empty());
}

#[test]
fn write_private_profile_string_takes_the_file_name_from_r3() {
    // Regression (M5): the file name was read from the stack, so the
    // firmware's `[ENV] PBITS=…` landed in a stray `C/C/DATA` file.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("C/DATA")).unwrap();
    let mut g = Guest::new(dir.path());
    let app = g.cstr(DATA, "ENV");
    let key = g.cstr(DATA + 0x20, "PBITS");
    let value = g.cstr(DATA + 0x40, "3");
    let file = g.cstr(DATA + 0x60, "C:\\DATA\\TEST.INI");
    // r0..r2 via call(); r3 set explicitly.
    g.cpu.set_reg(RegisterARM::R3, file).unwrap();
    let ok = g.call_keep_r3("_WritePrivateProfileString", [app, key, value]);
    assert_eq!(ok, 1);
    let ini = std::fs::read_to_string(dir.path().join("C/DATA/TEST.INI")).unwrap();
    assert!(ini.contains("[ENV]") && ini.contains("PBITS=3"), "{ini}");
    assert!(!dir.path().join("C/C").exists());
}
