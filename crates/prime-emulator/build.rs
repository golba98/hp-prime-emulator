// Link libatomic: Unicorn 2.x (built from source by `unicorn-engine-sys`)
// references the 16-byte atomic helpers (`__atomic_*_16`) from its bundled
// QEMU TCG code. On Fedora's toolchain (GCC 16 + rust-lld) these are provided
// by libatomic, which is not pulled in automatically. Verified during Phase 1
// (see docs/emulator/FEDORA_COMPATIBILITY.md, finding F-2).
//
// On platforms where libatomic is part of the default link set this flag is
// redundant but harmless.
fn main() {
    println!("cargo:rustc-link-lib=atomic");
}
