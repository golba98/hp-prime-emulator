//! Read-only forensic capture; never sends mutation commands.
use std::{fs, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("output directory required")?);
    fs::create_dir(&out)?;
    let mut prime = ppl_device::Prime::open_first()?;
    let info = prime.info()?;
    if prime.detected.product_id != 0x1541 || info.build != Some(15157) {
        return Err("not the reference G1".into());
    }
    println!("Device fingerprint: {:?}", info.serial_fingerprint());
    if std::env::args().any(|a| a == "--mode1") {
        prime.ensure_mode1()?;
    }
    let inventory = prime.receive_inventory()?;
    fs::write(out.join("usb.log"), prime.log_text())?;
    for file in &inventory.records {
        println!(
            "type={} name={:?} bytes={}",
            file.ty.code(),
            file.name,
            file.data.len()
        );
    }
    println!(
        "terminal={} reason={}",
        inventory.terminal_observed, inventory.reason
    );
    Ok(())
}
