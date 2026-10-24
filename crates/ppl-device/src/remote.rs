//! Request-correlated physical mailbox transactions. No simulated execution.
use crate::{ControlObject, DeviceError, FileType, Prime, Probe, ReadBack, Result};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub request_note: String,
    pub receipt_note: String,
    pub result_note: String,
    pub nonce: String,
    pub payload: String,
}

impl Request {
    pub fn new(session: &str, expression: &str, probe: bool) -> Result<Self> {
        if session.len() != 6 || !session.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(DeviceError::Refused("invalid listener session".into()));
        }
        let mut random = [0u8; 18];
        getrandom::fill(&mut random)
            .map_err(|e| DeviceError::Refused(format!("random request ID unavailable: {e}")))?;
        let hex = |b: &[u8]| b.iter().map(|b| format!("{b:02X}")).collect::<String>();
        let id = format!("{session}{}", hex(&random[..2]));
        let nonce = hex(&random[2..]);
        let payload = format!("{}|{nonce}|{expression}", if probe { "P" } else { "E" });
        if payload.encode_utf16().count() > crate::listener::MAX_REQUEST_CHARS {
            return Err(DeviceError::Refused(
                "request exceeds 4096 UTF-16 units".into(),
            ));
        }
        Ok(Self {
            request_note: format!("ZZT_Q{id}"),
            receipt_note: format!("ZZT_A{id}"),
            result_note: format!("ZZT_R{id}"),
            id,
            nonce,
            payload,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reply {
    pub success: bool,
    pub result_type: i32,
    pub text: String,
    /// Complete validated terminal note, retained for hardware evidence.
    pub raw_note: String,
}

pub fn decode_reply(request: &Request, text: &str) -> Result<Reply> {
    let text = text.replace("\r\n", "\n");
    let (head, body) = text
        .split_once('\n')
        .ok_or_else(|| DeviceError::Protocol("result has no header separator".into()))?;
    let fields: Vec<_> = head.split('|').collect();
    if fields.len() != 6
        || fields[0] != "@@PPLREMOTE"
        || fields[1] != "1"
        || fields[2] != request.request_note
        || fields[3] != request.nonce
        || !matches!(fields[4], "ok" | "error")
    {
        return Err(DeviceError::Protocol(
            "result identity, version, or status mismatch".into(),
        ));
    }
    let result_type = fields[5]
        .parse::<i32>()
        .map_err(|_| DeviceError::Protocol("invalid result type".into()))?;
    let payload = body
        .strip_suffix("\n@@END")
        .ok_or_else(|| DeviceError::Protocol("result is incomplete (missing END marker)".into()))?;
    if payload.encode_utf16().count() > crate::listener::MAX_RESULT_CHARS {
        return Err(DeviceError::Protocol("result exceeds limit".into()));
    }
    Ok(Reply {
        success: fields[4] == "ok",
        result_type,
        text: payload.into(),
        raw_note: text.clone(),
    })
}

/// Upload only after the caller has durably recorded the pending request.
/// Callback records the last safe state before any F7 can be sent.
pub fn publish<F>(
    prime: &mut Prime,
    request: &Request,
    control: &ControlObject,
    before_send: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let guard = prime.preflight_mailbox(request, Some(control))?;
    before_send()?;
    let upload = prime.send_new_note(guard, &request.payload)?;
    if upload.readback != ReadBack::Matches {
        return Err(DeviceError::Protocol(format!(
            "request upload unverified: {:?}; do not resend",
            upload.readback
        )));
    }
    Ok(())
}

/// A missing result means uncertainty, not failure to execute.
pub fn poll(prime: &mut Prime, request: &Request, deadline: Instant) -> Result<Reply> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(DeviceError::Timeout(
                "physical execution result (execution may still be running)",
            ));
        }
        match prime.probe_file(
            FileType::Note,
            &request.result_note,
            remaining.min(Duration::from_secs(8)),
        )? {
            Probe::Present(file) => {
                if file.name != request.result_note || file.ty != FileType::Note || !file.crc_ok {
                    return Err(DeviceError::Protocol(
                        "result object identity or CRC mismatch".into(),
                    ));
                }
                return decode_reply(request, &file.text());
            }
            Probe::NoReply { .. } => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request::new("ABC123", "2+2", false).unwrap()
    }
    fn note(r: &Request, body: &str) -> String {
        format!(
            "@@PPLREMOTE|1|{}|{}|ok|0\n{body}\n@@END",
            r.request_note, r.nonce
        )
    }
    #[test]
    fn request_names_are_bounded_and_related() {
        let r = request();
        for n in [&r.request_note, &r.receipt_note, &r.result_note] {
            crate::validate_test_name(n).unwrap();
            assert_eq!(n.len(), 15);
        }
        assert_eq!(r.nonce.len(), 32);
        assert!(r.payload.ends_with("|2+2"));
        assert_ne!(r.nonce, request().nonce);
        assert!(Request::new("../bad", "2", false).is_err());
        assert!(Request::new("ABC123", &"😀".repeat(2040), false).is_err());
    }
    #[test]
    fn terminal_note_requires_identity_nonce_status_and_end() {
        let r = request();
        let good = note(&r, "4");
        let reply = decode_reply(&r, &good).unwrap();
        assert!(reply.success);
        assert_eq!(reply.text, "4");
        assert_eq!(reply.result_type, 0);
        for bad in [
            good.replace(&r.nonce, "wrong"),
            good.replace(&r.request_note, "ZZT_OTHER"),
            good.replace("|ok|", "|accepted|"),
            good.replace("@@END", ""),
            good.replace("|ok|0", "|ok|zero"),
            good.replace("|1|", "|2|"),
        ] {
            assert!(decode_reply(&r, &bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn payload_delimiters_newlines_and_crlf_are_preserved() {
        let r = request();
        let body = "a|b\n@@END\nc";
        let reply = decode_reply(&r, &note(&r, body).replace('\n', "\r\n")).unwrap();
        assert_eq!(reply.text, body);
    }
    #[test]
    fn errors_are_not_successful_execution() {
        let r = request();
        let text = note(&r, "calculator evaluation failed").replace("|ok|0", "|error|-1");
        assert!(!decode_reply(&r, &text).unwrap().success);
    }
    #[test]
    fn listener_template_has_matching_names_and_parses() {
        // Syntax validation is covered in ppl-cli; this checks source injection
        // boundaries and the persistent receipt that precedes evaluation.
        assert!(crate::listener::source("BAD", "ABC123").is_err());
        assert!(crate::listener::source("ZZT_TEST", "bad\"xx").is_err());
        let s = crate::listener::source("ZZT_TEST", "ABC123").unwrap();
        assert!(s.contains("ZZT_QABC123"));
        assert!(s.find("Notes(zza):=").unwrap() < s.find("EXPR(").unwrap());
    }
}
