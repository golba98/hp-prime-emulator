//! Physical control foundation. Hardware input has no verified backend: every
//! navigation/execution request refuses before any mutation or guessed packet.
use super::device::{DeviceAction, DeviceOpts};
use clap::Subcommand;
use ppl_device::{
    DeviceError, FileType, Prime,
    control::{Request, Response},
    screenshot::ScreenshotSession,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[derive(Debug, Subcommand)]
pub enum ProgramsAction {
    /// Receive the read-only F9 backup stream; completeness remains unverified.
    List,
    /// Retrieve exact physical UTF-16LE source, including original line endings.
    Show { name: String },
    /// Export exact device payload and an encoding/hash sidecar; never overwrites.
    Export {
        name: String,
        #[arg(long)]
        out: PathBuf,
    },
}
#[derive(Debug, Subcommand)]
pub enum UiAction {
    Inspect {
        #[arg(long, default_value = "artifacts/device-debug")]
        artifacts_dir: PathBuf,
        /// Optional local Tesseract executable (no shell or cloud service).
        #[arg(long)]
        ocr: Option<PathBuf>,
    },
    Home,
    Programs,
    Esc,
    Enter,
    Up,
    Down,
    Select {
        name: String,
    },
    Edit {
        name: String,
    },
    Check {
        name: String,
        #[arg(long)]
        dismiss: bool,
    },
}
#[derive(Debug, Subcommand)]
pub enum ControlAction {
    /// Hold the exclusive USB lock in the foreground; no background autostart.
    Serve {
        #[arg(long)]
        socket: PathBuf,
    },
    /// Inspect the explicitly selected local service.
    Status,
    /// Stop the local service; does not send a calculator key or cancellation.
    Stop,
}

pub(super) fn base(operation: &str) -> Value {
    json!({"schema_version":1,"operation":operation,"engine":"physical_g1",
        "request_id":null,"result":null,"result_type":null,
        "device":null,"status":"unsupported_capability","uploaded":false,
        "upload_state":"not_attempted","navigation_confirmed":false,
        "check_executed":false,"execution_confirmed":false,
        "physical_execution_verified":false,"completion_verified":false,
        "exit_state":"not_observed","recovery":{"attempted":false,"outcome":"not_requested"},
        "execution_state":"not_requested","diagnostics":[],"error":null,
        "evidence_paths":[],"safe_next_actions":["device status","device ui inspect"]})
}
fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
pub(super) fn failure(out: &mut Value, status: &str, code: u8, message: impl ToString) {
    out["status"] = json!(status);
    out["error"] = json!({"kind":status,"message":message.to_string(),"exit_code":code});
}
pub(super) fn private_directory(path: &Path, recursive: bool) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
pub(super) fn unsupported(operation: &str) -> Value {
    let mut out = base(operation);
    failure(
        &mut out,
        "unsupported_capability",
        6,
        "No physical remote-input backend is verified on G1 build 15157. Nothing was uploaded, checked, run, or injected; firmware-emulator results are not physical evidence.",
    );
    out
}
pub(super) fn emit(out: &Value, json_output: bool) -> u8 {
    if json_output {
        println!("{out}");
    } else {
        println!("{}", serde_json::to_string_pretty(out).expect("JSON value"));
    }
    out["error"]["exit_code"].as_u64().unwrap_or(0) as u8
}
fn device_failure(out: &mut Value, error: DeviceError) {
    let (status, code) = match &error {
        DeviceError::Busy(_) => ("busy", 8),
        DeviceError::Timeout(_) => ("timeout", 7),
        DeviceError::NotFound => ("device_disconnected", 5),
        DeviceError::Hid(_) => ("device_io_error", 5),
        DeviceError::Protocol(_) => ("invalid_evidence", 9),
        DeviceError::Inconclusive(_) => ("connection_retired", 5),
        _ => ("refused", 2),
    };
    failure(out, status, code, error);
}

/// These flags describe evidence, never permission or a simulated backend.
pub(super) fn capabilities(device: &Value) -> Value {
    let reference = device["usb_pid"] == json!(0x1541)
        && device["firmware_build"] == json!(15157)
        && device["boot_version"] == json!("V0.050.640");
    json!({"screenshot":reference,"program_read":reference,"program_upload":reference,
        "program_delete":reference,"program_inventory":false,"remote_keys":false,
        "remote_navigation":false,"remote_execution":false,
        "physical_error_capture":"screenshots_only",
        "firmware_emulator_check":super::device_diagnose::firmware_available(),
        "firmware_emulator_check_verification":"local_backend_configuration_not_physical_acceptance",
        "verified_actions":[],"verification_scope":if reference {"g1_15157_existing_hardware_evidence"} else {"unverified_profile"},
        "inventory_verification":"unverified","remote_input_verification":if reference {"tested_ec_encoding_ineffective"}else{"unverified_profile"},
        "authorization_required":["upload","delete","remote_input","execution"]})
}

pub(super) struct Owner {
    prime: Option<Prime>,
    stream: Option<ScreenshotSession>,
    pub(super) device: Value,
}
impl Owner {
    pub(super) fn new(mut prime: Prime) -> Result<Self, DeviceError> {
        let info = prime.info()?;
        let serial = info
            .serial
            .ok_or_else(|| DeviceError::Refused("device identity has no valid serial".into()))?;
        let device = json!({"model":prime.detected.model,"usb_pid":prime.detected.product_id,
            "firmware_build":info.build,"boot_version":info.version,
            "serial_id":ppl_compat::serial_fingerprint(&serial)});
        prime.log.clear(); // FA contains private identity; never emit its raw reports.
        Ok(Self {
            prime: Some(prime),
            stream: None,
            device,
        })
    }
    fn prime(&mut self) -> Result<&mut Prime, DeviceError> {
        if let Some(stream) = self.stream.take() {
            self.prime = Some(stream.into_prime()?);
        }
        self.prime.as_mut().ok_or(DeviceError::NotFound)
    }
    fn capture(
        &mut self,
        format: u8,
    ) -> Result<ppl_device::screenshot::ScreenshotCapture, DeviceError> {
        if !matches!(format, 8 | 10) {
            return Err(DeviceError::Refused(
                "control capture supports only RGB565 formats 08 and 0A".into(),
            ));
        }
        if self.stream.is_none() {
            self.stream = Some(ScreenshotSession::new(
                self.prime.take().ok_or(DeviceError::NotFound)?,
            ));
        }
        self.stream.as_mut().unwrap().capture(
            format,
            Duration::from_secs(20),
            &AtomicBool::new(false),
        )
    }
    pub(super) fn handle(&mut self, request: &Request) -> Response {
        let started = Instant::now();
        let requested_at = unix_ms();
        let op = match request {
            Request::Status => "status",
            Request::Capture { .. } => "capture",
            Request::Program { .. } => "program_show",
            Request::Inventory => "programs_list",
            Request::Manage { .. } => "program_management",
            Request::Stop => "control_stop",
        };
        let mut report = base(op);
        report["device"] = self.device.clone();
        let mut payload = Vec::new();
        let result = (|| -> Result<(), DeviceError> {
            if !matches!(request, Request::Stop) && self.prime.is_none() && self.stream.is_none() {
                return Err(DeviceError::Inconclusive("control connection was retired; stop and restart the service before further device operations".into()));
            }
            match request {
                Request::Status => {
                    let current = self.prime()?.info()?;
                    if current
                        .serial
                        .as_deref()
                        .map(ppl_compat::serial_fingerprint)
                        .as_deref()
                        != self.device["serial_id"].as_str()
                        || json!(current.build) != self.device["firmware_build"]
                        || json!(current.version) != self.device["boot_version"]
                    {
                        return Err(DeviceError::Refused(
                            "control device identity changed".into(),
                        ));
                    }
                    self.prime()?.log.clear();
                    report["status"] = json!("valid");
                    report["capabilities"] = capabilities(&self.device);
                    report["listener_running"] = Value::Null;
                }
                Request::Stop => {
                    report["status"] = json!("valid");
                }
                Request::Capture { format } => {
                    let capture = self.capture(*format)?;
                    payload = capture.png;
                    report["usb_timings"] = json!(capture.timings);
                    report["status"] = json!("valid");
                    report["format"] = json!(format);
                }
                Request::Program { name } => {
                    let file = self.prime()?.read_program_exact(name)?;
                    payload = file.data;
                    report["program"] = json!(name);
                    report["object_type"] = json!({"code":6,"name":"program"});
                    report["encoding"] = json!("utf-16le");
                    report["source_length_bytes"] = json!(payload.len());
                    report["sha256"] = json!(ppl_compat::sha256_hex(&payload));
                    report["verification"] = json!("physical_read_crc_name_type_verified");
                    match ppl_device::program::utf16(&payload) {
                        Ok(source) => {
                            report["source_length_utf16_units"] = json!(payload.len() / 2);
                            report["source_sha256"] =
                                json!(ppl_compat::sha256_hex(source.as_bytes()));
                            report["source"] = json!(source);
                            report["status"] = json!("valid");
                        }
                        Err(e) => {
                            report["source"] = Value::Null;
                            failure(&mut report, "diagnostic_uncertain", 9, e);
                        }
                    }
                }
                Request::Manage {
                    device_serial_id,
                    action,
                } => {
                    let result = ppl_programs::service::handle(
                        self.prime()?,
                        device_serial_id,
                        action,
                        &ppl_programs::delete_store::Store::new(Path::new(
                            super::device_delete::DEFAULT_DELETE_DIR,
                        )),
                    )?;
                    report["result"] = result;
                    report["status"] = json!("valid");
                }
                Request::Inventory => {
                    let inventory = self.prime()?.receive_inventory()?;
                    let programs:Vec<_> = inventory.records.iter().filter(|f| f.ty == FileType::Program)
                        .map(|f| json!({"name":f.name,"size_bytes":f.data.len(),"size_kind":"wire_payload",
                            "status":"present","sha256":ppl_compat::sha256_hex(&f.data),"crc_verified":true})).collect();
                    let other: Vec<_> = inventory
                        .records
                        .iter()
                        .filter(|f| matches!(f.ty, FileType::Other(_)))
                        .map(|f| json!({"object_type":f.ty.code(),"size_bytes":f.data.len()}))
                        .collect();
                    report["programs"] = json!(programs);
                    report["unknown_objects"] = json!(other);
                    report["inventory"] = json!({"completeness":"inventory_incomplete","complete":false,
                        "method":"hplp_f9_backup","terminal_observed":inventory.terminal_observed,
                        "record_count":inventory.records.len(),"reason":inventory.reason});
                    failure(&mut report, "inventory_incomplete", 9, &inventory.reason);
                    // Until terminal behavior has a verified hardware profile,
                    // this connection cannot safely return to command service.
                    self.prime = None;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            // Never reuse a handle after an incomplete response or uncertain I/O.
            self.stream = None;
            self.prime = None;
            device_failure(&mut report, error);
        }
        report["requested_at_unix_ms"] = json!(requested_at);
        report["connection_state"] = json!(if self.prime.is_some() || self.stream.is_some() {
            "open"
        } else {
            "retired"
        });
        if report["connection_state"] == "retired" && !matches!(request, Request::Stop) {
            report["recovery"] =
                json!({"attempted":false,"outcome":"restart_control_service_required"});
            report["safe_next_actions"] = json!(["control stop", "restart_control_service"]);
        }
        report["completed_at_unix_ms"] = json!(unix_ms());
        report["operation_elapsed_ms"] = json!(started.elapsed().as_secs_f64() * 1000.);
        Response { report, payload }
    }
}

pub fn handles(opts: &DeviceOpts) -> bool {
    opts.control_socket.is_some()
        || matches!(
            opts.action,
            DeviceAction::Programs { .. }
                | DeviceAction::Ui { .. }
                | DeviceAction::Control { .. }
                | DeviceAction::Diagnose(_)
                | DeviceAction::RunFile(_)
                | DeviceAction::Debug(_)
        )
}
pub(super) fn request(opts: &DeviceOpts, req: &Request) -> Response {
    if let Some(path) = &opts.control_socket {
        #[cfg(unix)]
        let result = (|| -> std::io::Result<Response> {
            if let Some(expected) = &opts.device {
                if !matches!(req, Request::Status) {
                    let identity = ppl_device::control::call(path, &Request::Status)?;
                    // Resolve selection before reading another calculator's
                    // source/inventory, or stopping another local owner.
                    if identity.report["device"]["serial_id"].as_str() != Some(expected)
                        || (!identity.report["error"].is_null() && !matches!(req, Request::Stop))
                    {
                        return Ok(identity);
                    }
                }
            }
            ppl_device::control::call(path, req)
        })();
        #[cfg(unix)]
        match result {
            Ok(response) => {
                if let Some(expected) = &opts.device {
                    if response.report["device"]["serial_id"].as_str() != Some(expected) {
                        let mut report = base("control_client");
                        failure(
                            &mut report,
                            "identity_mismatch",
                            9,
                            "service calculator differs from --device",
                        );
                        return Response {
                            report,
                            payload: vec![],
                        };
                    }
                }
                return response;
            }
            Err(e) => {
                let mut report = base("control_client");
                failure(&mut report, "service_unavailable", 5, e);
                return Response {
                    report,
                    payload: vec![],
                };
            }
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            return Response {
                report: unsupported("control_client"),
                payload: vec![],
            };
        }
    }
    match super::remote::open_selected(opts.device.as_deref()).and_then(Owner::new) {
        Ok(mut owner) => owner.handle(req),
        Err(e) => {
            let mut report = base("device_open");
            device_failure(&mut report, e);
            Response {
                report,
                payload: vec![],
            }
        }
    }
}

fn validate_program_response(name: &str, response: &Response) -> bool {
    let r = &response.report;
    if r["program"] != name
        || r["object_type"]["code"] != 6
        || r["encoding"] != "utf-16le"
        || r["source_length_bytes"].as_u64() != Some(response.payload.len() as u64)
        || r["sha256"] != ppl_compat::sha256_hex(&response.payload)
    {
        return false;
    }
    match ppl_device::program::utf16(&response.payload) {
        Ok(source) => {
            r["source"] == source && r["source_sha256"] == ppl_compat::sha256_hex(source.as_bytes())
        }
        Err(_) => r["source"].is_null() && r["status"] == "diagnostic_uncertain",
    }
}

pub fn run(opts: &DeviceOpts) -> u8 {
    let started = Instant::now();
    if opts.log.is_some() || opts.trace.is_some() {
        let mut out = base("control_options");
        failure(
            &mut out,
            "invalid_input",
            2,
            "raw USB logging is not supported by these commands; use the structured reports and screenshot evidence",
        );
        return emit(&out, opts.json);
    }
    let mut report = match &opts.action {
        DeviceAction::Diagnose(o) => return super::device_diagnose::diagnose(opts, o),
        DeviceAction::RunFile(o) => return super::device_diagnose::run_file(opts, o),
        DeviceAction::Debug(o) => return super::device_diagnose::debug(opts, o),
        DeviceAction::Control {
            action: ControlAction::Serve { socket },
        } => return serve(opts, socket),
        DeviceAction::Control {
            action: ControlAction::Status,
        }
        | DeviceAction::Status { request_id: None } => {
            if opts.control_socket.is_none() {
                let mut out = base("control_status");
                failure(
                    &mut out,
                    "invalid_input",
                    2,
                    "control status requires --control-socket PATH",
                );
                out
            } else {
                request(opts, &Request::Status).report
            }
        }
        DeviceAction::Control {
            action: ControlAction::Stop,
        } => {
            if opts.control_socket.is_none() {
                let mut out = base("control_stop");
                failure(
                    &mut out,
                    "invalid_input",
                    2,
                    "control stop requires --control-socket PATH",
                );
                out
            } else {
                request(opts, &Request::Stop).report
            }
        }
        DeviceAction::Programs { action } => {
            let (req, export) = match action {
                ProgramsAction::List => (Request::Inventory, None),
                ProgramsAction::Show { name } => (Request::Program { name: name.clone() }, None),
                ProgramsAction::Export { name, out } => {
                    (Request::Program { name: name.clone() }, Some(out))
                }
            };
            // Refuse existing destinations before touching USB.
            if let Some(path) = export {
                if path.exists() || path.with_extension("hpprgm.metadata.json").exists() {
                    let mut out = base("program_export");
                    failure(
                        &mut out,
                        "refused",
                        2,
                        "export destination or sidecar already exists",
                    );
                    return emit(&out, opts.json);
                }
            }
            let mut response = request(opts, &req);
            if let Request::Program { name } = &req {
                if matches!(
                    response.report["status"].as_str(),
                    Some("valid" | "diagnostic_uncertain")
                ) && !validate_program_response(name, &response)
                {
                    failure(
                        &mut response.report,
                        "invalid_evidence",
                        9,
                        "program response payload, source, name, type, or hash does not match its metadata",
                    );
                }
            }
            if let Some(path) = export {
                if response.report["error"].is_null()
                    || response.report["status"] == "diagnostic_uncertain"
                {
                    if let Err(e) = export_exact(path, &response.payload, &response.report) {
                        failure(&mut response.report, "io_error", 5, e);
                    } else {
                        response.report["operation"] = json!("program_export");
                        response.report["exported"] = json!(true);
                        response.report["evidence_paths"] =
                            json!([path, path.with_extension("hpprgm.metadata.json")]);
                    }
                }
            }
            response.report
        }
        DeviceAction::Ui {
            action: UiAction::Inspect { artifacts_dir, ocr },
        } => {
            let response = request(opts, &Request::Capture { format: 8 });
            if response.report["error"].is_null() {
                let second = request(opts, &Request::Capture { format: 8 });
                super::device_inspect::inspect(response, second, artifacts_dir, ocr.as_deref())
            } else {
                response.report
            }
        }
        DeviceAction::Ui { action } => {
            let mut out = unsupported("ui_navigation");
            out["requested_action"] = json!(format!("{action:?}"));
            out
        }
        #[cfg(unix)]
        DeviceAction::Mirror {
            scale,
            fps,
            fullscreen,
            snapshot,
            filter,
            quality,
            capture,
            diagnostics,
            acknowledge_experimental_fc04,
        } => {
            return mirror(
                opts,
                ppl_gui::MirrorConfig {
                    scale: *scale,
                    fps: *fps,
                    fullscreen: *fullscreen,
                    snapshot: snapshot.clone(),
                    filter: *filter,
                    quality: *quality,
                    capture: *capture,
                    diagnostics: diagnostics.clone(),
                    acknowledge_experimental_fc04: *acknowledge_experimental_fc04,
                    ..Default::default()
                },
            );
        }
        _ => unsupported("control_socket_operation"),
    };
    report["timings"] = json!({"total_ms":started.elapsed().as_secs_f64()*1000.});
    if !opts.json {
        if let DeviceAction::Programs {
            action: ProgramsAction::Show { .. },
        } = &opts.action
        {
            if report["error"].is_null() {
                eprintln!(
                    "{} — {} bytes, UTF-16LE, SHA-256 {}",
                    report["program"].as_str().unwrap_or("program"),
                    report["source_length_bytes"],
                    report["sha256"].as_str().unwrap_or("unknown")
                );
                print!("{}", report["source"].as_str().unwrap_or(""));
                return 0;
            }
        }
        if matches!(
            opts.action,
            DeviceAction::Programs {
                action: ProgramsAction::List
            }
        ) {
            if let Some(programs) = report["programs"].as_array() {
                println!(
                    "HP Prime — partial installed-program inventory (completeness unverified)\n\nNAME                             PAYLOAD BYTES  STATUS"
                );
                for program in programs {
                    let name = program["name"]
                        .as_str()
                        .unwrap_or("")
                        .escape_debug()
                        .to_string();
                    println!(
                        "{name:<32} {:>13}  Present (CRC verified)",
                        program["size_bytes"]
                    );
                }
                eprintln!(
                    "{}",
                    report["inventory"]["reason"]
                        .as_str()
                        .unwrap_or("inventory incomplete")
                );
                return report["error"]["exit_code"].as_u64().unwrap_or(9) as u8;
            }
        }
    }
    emit(&report, opts.json)
}

fn export_exact(path: &Path, payload: &[u8], report: &Value) -> std::io::Result<()> {
    use std::io::Write;
    // create_new also refuses dangling symlinks and races after preflight.
    let mut file = private_file(path)?;
    file.write_all(payload)?;
    file.sync_all()?;
    let sidecar = path.with_extension("hpprgm.metadata.json");
    let mut file = private_file(&sidecar)?;
    file.write_all(&serde_json::to_vec_pretty(report)?)?;
    file.sync_all()
}

#[cfg(unix)]
fn serve(opts: &DeviceOpts, path: &Path) -> u8 {
    use std::{
        io,
        os::unix::{
            fs::{DirBuilderExt, MetadataExt, PermissionsExt},
            net::UnixListener,
        },
    };
    let mut report = base("control_serve");
    if opts.control_socket.is_some() {
        failure(
            &mut report,
            "invalid_input",
            2,
            "serve cannot connect to another control socket",
        );
        return emit(&report, opts.json);
    }
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or("socket requires a private parent directory")?;
        if !parent.exists() {
            std::fs::DirBuilder::new().mode(0o700).create(parent)?;
        }
        ppl_device::control::validate_private_directory(parent)?;
        // Never unlink somebody else's socket or claim stale sessions automatically.
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        struct SocketCleanup {
            path: PathBuf,
            inode: u64,
        }
        impl Drop for SocketCleanup {
            fn drop(&mut self) {
                if self
                    .path
                    .symlink_metadata()
                    .is_ok_and(|m| m.ino() == self.inode)
                {
                    let _ = std::fs::remove_file(&self.path);
                }
            }
        }
        let _cleanup = SocketCleanup {
            path: path.into(),
            inode: path.symlink_metadata()?.ino(),
        };
        let mut owner = Owner::new(super::remote::open_selected(opts.device.as_deref())?)?;
        report["device"] = owner.device.clone();
        let cancel = ppl_engine::Cancellation::default();
        let signal = cancel.clone();
        ctrlc::set_handler(move || signal.cancel())?;
        listener.set_nonblocking(true)?;
        eprintln!(
            "USB owner ready at {}; read-only operations only",
            path.display()
        );
        while !cancel.cancelled() {
            let (mut stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            stream.set_write_timeout(Some(Duration::from_secs(5)))?;
            let req = match ppl_device::control::read_message::<Request>(&mut stream) {
                Ok(req) => req,
                Err(_) => continue,
            };
            let stop = matches!(req, Request::Stop);
            let response = owner.handle(&req);
            // A disconnected client never causes an operation to be retried.
            let _ = ppl_device::control::write_response(&mut stream, &response);
            if stop {
                break;
            }
        }
        report["status"] = json!("valid");
        Ok(())
    })();
    if let Err(e) = result {
        let busy = matches!(e.downcast_ref::<DeviceError>(), Some(DeviceError::Busy(_)))
            || e.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::AddrInUse);
        failure(
            &mut report,
            if busy { "busy" } else { "service_error" },
            if busy { 8 } else { 5 },
            e,
        );
    }
    emit(&report, opts.json)
}
#[cfg(not(unix))]
fn serve(opts: &DeviceOpts, _: &Path) -> u8 {
    emit(&unsupported("control_serve"), opts.json)
}

#[cfg(unix)]
struct SocketMirror {
    path: PathBuf,
    identity: Value,
    format: u8,
    decoder: ppl_graphics::PrimeScreenshotDecoder,
}
#[cfg(unix)]
impl ppl_gui::MirrorSource for SocketMirror {
    fn capture_frame(&mut self) -> Result<ppl_graphics::Grob, DeviceError> {
        self.capture(&AtomicBool::new(false))
            .map(|frame| std::sync::Arc::unwrap_or_clone(frame.grob))
    }
    fn capture(
        &mut self,
        cancel: &AtomicBool,
    ) -> Result<ppl_gui::mirror::SourceFrame, DeviceError> {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err(DeviceError::Refused("capture cancelled".into()));
        }
        let requested = Instant::now();
        let r = ppl_device::control::call(
            &self.path,
            &Request::Capture {
                format: self.format,
            },
        )
        .map_err(|e| DeviceError::Hid(e.to_string()))?;
        if r.report["device"] != self.identity {
            return Err(DeviceError::Refused(
                "control mirror identity changed".into(),
            ));
        }
        if !r.report["error"].is_null() {
            return Err(DeviceError::Hid(r.report["error"].to_string()));
        }
        let expected = if self.format == 8 {
            (320, 240)
        } else {
            (160, 120)
        };
        let (grob, decode) = self
            .decoder
            .decode(&r.payload, Some(expected))
            .map_err(DeviceError::Protocol)?;
        let usb = serde_json::from_value(r.report["usb_timings"].clone())
            .map_err(|e| DeviceError::Protocol(format!("invalid owner capture timings: {e}")))?;
        Ok(ppl_gui::mirror::SourceFrame {
            grob: std::sync::Arc::new(grob),
            requested,
            completed: Instant::now(),
            usb,
            decode,
            raw: Some(r.payload),
            update: None,
        })
    }
    fn device_name(&self) -> &str {
        self.identity["model"].as_str().unwrap_or("HP Prime")
    }
    fn device_serial(&self) -> Option<&str> {
        self.identity["serial_id"].as_str()
    }
    fn try_reconnect(&mut self) -> Result<bool, DeviceError> {
        Ok(false)
    }
}
#[cfg(unix)]
fn mirror(opts: &DeviceOpts, config: ppl_gui::MirrorConfig) -> u8 {
    let response = request(opts, &Request::Status);
    if !response.report["error"].is_null() {
        return emit(&response.report, opts.json);
    }
    let source = SocketMirror {
        path: opts.control_socket.clone().expect("socket dispatch"),
        identity: response.report["device"].clone(),
        format: if config.quality == ppl_gui::MirrorQuality::Full {
            8
        } else {
            10
        },
        decoder: Default::default(),
    };
    match ppl_gui::run_mirror(config, source) {
        Ok(()) => 0,
        Err(e) => {
            let mut out = base("mirror");
            failure(&mut out, "mirror_error", 5, e);
            emit(&out, opts.json)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };
    struct Fake {
        pending: VecDeque<Vec<u8>>,
        commands: Arc<Mutex<Vec<u8>>>,
        wrong_name: bool,
    }
    impl ppl_device::Transport for Fake {
        fn write(&mut self, request: &[u8]) -> Result<usize, DeviceError> {
            assert_eq!(&request[..2], &[0, 0]);
            let cmd = request[2];
            self.commands.lock().unwrap().push(cmd);
            let raw = match cmd {
                0xfa => {
                    let mut raw = vec![0xfa, 1, 0, 0, 0, 64];
                    raw.resize(70, 0);
                    raw[34..36].copy_from_slice(&15157u16.to_le_bytes());
                    raw[38..48].copy_from_slice(b"V0.050.640");
                    raw[54..64].copy_from_slice(b"1234567890");
                    raw
                }
                0xf8 => ppl_device::files::encode_file_message(
                    0xf7,
                    FileType::Program,
                    if self.wrong_name { "OTHER" } else { "PERSONAL" },
                    &"EXPORT PERSONAL()\r\nBEGIN END;"
                        .encode_utf16()
                        .flat_map(u16::to_le_bytes)
                        .collect::<Vec<_>>(),
                ),
                0xfc => {
                    let png = include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../tests/fixtures/prime-rgb565.png"
                    ));
                    let mut raw = vec![0xfc, 1, 0, 0, 0, 0, 0, 0, request[3], 255, 255, 255, 255];
                    raw.extend(png);
                    let n = raw.len() - 6;
                    raw[2..6].copy_from_slice(&(n as u32).to_be_bytes());
                    let crc = ppl_device::crc16(&raw[6..]);
                    raw[6..8].copy_from_slice(&crc.to_be_bytes());
                    raw
                }
                _ => panic!("unexpected USB write {cmd:02x}; test permits read-only commands only"),
            };
            self.pending = raw
                .chunks(63)
                .enumerate()
                .map(|(n, chunk)| {
                    let mut r = vec![((n + n / 255) & 255) as u8];
                    r.extend(chunk);
                    r
                })
                .collect();
            Ok(request.len())
        }
        fn read_timeout(&mut self, out: &mut [u8], _: i32) -> Result<usize, DeviceError> {
            if let Some(r) = self.pending.pop_front() {
                out[..r.len()].copy_from_slice(&r);
                Ok(r.len())
            } else {
                Ok(0)
            }
        }
    }
    fn owner(wrong_name: bool) -> (Owner, Arc<Mutex<Vec<u8>>>) {
        let commands = Arc::new(Mutex::new(vec![]));
        let prime = Prime::with_transport(
            ppl_device::Detected {
                vendor_id: 0x03f0,
                product_id: 0x1541,
                model: "mock G1",
                path: "test-only".into(),
                manufacturer: None,
                product: None,
                serial: None,
                release: 0,
                interface: 0,
            },
            Box::new(Fake {
                pending: VecDeque::new(),
                commands: commands.clone(),
                wrong_name,
            }),
        );
        (Owner::new(prime).unwrap(), commands)
    }
    #[test]
    fn screen_stream_hands_off_to_exact_reads_and_back_without_mutations() {
        let (mut owner, commands) = owner(false);
        let capture = owner.handle(&Request::Capture { format: 8 });
        assert!(capture.report["error"].is_null(), "{}", capture.report);
        let program = owner.handle(&Request::Program {
            name: "PERSONAL".into(),
        });
        assert!(program.report["error"].is_null(), "{}", program.report);
        assert_eq!(program.report["source"], "EXPORT PERSONAL()\r\nBEGIN END;");
        assert!(!program.report.to_string().contains("1234567890"));
        let capture2 = owner.handle(&Request::Capture { format: 8 });
        assert_eq!(capture.payload, capture2.payload);
        let status = owner.handle(&Request::Status);
        assert!(status.report["error"].is_null());
        assert_eq!(status.report["capabilities"]["remote_execution"], false);
        assert_eq!(*commands.lock().unwrap(), [0xfa, 0xfc, 0xf8, 0xfc, 0xfa]);
    }
    #[test]
    fn wrong_program_identity_poisoning_prevents_followup_traffic() {
        let (mut owner, commands) = owner(true);
        let response = owner.handle(&Request::Program {
            name: "PERSONAL".into(),
        });
        assert_eq!(response.report["status"], "invalid_evidence");
        assert!(response.payload.is_empty());
        let before = commands.lock().unwrap().len();
        let response = owner.handle(&Request::Capture { format: 8 });
        assert_eq!(response.report["status"], "connection_retired");
        assert_eq!(commands.lock().unwrap().len(), before);
        let response = owner.handle(&Request::Stop);
        assert_eq!(response.report["status"], "valid");
        assert_eq!(response.report["connection_state"], "retired");
        assert_eq!(commands.lock().unwrap().len(), before);
    }
    #[test]
    fn arbitrary_devices_and_successful_mock_navigation_never_enable_input() {
        for id in [
            json!({}),
            json!({"usb_pid":0x2441,"firmware_build":15157,"boot_version":"V0.050.640"}),
            json!({"usb_pid":0x1541,"firmware_build":15157,"boot_version":"V0.050.640"}),
        ] {
            let caps = capabilities(&id);
            for name in [
                "remote_keys",
                "remote_navigation",
                "remote_execution",
                "program_inventory",
            ] {
                assert_eq!(caps[name], false);
            }
        }
    }
}
