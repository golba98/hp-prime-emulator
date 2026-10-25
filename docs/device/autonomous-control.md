# Physical HP Prime control foundation

The reference G1 (`03f0:1541`, build 15157, boot `V0.050.640`) supports physical screenshots, exact program reads, and the existing guarded object operations. **Remote navigation, Check invocation, Run, and automatic recovery are not verified.** Their new CLI interfaces refuse with `unsupported_capability`, exit 6, before uploading or injecting input. No emulator result is substituted for physical execution.

Examples assume a freshly built `ppl`. Build with `scripts/emulator/run.sh build --bin ppl`; this checkout places the binary at `../.cache/prime-emulator-target/debug/ppl`.

## Commands

```sh
ppl device status --json
ppl device programs list --json
ppl device programs show FIREWORKS --json
ppl device programs export FIREWORKS --out /tmp/FIREWORKS.hpprgm --json
ppl device ui inspect --json
```

`programs list` receives the documented read-only F9 backup stream. It currently returns a verified subset and `inventory_incomplete`, exit 9; it does not promise all programs. Names and payload sizes come only from CRC-valid returned objects, never upload receipts. See [inventory](program-inventory.md).

These interfaces are present but gated:

```sh
ppl device ui home --json
ppl device ui programs --json
ppl device ui up --json
ppl device ui down --json
ppl device ui esc --json
ppl device ui enter --json
ppl device ui select FIREWORKS --json
ppl device ui edit FIREWORKS --json
ppl device ui check FIREWORKS --dismiss --json
ppl device run-file examples/FIREWORKS.hpprgm --json
```

The gates are compiled from actual verification scope, not a user-editable success flag. Existing legacy key/session research commands remain separate and unverified; their ability to transmit EC does not enable these commands. A successful USB write is not a verified UI action.

## One exclusive USB owner

On Unix, start an explicit foreground service in another terminal or a host-managed subprocess:

```sh
ppl device control serve --socket "$XDG_RUNTIME_DIR/ppl-control/device.sock"
ppl device --control-socket "$XDG_RUNTIME_DIR/ppl-control/device.sock" control status --json
ppl device --control-socket "$XDG_RUNTIME_DIR/ppl-control/device.sock" programs show FIREWORKS --json
ppl device --control-socket "$XDG_RUNTIME_DIR/ppl-control/device.sock" ui inspect --json
ppl device --control-socket "$XDG_RUNTIME_DIR/ppl-control/device.sock" mirror
ppl device --control-socket "$XDG_RUNTIME_DIR/ppl-control/device.sock" control stop --json
```

The immediate parent is created as 0700 if absent; existing directories must be private, owned by the current user, and not symlinks. The socket is 0600. An existing socket is never unlinked automatically. Startup progress goes to stderr; the foreground process prints its terminal result on shutdown. There is no background autostart or TCP endpoint.

One owner retains `Prime` and its existing advisory USB lock. Requests are serialized. Screenshot streaming uses the existing validated decoder and exchange implementation; ownership passes to another operation only at a complete response boundary. The mirror retains its rendering, latest-frame behavior, raw-image diagnostics, and USB timings. Standalone mirror behavior is unchanged.

A direct cooperating command fails busy while the owner holds USB. Use the socket for supported operations, or stop the service before existing standalone upload/deletion/mailbox operations. Unsupported socket operations refuse instead of opening another connection. Local firmware diagnosis never needs this service.

The initial service is read-only: status, screenshots, exact program reads, backup inventory, and host service stop. There is no raw-packet, key, upload, deletion, or execution RPC. It cannot override existing mutation authorization. After inventory, the service deliberately retires the connection because backup termination is not verified on this firmware; stop and restart the service before further device operations.

There is no automatic reconnect or replay after ambiguous I/O. Stop closes the host service; it does not interrupt a calculator program or dismiss a calculator dialog. Connectivity Kit and unrelated tools do not honor this project's advisory lock and must remain closed.

## Screen evidence and OCR

`ui inspect` takes two fresh full-resolution FC 08 captures and retains the original raw PNG, decoded RGB PNG, and inspection JSON in a unique private artifact directory. Decoding validates CRC, format, dimensions, and image structure. Format 09 palettes are not assumed equivalent to RGB565.

Optional local Tesseract with English language data can extract text. Install it separately or supply `--ocr /path/to/tesseract`. The executable receives an enlarged derivative of the original image, runs without a shell or network dependency, and has a five-second deadline per image. TSV output is bounded and retained.

Recognition is deliberately narrow: the observed English G1 Program Catalog title and the observed syntax-dialog layout/text. Action-critical OCR tokens require a confidence score of at least 90/100; those are heuristic scores, not calibrated probabilities. Two captures must agree. A modal must not inherit the Catalog title behind it. Other languages, layouts, runtime dialogs, confirmations, editors, running programs, missing OCR, and uncertain text remain unknown until supported by validated evidence.

A syntax observation reports `source: physical_screen_ocr`, nullable column, and unverified program attribution. Merely inspecting a pre-existing dialog leaves `check_executed: false`. Recognition never enables a navigation capability. The agent can inspect preserved original screenshots independently when OCR is unavailable.

## JSON and recovery

New operational responses use schema version 1 and one JSON object on stdout, including failures. Fields separate upload, navigation, Check, observed execution, and completion. `physical_g1` and `firmware_emulator` are distinct engines. Device provenance uses the existing cryptographic serial fingerprint; raw FA serial fields and whole backup contents are not emitted.

Exit 0 means the requested supported operation succeeded. Exit 1 is a genuine firmware syntax rejection; 2 is invalid input/refusal; 5 is device/service/configuration I/O failure; 6 is unsupported capability; 7 is timeout; 8 is busy; 9 is incomplete inventory, uncertain diagnosis, or invalid evidence; 130 is local firmware cancellation. Inspect the structured error and evidence, not just exit status.

On ambiguous input or execution outcomes, a future verified controller must stop, retain its journal, and allow read-only observation only. This implementation has no physical input backend, so it never initiates those uncertain operations. See [physical debugging](physical-debugging.md) and [input investigation](../reverse-engineering/remote-input-investigation.md).
