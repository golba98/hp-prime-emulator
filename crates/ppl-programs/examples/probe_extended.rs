//! Read-only official negotiation probe; closes the changed session immediately.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let serial = std::env::args()
        .nth(1)
        .ok_or("expected exact serial fingerprint")?;
    let mut prime = ppl_programs::open_selected(Some(&serial))?;
    let identity = ppl_programs::DeviceIdentity::read(&mut prime)?;
    identity.require_reference()?;
    let result = prime.probe_extended_transport();
    let report = serde_json::json!({"device":identity,"application_mutations":0,
        "reply":result.as_ref().ok(),"error":result.as_ref().err().map(ToString::to_string),
        "exchanges":prime.log});
    println!("{}", serde_json::to_string_pretty(&report)?);
    result?;
    Ok(())
}
