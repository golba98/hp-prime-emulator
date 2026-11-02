# Firmware Run benchmark

`benchmark_run.py` exercises the framed helper protocol with an isolated copy of initialized calculator storage. It never opens USB or edits the supplied profile. Firmware resources must be provided by the user.

```sh
python3 scripts/emulator/benchmark_run.py \
  --helper /absolute/path/to/prime-emulator \
  --assets "$HOME/.local/share/hp-prime-emulator/firmware" \
  --output /tmp/prime-run-after
```

The output directory must not exist. `--initial-data PATH` selects an alternative initialized profile to copy. Keep the assets, initial profile, script and workloads identical when comparing helpers. Copy each built helper to a distinct path before rebuilding another revision. Run on a quiet machine; concurrent tests and builds affect timings.

The workloads cover tiny, mathematical, medium and 10,160-byte successful programs, unchanged reruns, edited-source Check then Run, alternating sources, a syntax error, recovery after the error, and two runs after a normal OFF/restart. `large_cold` means the first large-source submission in an already booted session; `tiny_cold` includes initial loader preparation. Boot is measured separately. All expected values and rejection verdicts are checked, and an unexpected result causes a nonzero exit status.

`results.json` contains request-to-Result wall times, Linux helper-process user/system CPU time when available, source and executable hashes, verdicts, and observed pipeline timings. Each session directory retains source fixtures, stage reports and stderr. This measures the helper pipeline, excluding IDE dispatch/rendering latency. It does not measure emulated instruction counts or establish physical hardware parity.

## Loader format evidence

The optimized loader uses the documented `AFiles(name)` object reader instead of interpreted `AFilesB` byte conversion. HP describes both interfaces in its [Prime User Guide](https://h10032.www1.hp.com/ctg/Manual/c05332710.pdf). It still uses `Programs("ZZT_IDE"):=source` for genuine firmware compilation and storage.

Local G1 2.4.2 firmware experiments wrote strings using `Function.AFiles`, then read and rewrote host-created equivalents. The serialized dynamic string has little-endian tag `0x0012FFFF`, a 32-bit UTF-16 unit count, UTF-16LE text and a terminating zero unit. A firmware string literal can have transient tag flags `0x30`; reading and rewriting normalizes them to zero. The implementation emits the normalized form.

Round trips preserved Greek text, an emoji surrogate pair, newlines, quotes and backslashes. The live integration test covers Unicode program source, 10 KB import, execution and both old-loader migrations. Source hashes observed inside the genuine compiler and the firmware-written program source must still match exactly before Run succeeds. These are firmware-only checks, with no physical-device claim.
