fn main() {
    let directory = "src/text/legacy_gray";
    for file in ["adapter.c", "ftgrays.c", "ftgrays.h", "ftimage.h"] {
        println!("cargo:rerun-if-changed={directory}/{file}");
    }
    cc::Build::new()
        .file(format!("{directory}/adapter.c"))
        .include(directory)
        .warnings(false) // Upstream C retains its original compiler conventions.
        .compile("ppl_prime_gray");
}
