//! Explicit local-firmware USB target; cannot fall through to physical USB.
use clap::{Args, Subcommand};
use ppl_device::{DeviceError, FileType, Prime, Probe};
use ppl_engine::firmware_options::{AssetPaths, Options};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct VirtualOptions {
    /// Existing emulator storage profile. It is locked while firmware runs.
    #[arg(long)]
    pub profile: PathBuf,
    #[arg(long)]
    pub firmware: Option<PathBuf>,
    #[arg(long)]
    pub os_rom: Option<PathBuf>,
    #[arg(long)]
    pub runner: Option<PathBuf>,
    #[command(subcommand)]
    pub action: VirtualAction,
}
#[derive(Subcommand, Debug)]
pub enum VirtualAction {
    Ping,
    Info,
    Screenshot {
        out: PathBuf,
    },
    Get {
        name: String,
        out: PathBuf,
        #[arg(long)]
        note: bool,
    },
}
pub fn run(options: &VirtualOptions) -> u8 {
    match execute(options) {
        Ok(()) => super::common::exit::OK,
        Err(error) => {
            eprintln!("virtual firmware USB: {error}");
            super::common::exit::IO
        }
    }
}
fn execute(options: &VirtualOptions) -> Result<(), DeviceError> {
    let assets = AssetPaths {
        runner: options.runner.clone(),
        firmware: options.firmware.clone(),
        os_rom: options.os_rom.clone(),
        ..Default::default()
    }
    .resolve()
    .map_err(DeviceError::Protocol)?;
    let firmware = Options {
        runner: assets.runner,
        firmware: assets.firmware,
        os_rom: assets.os_rom,
        factory_a: assets.factory_a,
        init_data: assets.init_data,
        ..Default::default()
    };
    let mut prime = Prime::open_virtual(&firmware, &options.profile)?;
    match &options.action {
        VirtualAction::Ping => {
            println!("virtual G1 check-ready: {:02x?}", prime.check_ready()?);
        }
        VirtualAction::Info => {
            let info = prime.info()?;
            println!(
                "virtual G1 firmware build: {:?}; version: {}",
                info.build,
                info.version.as_deref().unwrap_or("unknown")
            );
        }
        VirtualAction::Screenshot { out } => {
            let png = prime.screenshot_timeout(8, std::time::Duration::from_secs(60))?;
            let image =
                ppl_graphics::Grob::from_prime_screenshot(&png).map_err(DeviceError::Protocol)?;
            image
                .save_png(out)
                .map_err(|e| DeviceError::Protocol(e.to_string()))?;
            println!(
                "saved {} ({}×{})",
                out.display(),
                image.width(),
                image.height()
            );
        }
        VirtualAction::Get { name, out, note } => {
            match prime.request_file(
                if *note {
                    FileType::Note
                } else {
                    FileType::Program
                },
                name,
            )? {
                Probe::Present(file) => {
                    std::fs::write(out, &file.data)
                        .map_err(|e| DeviceError::Protocol(e.to_string()))?;
                    println!("saved {} ({} bytes)", out.display(), file.data.len());
                }
                Probe::NoReply { .. } => {
                    return Err(DeviceError::Timeout("virtual firmware file response"));
                }
            }
        }
    }
    Ok(())
}
