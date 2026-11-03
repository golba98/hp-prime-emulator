//! Invalid transfers fail before touching stream position, host allocation or data.
use prime_emulator::{
    memory::GuestMemory,
    services::{GuestArgs, SharedState, SvcCtx, fsops},
    trace::{TraceConfig, TraceHub},
};
use unicorn_engine::{
    Unicorn,
    unicorn_const::{Arch, Mode},
};

#[test]
fn malformed_file_transfers_are_atomic_and_bounded() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("C")).unwrap();
    std::fs::write(root.path().join("C/data"), b"sentinel").unwrap();
    let shared = SharedState::new(root.path().into(), "A:/test".into());
    let handle = shared.vfs.open("C:/data", "r+").unwrap();
    let mut mem = GuestMemory::new();
    mem.map("buffer", 0x1000, 4096).unwrap();
    let mem = std::sync::Arc::new(mem);
    let mut uc = Unicorn::new(Arch::ARM, Mode::ARM).unwrap();
    let trace = TraceHub::new(TraceConfig::default()).unwrap();
    let mut ctx = SvcCtx {
        uc: &mut uc,
        mem: &mem,
        shared: &shared,
        trace: &trace,
        thread_id: 0,
        redirected: false,
        args: GuestArgs {
            r0: 0x1fff,
            r1: 2,
            r2: 2,
            r3: handle,
            sp: 0,
            caller_pc: 0,
            stack_slots: [0; 2],
        },
    };
    assert!(fsops::fread(&mut ctx).is_err());
    assert_eq!(shared.vfs.tell(handle), Some(0));
    ctx.args.r0 = 0x1000;
    ctx.args.r1 = u32::MAX;
    ctx.args.r2 = u32::MAX;
    assert!(fsops::fread(&mut ctx).is_err());
    assert!(fsops::fwrite(&mut ctx).is_err());
    assert_eq!(shared.vfs.tell(handle), Some(0));
    assert_eq!(
        std::fs::read(root.path().join("C/data")).unwrap(),
        b"sentinel"
    );
    ctx.args.r1 = 1;
    ctx.args.r2 = 8;
    assert_eq!(fsops::fread(&mut ctx).unwrap(), 8);
    assert_eq!(mem.read_bytes_alloc(0x1000, 8).unwrap(), b"sentinel");
    ctx.args.r0 = 0;
    ctx.args.r1 = 0;
    ctx.args.r2 = u32::MAX;
    assert_eq!(fsops::fread(&mut ctx).unwrap(), 0);
    ctx.args.r3 = shared.vfs.open("C:/data", "rb").unwrap();
    assert_eq!(fsops::fwrite(&mut ctx).unwrap(), 0);
    shared.vfs.check_durable().unwrap();
}
