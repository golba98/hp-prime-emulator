#!/usr/bin/env python3
"""Benchmark real firmware Run through the framed session protocol.

Requires user-supplied firmware. Never accesses USB or an existing live profile.
Run the same script/helper revisions on a quiet machine for meaningful comparisons.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import queue
import shutil
import struct
import subprocess
import threading
import time


def cpu_seconds(pid):
    """Linux process CPU time; None on other hosts or unavailable processes."""
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")
    except (OSError, ValueError, IndexError):
        return None


def profile_sha256(directory):
    """Bind comparisons to the same initial data, excluding host timestamps."""
    digest = hashlib.sha256()
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"initial profile contains a symlink: {path}")
        name = path.relative_to(directory).as_posix().encode()
        digest.update(struct.pack("<I", len(name)))
        digest.update(name)
        if path.is_dir():
            digest.update(b"D")
        elif path.is_file():
            digest.update(b"F")
            digest.update(struct.pack("<Q", path.stat().st_size))
            with path.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(chunk)
        else:
            raise ValueError(f"initial profile contains a nonregular entry: {path}")
    return digest.hexdigest()


def copy_initial_profile(source, destination):
    expected = profile_sha256(source)
    # Preserve unexpected links for validation, rather than following them if
    # the source tree changes during the copy.
    shutil.copytree(source, destination, symlinks=True)
    copied = profile_sha256(destination)
    if copied != expected:
        raise ValueError("initial profile changed while preparing the benchmark")
    return copied


def host_identity():
    processor = platform.processor()
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                processor = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    return {"hostname": platform.node(), "machine": platform.machine(),
            "processor": processor, "logical_cpus": os.cpu_count()}


class Session:
    def __init__(self, helper, assets, profile, output):
        self.log = (output / "session.log").open("w")
        self.process = subprocess.Popen(
            [str(helper), "session", "--firmware", str(assets / "armfir.elf"),
             "--os-rom", str(assets / "PRIME_OS.ROM"), "--data-dir", str(profile)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log,
        )
        self.events = queue.Queue()
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def read_exact(self, count):
        result = bytearray()
        while len(result) < count:
            chunk = self.process.stdout.read(count - len(result))
            if not chunk:
                raise EOFError("firmware helper closed stdout")
            result.extend(chunk)
        return result

    def read(self):
        try:
            while True:
                header = self.read_exact(5)
                size = struct.unpack_from("<I", header)[0]
                if size > 32 * 1024 * 1024:
                    raise ValueError("invalid protocol packet size")
                data = self.read_exact(size)
                if header[4] == 1:
                    self.events.put(json.loads(data))
                # Drain frames continuously so rendering cannot block the helper.
        except (EOFError, OSError, ValueError) as error:
            self.events.put(error)

    def send(self, command):
        data = json.dumps(command).encode()
        self.process.stdin.write(struct.pack("<IB", len(data), 3) + data)
        self.process.stdin.flush()

    def wait(self, predicate, timeout=240):
        deadline = time.monotonic() + timeout
        while True:
            event = self.events.get(timeout=max(0, deadline - time.monotonic()))
            if isinstance(event, Exception):
                raise event
            if predicate(event):
                return event

    def shutdown(self):
        self.send({"command": "power_down"})
        self.wait(lambda event: event.get("event") == "closed", 60)
        self.process.wait(timeout=30)
        if self.process.returncode:
            raise RuntimeError(f"helper exited {self.process.returncode}")

    def close(self):
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait()
        self.reader.join(timeout=2)
        self.log.close()


def workloads():
    tiny = "EXPORT PERF()\nBEGIN\n RETURN 42;\nEND;\n"
    math = "EXPORT MATHPERF()\nBEGIN\n LOCAL n:=0,k; FOR k FROM 1 TO 100 DO n:=n+k; END; RETURN n;\nEND;\n"
    medium = "EXPORT MEDPERF()\nBEGIN\n" + "// medium fixture\n" * 100 + " RETURN 42;\nEND;\n"
    large = "EXPORT BIGPERF()\nBEGIN\n" + ("// " + "filler " * 12 + "\n") * 115 + " RETURN 42;\nEND;\n"
    edited = large.replace("RETURN 42", "RETURN 43")
    invalid = "EXPORT BAD()\nBEGIN\n RETURN 1+;\nEND;\n"
    return [
        ("tiny_cold", "run", tiny, 42),
        ("tiny_repeat", "run", tiny, 42),
        ("small_math", "run", math, 5050),
        ("medium", "run", medium, 42),
        ("large_cold", "run", large, 42),
        ("large_repeat", "run", large, 42),
        ("edited_check", "compile", edited, None),
        ("edited_run", "run", edited, 43),
        ("alternate_tiny", "run", tiny, 42),
        ("syntax_error", "run", invalid, False),
        ("after_error_tiny", "run", tiny, 42),
    ], tiny


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--helper", required=True, type=Path)
    parser.add_argument("--assets", type=Path, default=Path.home() / ".local/share/hp-prime-emulator/firmware")
    parser.add_argument("--initial-data", type=Path, help="initialized storage to copy; defaults to assets/initial-data")
    parser.add_argument("--output", required=True, type=Path, help="new evidence directory; must not already exist")
    parser.add_argument("--no-restart", action="store_true")
    args = parser.parse_args()
    helper = args.helper.resolve(strict=True)
    assets = args.assets.resolve(strict=True)
    initial = (args.initial_data or assets / "initial-data").resolve(strict=True)
    for item in [assets / "armfir.elf", assets / "PRIME_OS.ROM", initial / "A", initial / "C"]:
        if not item.exists():
            parser.error(f"missing resource: {item}")
    args.output.mkdir(parents=True, exist_ok=False)
    profile = args.output / "profile"
    initial_identity = copy_initial_profile(initial, profile)
    cases, tiny = workloads()
    metadata = {
        "helper": str(helper), "helper_sha256": hashlib.sha256(helper.read_bytes()).hexdigest(),
        "firmware_sha256": hashlib.sha256((assets / "armfir.elf").read_bytes()).hexdigest(),
        "os_rom_sha256": hashlib.sha256((assets / "PRIME_OS.ROM").read_bytes()).hexdigest(),
        "platform": platform.platform(), "clock": "time.monotonic request write to matching Result event",
        "host_identity": host_identity(),
        "cpu_clock": "/proc/pid/stat user+system ticks when available", "samples_per_case": 1,
        "verification": "local firmware only; no physical calculator",
        "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "initial_profile_sha256": initial_identity,
        "boot_seconds": [], "results": [],
    }
    request = 0
    phases = [cases]
    if not args.no_restart:
        phases.append([("tiny_after_restart", "run", tiny, 42), ("repeat_after_restart", "run", tiny, 42)])
    try:
        for phase, cases in enumerate(phases):
            directory = args.output / f"session-{phase}"
            directory.mkdir()
            started = time.monotonic()
            session = Session(helper, assets, profile, directory)
            try:
                session.wait(lambda event: event.get("state") == "ready")
                metadata["boot_seconds"].append(time.monotonic() - started)
                for name, command, source, expected in cases:
                    request += 1
                    (directory / f"{name}.hpprgm").write_text(source)
                    payload = {"command": command, "request": request, "source": source}
                    if command == "run":
                        payload.update(entry=None, arguments="")
                    cpu_before = cpu_seconds(session.process.pid)
                    started = time.monotonic()
                    wall_started = time.time_ns()
                    session.send(payload)
                    result = session.wait(lambda event: event.get("event") == "result" and event.get("request") == request)
                    elapsed = time.monotonic() - started
                    cpu_after = cpu_seconds(session.process.pid)
                    correct = result.get("success") is (expected is not False)
                    if expected is not None and expected is not False:
                        correct &= f"returned {expected}" in result.get("message", "")
                    if expected is False:
                        correct &= result.get("check", {}).get("accepted") is False
                    record = {
                        "case": name, "request": request, "elapsed_seconds": elapsed,
                        "cpu_seconds": None if cpu_before is None or cpu_after is None else cpu_after - cpu_before,
                        "source_bytes": len(source.encode()), "source_sha256": hashlib.sha256(source.encode()).hexdigest(),
                        "correct": correct, "result": result,
                    }
                    report = profile / "ide-reports" / f"request-{request}.json"
                    if report.exists() and report.stat().st_mtime_ns >= wall_started:
                        shutil.copyfile(report, directory / f"{name}.json")
                        record["timeline"] = json.loads(report.read_text()).get("timeline", [])
                    metadata["results"].append(record)
                    print(json.dumps({key: value for key, value in record.items() if key != "timeline"}), flush=True)
                session.shutdown()
            finally:
                session.close()
    finally:
        (args.output / "results.json").write_text(json.dumps(metadata, indent=2))
    return 0 if all(result["correct"] for result in metadata["results"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())
