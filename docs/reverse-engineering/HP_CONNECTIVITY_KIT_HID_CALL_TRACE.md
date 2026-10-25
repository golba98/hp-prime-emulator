# Connectivity Kit HID call trace — 2026-10-25

This is static analysis of the retained PE32 ConnectivityKit.exe, SHA-256
`a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb`,
resource version **2.4.2 r15751**, preferred image base **0x400000**.
Addresses below are preferred VAs; RVA = VA − 0x400000. For code in
`.text`, file offset = VA − 0x400c00. These rules and the evidence manifest
make every address below relocatable and reproducible. No vendor code or USB
operation was executed. A static call to WriteFile proves software routing,
not calculator acceptance.

## Evidence and function records

The retained focused disassembly is
`scratch/research/autonomous-control-investigation/continuation-20261025/trace-v1/`.
Its `manifest.json` records input hash, section mappings, VA/RVA/file offsets,
and hashes of each evidence file. `tables.txt` contains imports, RTTI, Qt
metadata and dispatch tables. Reproduce into a **new** output directory:

```sh
python3 scripts/research/ck_trace_evidence.py \
  scratch/research/autonomous-control-investigation/ck/ConnectivityKit.exe \
  scratch/research/autonomous-control-investigation/continuation-20261025/trace-reproduction
```

Requires Python and Capstone (analysis used 5.0.7). This script reads the PE;
it does not load it. `ck_inspect.py` also exposes disassembly, pointer, import,
Qt metadata and RTTI inspection. Linear xref candidates require manual
instruction-boundary and control-flow validation; absence from that scan is
not proof that a computed call cannot exist.

The following function descriptions are reconstructed, not recovered symbols
except for RTTI class names. C++ member functions use x86 ECX `this` where
observed; exact source prototypes are unavailable. Confidence is high for
listed call edges and field accesses, conditional for descriptor values and
firmware behavior.

| VA / RVA | Purpose, inputs and return | Established callers → callees / evidence |
|---|---|---|
| 414300 / 14300 | Enumeration; cdecl-style helper returns device records | 4d02c0 → SetupDi/HidD helpers; `hid-enumeration.asm` |
| 4146b0 / 146b0 | Enumeration-record factory, ECX record → transport | Selected record virtual call → CusbHid_win construction |
| 414b30 / 14b30 | Transport constructor, record → initialized object | Also inlined by factory; copies handle, input/output sizes |
| 414820 / 14820 | thiscall write(buffer,length), boolean | Worker slot +8 → WriteFile/GetOverlappedResult; `hid-os-io.asm` |
| 414a50 / 14a50 | thiscall write(buffer,length,reportID), boolean | Transport slot +4; alternate explicit report-ID writer |
| 414900 / 14900 | thiscall read(destination,length,wait_ms), count/0/−1 | Worker slot +c → ReadFile/GetOverlappedResult/WaitForSingleObject |
| 4147a0 / 147a0 | Transport close/cancel/free | Teardown/destruction → CancelIoEx, CloseHandle |
| 4c6440 / c6440 | Physical G1 client factory, transport record | Registered VID/PID match → 4c3e40 |
| 4c3e40 / c3e40 | Client constructor, allocates worker context | Factory → thread launcher 415b10 with entry 4c5270 |
| 4d1560 / d1560 | Client enqueue, message → success | Endpoint virtual +38 → queue 419460 and worker signal |
| 4c5270 / c5270 | Worker entry(context), runs transport loop | Constructor thread → transport virtual reads/writes, 4c9b00 |
| 4c9b00 / c9b00 | Received-report handler(context,report), success/failure | Worker → framing and 4c6d60 |
| 4c6d60 / c6d60 | Logical-message assembler | Receive → complete-message dispatcher 4cb930 |
| 4cb930 / cb930 | Completed-message routing and state update | Assembler → listeners through 4c7cc0, or incoming queue |
| 4cb220 / cb220 | Endpoint send(message), queued/routed result | UI callback and local engine replies → client virtual +38 |

RTTI resolves `CusbHid_win` vtable **ee6c0c** to the writers/read methods above.
`CIOObjectClientHID` vtable **f61c1c** resolves +38 to 4d1560 and +0 to
4c6900. G2 has a separate vtable, sharing substantial machinery. This is
stronger evidence than merely finding imported APIs.

## Enumeration, selection and opening

4d02c0 calls enumeration at 4d0383 and matches registered vendor/product
pairs at 4d03c0–4d03da. Records around eef7bc include `03f0:0441`,
`03f0:1541` and `03f0:2441`; 1541 selects factory 4c6440.

414300 calls HidD_GetHidGuid (thunk 4a2cfe), SetupDiGetClassDevsW,
SetupDiEnumDeviceInterfaces and SetupDiGetDeviceInterfaceDetailW. It opens a
metadata handle with CreateFileW at 41440c (access 0, sharing 3,
OPEN_EXISTING), obtains attributes at 414434, preparsed data at 414441,
HIDP_CAPS at 414457, and frees preparsed data at 414462. Selection opens the
working handle at **414532** with GENERIC_READ|GENERIC_WRITE, sharing 0,
OPEN_EXISTING and FILE_FLAG_OVERLAPPED.

Relevant IAT slots: ece010 FreePreparsedData, ece014 HidP_GetCaps,
ece018 GetPreparsedData, ece01c GetAttributes, ece020 GetHidGuid;
ed0740/744/748/74c SetupDi GetClassDevs/Enum/Destroy/Detail;
ece0c0 CreateFileW, ece0c4 ReadFile, ece0c8 WriteFile,
ece0cc CloseHandle, ece0d4 GetOverlappedResult, ece0d8 CancelIoEx.
No DeviceIoControl import appears in this inspected import table.

A different API cluster at 4a2d60/4a3070 uses disk-interface GUID
`53f56307-b6bf-11d0-94f2-00a0c91efb8b`, seeks and large block transfers.
It is not the selected ordinary HID worker. Likewise, 419xxx named-pipe I/O
must not be used to infer physical HID sizes.

## Report size at the actual OS boundary

HIDP_CAPS starts at stack ebp−88. At 414555/41455c the code loads
InputReportByteLength (+4) and OutputReportByteLength (+6), subtracts **one**
from each, and stores enumeration-record +18/+1c. Constructor 414b30 copies
output-minus-one to transport +4 and input-minus-one to +8.

| Layer | Length and leading bytes |
|---|---|
| Windows API buffer | Descriptor report length; leading Windows report ID |
| CusbHid_win caller buffer | Descriptor length −1; leading protocol sequence |
| Legacy logical fragment | Caller length −1; message bytes following sequence |
| Historical physical G1 record | 64-byte protocol report, including sequence; 63 message bytes |

Thus **if** the descriptor describes the preserved G1 64-byte protocol
reports, the Windows API buffer is **65 bytes**, including report ID 0.
This is a static derivation combined with historical Linux report evidence,
not a newly measured Windows HIDP_CAPS value. The application does not
hardcode 64 for all interfaces or firmware.

414820 builds `[00][caller bytes…]`, copies min(requested length, output
capacity), and calls **WriteFile at 41488f with output capacity +1**.
It therefore writes a full API report even for a short logical request.
The unused slack is not visibly cleared in this function; its contents are
not a protocol invariant. Preserve Rust's existing safe padding behavior.
The explicit-report-ID variant writes at 414ab5.

ReadFile at **41495b** requests input capacity +1 into transport buffer +4c.
The read method validates completed length and strips the report-ID byte
before copying to its caller (414a1b onward). It retains that ID separately
at transport +18.

## Verified outgoing chain

```text
Capture UI 4289e0 / automatic screen service 4c7580
  → FC builder 450320 / automatic builder 4cce00
  → endpoint selection 42c220, callback 4648c0 [manual builder]
  → endpoint send 4cb220
  → CIOObjectClientHID virtual +38 = 4d1560
  → outgoing queue +6c, queue insert 419460, worker notification
  → worker 4c5270 (dequeue 4c5480)
  → legacy fragment construction 4c5602–4c567a
  → CusbHid_win virtual +8 = 414820
  → WriteFile 41488f; GetOverlappedResult 4148ed
  → opened HID interface [physical acceptance not tested here]
```

Worker legacy mode resets sequence to 0 for a message, copies at most
transport output−1 message bytes after it, and advances sequence **FE→00**.
Completion releases message references and current queue ownership at
4c5753–4c579e. Internal messages have a 0x24-byte metadata prefix; only the
payload beginning at +24 is serialized. Allocation size is not report size.

## Receiving, queues and connection lifetime

```text
ReadFile 41495b → strip Windows report ID in 414900
  → worker 4c539e/4c53e1 → report handler 4c9b00
  → legacy sequence validation 4ca7af onward
  → strip sequence → assembler 4c6d60
  → six-byte header / BE body length / complete message
  → 4cb930 → listener virtual +10 via 4c7cc0, or incoming queue +60
```

Assembler accumulates headers and body across fragments. Protocol-specific
short messages exist, including outgoing FC's two bytes and F9's one byte;
these are not a universal six-byte-header rule for every direction. Incoming
legacy sequence mismatch discards partial state and returns failure to the
worker. Incoming sequence also wraps FE→00. Message padding is not appended
to the declared logical body.

Writes handle ERROR_IO_PENDING (997), then wait in GetOverlappedResult with
bWait=1. There is no bounded write timeout established in that method.
Read polls completion with bWait=0, preserves pending state for
ERROR_IO_INCOMPLETE (996), and can WaitForSingleObject for the caller's
wait_ms. Worker calls read with wait 0. Failure cancels/closes the handle.
4147a0 cancels outstanding operations, waits for active I/O flags, closes
both events and frees buffers/path. Client close 4c6900 suspends its worker;
worker cleanup releases transport ownership. This does not establish an
application-level retry for failed legacy screenshots.

## Negotiated transport: separate from application EC

Worker startup writes `[00 FA]`. The info handler at 4c9b00 has a conditional
negotiation branch: advertised value >=0x2915 plus configuration
111b08c ==1 leads to a control report beginning **`FF EC 00 00 00 00`**
(4c9dd3 onward). Here FF identifies a control report; EC is **not** an
application key opcode. Incoming control reports select mode 1 or 2
(4c9ec2–4c9f36). The alternative worker branches contain message IDs,
length/acknowledgment handling (FE), sequence 1 for a first header and
2…FD thereafter, a 76-report window in mode 1, 500 ms retry logic and a
0xc00 ms mode-2 wait. See `hid-worker.asm` and `hid-receive.asm` for the
branches rather than applying legacy framing to them.

Whether this configuration is enabled for a G1 running 15157, which mode it
negotiates, and its actual throughput remain unmeasured. Do not copy this
handshake into Rust or send it as a remote-input experiment. A Windows
capture must first establish the branch actually used.
