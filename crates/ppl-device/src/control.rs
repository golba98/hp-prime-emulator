//! Bounded local IPC for one exclusive device owner. There is deliberately no
//! raw USB, key injection, overwrite, deletion, or execution RPC.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::{
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    path::Path,
    time::Duration,
};

pub const VERSION: u32 = 1;
pub const MAX_METADATA: usize = 1024 * 1024;
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status,
    Capture { format: u8 },
    Program { name: String },
    Inventory,
    Stop,
}
#[derive(Debug)]
pub struct Response {
    pub report: Value,
    pub payload: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<T> {
    version: u32,
    message: T,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn write_message<T: Serialize>(out: &mut impl Write, message: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(&Envelope {
        version: VERSION,
        message,
    })
    .map_err(io::Error::other)?;
    if bytes.len() > MAX_METADATA {
        return Err(invalid("control message exceeds limit"));
    }
    out.write_all(&(bytes.len() as u32).to_be_bytes())?;
    out.write_all(&bytes)
}
pub fn read_message<T: serde::de::DeserializeOwned>(input: &mut impl Read) -> io::Result<T> {
    let mut len = [0; 4];
    input.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_METADATA {
        return Err(invalid("control message exceeds limit"));
    }
    let mut bytes = vec![0; len];
    input.read_exact(&mut bytes)?;
    let envelope: Envelope<T> = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if envelope.version != VERSION {
        return Err(invalid("unsupported control protocol version"));
    }
    Ok(envelope.message)
}
pub fn write_response(out: &mut impl Write, response: &Response) -> io::Result<()> {
    if response.payload.len() > MAX_PAYLOAD {
        return Err(invalid("control payload exceeds limit"));
    }
    write_message(out, &response.report)?;
    out.write_all(&(response.payload.len() as u32).to_be_bytes())?;
    out.write_all(&response.payload)
}
pub fn read_response(input: &mut impl Read) -> io::Result<Response> {
    let report = read_message(input)?;
    let mut len = [0; 4];
    input.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_PAYLOAD {
        return Err(invalid("control payload exceeds limit"));
    }
    let mut payload = vec![0; len];
    input.read_exact(&mut payload)?;
    Ok(Response { report, payload })
}
#[cfg(unix)]
pub fn effective_uid() -> u32 {
    // SAFETY: geteuid has no arguments, pointers, or failure mode.
    unsafe { libc::geteuid() }
}
#[cfg(unix)]
pub fn validate_private_directory(path: &Path) -> io::Result<()> {
    let m = path.symlink_metadata()?;
    if !m.is_dir() || m.uid() != effective_uid() || m.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "control directory must be owned by this user, private (0700), and not a symlink",
        ));
    }
    Ok(())
}
#[cfg(unix)]
pub fn call(path: &Path, request: &Request) -> io::Result<Response> {
    validate_private_directory(
        path.parent()
            .ok_or_else(|| invalid("socket has no parent"))?,
    )?;
    let m = path.symlink_metadata()?;
    if !m.file_type().is_socket() || m.uid() != effective_uid() || m.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "control socket must be private and owned by this user",
        ));
    }
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(match request {
        Request::Inventory => 135,
        Request::Capture { .. } => 30,
        _ => 45,
    })))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write_message(&mut stream, request)?;
    read_response(&mut stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_versioned_metadata_and_binary_roundtrip() {
        let response = Response {
            report: serde_json::json!({"execution_confirmed":false}),
            payload: vec![0, 255, 1],
        };
        let mut bytes = vec![];
        write_response(&mut bytes, &response).unwrap();
        let decoded = read_response(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded.payload, response.payload);
        assert_eq!(decoded.report, response.report);
        assert!(read_message::<Request>(&mut u32::MAX.to_be_bytes().as_slice()).is_err());
        let text = br#"{"version":2,"message":{"operation":"stop"}}"#;
        let mut bytes = (text.len() as u32).to_be_bytes().to_vec();
        bytes.extend(text);
        assert!(read_message::<Request>(&mut bytes.as_slice()).is_err());
        assert!(
            serde_json::from_value::<Request>(serde_json::json!({"operation":"keys"})).is_err()
        );
    }
}
