//! Experimental G1 PPL mailbox listener. Hardware support is determined by
//! challenge/response, never by a successful upload.
use crate::{Result, validate_test_name};

pub const VERSION: u32 = 1;
pub const MAX_REQUEST_CHARS: usize = 4096;
pub const MAX_RESULT_CHARS: usize = 32768;

/// Consume queued taps as well as held Esc, while retaining a one-second
/// idle period made of short sleeps. GETKEY preserves taps released between
/// polls; ISKEYDOWN alone cannot do that.
pub fn stop_helper(session: &str) -> Result<(String, String)> {
    if session.is_empty()
        || session.len() > 10
        || !session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(crate::DeviceError::Refused(
            "invalid stop-helper identifier".into(),
        ));
    }
    let name = format!("ZSTOP_{session}");
    let source = format!(
        r#"{name}()
BEGIN
  LOCAL zzkey,zzstep;
  FOR zzstep FROM 1 TO 20 DO
    REPEAT
      zzkey:=GETKEY;
      IF zzkey==4 THEN RETURN 1; END;
    UNTIL zzkey==-1;
    IF ISKEYDOWN(4) THEN RETURN 1; END;
    WAIT(0.05);
  END;
  RETURN 0;
END;
"#
    );
    Ok((name, source))
}

pub fn source(program: &str, session: &str) -> Result<String> {
    validate_test_name(program)?;
    if session.len() != 6 || !session.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(crate::DeviceError::Refused(
            "invalid listener session".into(),
        ));
    }
    let (stop_name, stop_body) = stop_helper(session)?;
    // Request: P|<32 uppercase hex nonce>|<challenge>, E|<nonce>|<expression>.
    // Each request owns a receipt (A) and terminal result (R). A receipt
    // prevents re-execution even if the listener is interrupted or restarted.
    Ok(format!(
        r#"{stop_body}
EXPORT {program}()
BEGIN
  LOCAL zzns,zzj,zzk,zzq,zza,zzr;
  LOCAL zzs,zzh,zzn,zzv,zzt,zzok;
  LOCAL zzseen,zzdone,zzpayload,zzop,zzvalid;
  LOCAL zzstage,zzhandled,zzcount,zzidle;
  REPEAT zzv:=GETKEY; UNTIL zzv==-1;
  zzstage:="startup"; zzhandled:={{}}; zzcount:=-1; zzidle:=0;
  Notes("ZZT_H{session}"):="STARTED";
  RECT_P(0,0,320,240,16777215);
  TEXTOUT_P("Remote listener active",12,16,3,0);
  TEXTOUT_P("Esc stops this foreground program",12,48,1,0);
  IFERR
  REPEAT
    zzstage:="catalog";
    zzns:=Notes; zzidle:=zzidle+1;
    IF SIZE(zzns)<>zzcount OR zzidle>=10 THEN
      IF zzcount==-1 THEN
        Notes("ZZT_H{session}"):="READY|notes="+STRING(SIZE(zzns));
      END;
      zzcount:=SIZE(zzns); zzidle:=0;
    FOR zzj FROM 1 TO SIZE(zzns) DO
      zzstage:="name"; zzq:=zzns(zzj);
      IF DIM(zzq)==15 AND LEFT(zzq,11)=="ZZT_Q{session}" AND POS(zzhandled,zzq)==0 THEN
        zzhandled:=CONCAT(zzhandled,{{zzq}});
        zza:="ZZT_A"+MID(zzq,6,10);
        zzr:="ZZT_R"+MID(zzq,6,10);
        zzseen:=POS(zzns,zza); zzdone:=POS(zzns,zzr);
        IF zzseen==0 AND zzdone==0 THEN
          zzstage:="payload"; zzs:=Notes(zzq); zzvalid:=1;
          IF DIM(zzs)<35 OR DIM(zzs)>{max_request} THEN zzvalid:=0; END;
          IF zzvalid THEN
            zzstage:="nonce"; zzn:=MID(zzs,3,32);
            FOR zzk FROM 1 TO 32 DO
              IF INSTRING("0123456789ABCDEF",MID(zzn,zzk,1))==0 THEN zzvalid:=0; END;
            END;
            IF MID(zzs,2,1)<>"|" OR MID(zzs,35,1)<>"|" THEN zzvalid:=0; END;
            zzop:=LEFT(zzs,1);
            IF zzop<>"P" AND zzop<>"E" THEN zzvalid:=0; END;
          END;
          IF zzvalid THEN
            zzstage:="receipt"; Notes(zza):="@@PPLREMOTE|1|"+zzq+"|"+zzn+"|accepted";
            zzpayload:=MID(zzs,36,DIM(zzs)-35);
            zzok:=1; zzt:=-1;
            IF zzop=="P" THEN
              zzv:="PONG:"+zzpayload; zzt:=2;
            ELSE
              IFERR
                zzstage:="eval"; zzv:=EXPR(zzpayload); zzt:=TYPE(zzv); zzv:=STRING(zzv);
              THEN
                zzok:=0; zzv:="calculator evaluation failed";
              END;
            END;
            IF DIM(zzv)>{max_result} THEN zzok:=0; zzv:="result exceeds limit"; END;
            zzh:="@@PPLREMOTE|1|"+zzq+"|"+zzn+"|";
            IF zzok THEN zzh:=zzh+"ok|"; ELSE zzh:=zzh+"error|"; END;
            zzstage:="result"; Notes(zzr):=zzh+STRING(zzt)+CHAR(10)+zzv+CHAR(10)+"@@END";
          END;
        END;
      END;
    END;
    END;
    zzstage:="wait";
  UNTIL {stop_name}();
  THEN
    Notes("ZZT_H{session}"):="ERROR|"+zzstage;
    RETURN -1;
  END;
  Notes("ZZT_H{session}"):="STOPPED";
  RETURN 0;
END;
"#,
        max_request = MAX_REQUEST_CHARS,
        max_result = MAX_RESULT_CHARS
    ))
}
