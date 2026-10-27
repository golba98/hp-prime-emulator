//! Read-only settings snapshots, with identity-bound opaque baselines.
use super::{common::exit, device::DeviceOpts};
use clap::{Subcommand, ValueEnum};
use ppl_device::{DeviceError, settings::SettingsKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Kind {
    Home,
    Cas,
}
impl Kind {
    fn wire(self) -> SettingsKind {
        match self {
            Self::Home => SettingsKind::Home,
            Self::Cas => SettingsKind::Cas,
        }
    }
}
#[derive(Debug, Subcommand)]
pub enum Action {
    /// Preserve the exact binary settings payload and its metadata; never overwrites.
    Read {
        kind: Kind,
        #[arg(long)]
        out: PathBuf,
    },
    /// Compare live settings with an identity-bound raw snapshot and sidecar.
    Verify {
        kind: Kind,
        #[arg(long)]
        baseline: PathBuf,
    },
}
#[derive(Serialize, Deserialize)]
struct Metadata {
    schema_version: u32,
    device: ppl_programs::DeviceIdentity,
    object_name: String,
    object_type: u8,
    bytes: usize,
    sha256: String,
}
fn sidecar(path: &Path) -> PathBuf {
    path.with_extension("settings.metadata.json")
}
fn fail(error: impl ToString) -> DeviceError {
    DeviceError::Refused(error.to_string())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), DeviceError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(fail)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(fail)
}
fn compare_baseline(
    metadata: &Metadata,
    bytes: &[u8],
    device: &ppl_programs::DeviceIdentity,
    kind: Kind,
    live: &[u8],
) -> Result<bool, DeviceError> {
    if metadata.schema_version != 1
        || &metadata.device != device
        || metadata.object_type != 0
        || metadata.object_name != kind.wire().object_name()
        || metadata.bytes != bytes.len()
        || metadata.sha256 != ppl_compat::sha256_hex(bytes)
        || bytes.is_empty()
    {
        return Err(fail(
            "settings baseline identity, category, length or hash mismatch",
        ));
    }
    Ok(bytes == live)
}
fn perform(opts: &DeviceOpts, action: &Action) -> Result<(Value, u8), DeviceError> {
    if opts.control_socket.is_some() || opts.trace.is_some() {
        return Err(fail(
            "settings needs a standalone connection; control-socket and trace are unsupported",
        ));
    }
    let (kind, baseline) = match action {
        Action::Read { kind, out } => {
            if out.exists() || sidecar(out).exists() {
                return Err(fail("snapshot or metadata already exists"));
            }
            (*kind, None)
        }
        Action::Verify { kind, baseline } => {
            let raw = fs::read(baseline).map_err(fail)?;
            let metadata: Metadata =
                serde_json::from_slice(&fs::read(sidecar(baseline)).map_err(fail)?)
                    .map_err(fail)?;
            (*kind, Some((raw, metadata)))
        }
    };
    let mut prime = ppl_programs::open_selected(opts.device.as_deref())?;
    let device = ppl_programs::DeviceIdentity::read(&mut prime)?;
    let received = prime.read_settings(kind.wire()).and_then(|file| {
        device.verify(&mut prime)?;
        Ok(file)
    });
    if let Some(path) = &opts.log {
        fs::write(path, prime.log_text()).map_err(fail)?;
    }
    let file = received?;
    let hash = ppl_compat::sha256_hex(&file.data);
    let matched = baseline
        .as_ref()
        .map(|(raw, m)| compare_baseline(m, raw, &device, kind, &file.data))
        .transpose()?;
    let present = !file.data.is_empty();
    let mut result = json!({"schema_version":1,"operation":if baseline.is_some() {"settings_verify"} else {"settings_read"},"device":device,
        "object_name":file.name,"object_type":0,"device_responded":true,
        "reply_integrity_valid":file.crc_ok,"payload_present":present,"bytes":file.data.len(),
        "sha256":if present {Some(hash.clone())} else {None},"encoding":"opaque_binary",
        "settings_decoded":false,"contents_preserved":false,"contents_compared":matched.is_some(),"contents_match":matched,
        "independently_verified":false,"status":if !present {"empty_reply"} else if matched==Some(false) {"baseline_mismatch"} else {"valid"}});
    if let Action::Read { out, .. } = action {
        if present {
            let metadata = Metadata {
                schema_version: 1,
                device,
                object_name: file.name,
                object_type: 0,
                bytes: file.data.len(),
                sha256: hash,
            };
            write_new(out, &file.data)?;
            write_new(
                &sidecar(out),
                &serde_json::to_vec_pretty(&metadata).map_err(fail)?,
            )?;
            result["snapshot_path"] = json!(out);
            result["contents_preserved"] = json!(true);
        }
    }
    Ok((
        result,
        if present && matched != Some(false) {
            exit::OK
        } else {
            exit::IO
        },
    ))
}
pub fn run(opts: &DeviceOpts, action: &Action) -> u8 {
    match perform(opts, action) {
        Ok((value, code)) => {
            if opts.json {
                println!("{value}");
            } else {
                println!(
                    "{}: {} bytes; payload present: {}; baseline match: {}",
                    value["object_name"],
                    value["bytes"],
                    value["payload_present"],
                    value["contents_match"]
                );
            }
            code
        }
        Err(e) => {
            if opts.json {
                println!(
                    "{}",
                    json!({"operation":"settings_read","status":"refused","error":e.to_string(),"independently_verified":false})
                );
            } else {
                eprintln!("{e}");
            }
            exit::IO
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn baseline_requires_identity_category_length_hash_and_content() {
        let device = ppl_programs::DeviceIdentity {
            serial_id: "mock".into(),
            usb_pid: 0x1541,
            firmware_build: Some(15157),
            boot_version: Some("V0.050.640".into()),
        };
        let mut m = Metadata {
            schema_version: 1,
            device: device.clone(),
            object_name: "calc.hpsettings".into(),
            object_type: 0,
            bytes: 2,
            sha256: ppl_compat::sha256_hex(&[1, 2]),
        };
        assert!(compare_baseline(&m, &[1, 2], &device, Kind::Home, &[1, 2]).unwrap());
        assert!(!compare_baseline(&m, &[1, 2], &device, Kind::Home, &[3, 4]).unwrap());
        assert!(compare_baseline(&m, &[1, 2], &device, Kind::Cas, &[1, 2]).is_err());
        m.device.serial_id = "another".into();
        assert!(compare_baseline(&m, &[1, 2], &device, Kind::Home, &[1, 2]).is_err());
        m.device = device.clone();
        m.sha256 = "corrupt".into();
        assert!(compare_baseline(&m, &[1, 2], &device, Kind::Home, &[1, 2]).is_err());
    }
}
