"""Immutable synthetic NAND bases and verified QEMU runtime bundles."""

import hashlib
import json
import os
import shutil
import tempfile
from pathlib import Path

PAGE, OOB, PAGES = 2048, 64, 131072
IMAGE_SIZE = (PAGE + OOB) * PAGES


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for b in iter(lambda: f.read(1024 * 1024), b""):
            h.update(b)
    return h.hexdigest()


def chunks(bootstrap, stage):
    if len(bootstrap) > 8192 or len(stage) > PAGE:
        raise ValueError("synthetic bootstrap/stage outside geometry")
    prefix = bytearray(b"\xff" * ((PAGE + OOB) * 5))
    for offset in range(0, len(bootstrap), PAGE):
        position = (offset // PAGE) * (PAGE + OOB)
        prefix[
            position : position + len(bootstrap[offset : offset + PAGE])
        ] = bootstrap[offset : offset + PAGE]
    prefix[4 * (PAGE + OOB) : 4 * (PAGE + OOB) + len(stage)] = stage
    yield prefix
    remaining = IMAGE_SIZE - len(prefix)
    block = b"\xff" * (1024 * 1024)
    while remaining:
        size = min(remaining, len(block))
        yield block[:size]
        remaining -= size


def nand_base(output, bootstrap=b"", stage=b"", cache=None):
    if (output / "base.raw").exists() or (output / "base.raw").is_symlink():
        raise FileExistsError(output / "base.raw")
    cache = (
        Path(cache)
        if cache is not None
        else Path(
            os.environ.get(
                "PPL_RESEARCH_CACHE",
                str(Path.home() / ".cache/hp-prime-emulator/research"),
            )
        )
        / "nand"
    )
    h = hashlib.sha256()
    for b in chunks(bootstrap, stage):
        h.update(b)
    identity = h.hexdigest()
    cache.mkdir(parents=True, exist_ok=True)
    if cache.is_symlink():
        raise ValueError("cache directory must not be a symlink")
    blob = cache / (identity + ".raw")
    if not blob.exists():
        with tempfile.NamedTemporaryFile(dir=cache, prefix="new-base-") as f:
            for b in chunks(bootstrap, stage):
                f.write(b)
            f.flush()
            os.fsync(f.fileno())
            os.fchmod(f.fileno(), 0o400)
            try:
                os.link(f.name, blob)
            except FileExistsError:
                pass
    if (
        blob.is_symlink()
        or not blob.is_file()
        or blob.stat().st_size != IMAGE_SIZE
        or digest(blob) != identity
    ):
        raise ValueError(
            "NAND cache identity differs; existing blob left untouched"
        )
    # Keep the relative qcow2 backing name; never share writable overlays.
    destination = output / "base.raw"
    try:
        os.link(blob, destination)
    except OSError as error:
        import errno

        if error.errno != errno.EXDEV:
            raise
        with blob.open("rb") as source, destination.open("xb") as target:
            shutil.copyfileobj(source, target)
            os.fchmod(target.fileno(), 0o400)
    return identity


def runtime_bundle(build, output):
    from full_system_run import checked_manifest

    meta = checked_manifest(build.resolve())
    recipe = {
        k: meta.get(k)
        for k in [
            "reference_revision",
            "dtc_revision",
            "patches",
            "configure_flags",
            "containerfile_sha256",
            "image_id",
        ]
    }
    meta["storage_bundle"] = {
        "source_build": str(build),
        "compiled": False,
        "recipe_sha256": hashlib.sha256(
            json.dumps(recipe, sort_keys=True).encode()
        ).hexdigest(),
    }
    output.mkdir(parents=True, exist_ok=False)
    binary = output / "build/arm-softmmu/qemu-system-arm"
    binary.parent.mkdir(parents=True)
    with (
        (build / "build/arm-softmmu/qemu-system-arm").open("rb") as source,
        binary.open("xb") as target,
    ):
        shutil.copyfileobj(source, target)
    binary.chmod(0o500)
    if digest(binary) != meta["binary_sha256"]:
        raise ValueError("runtime copy identity changed")
    (output / "build.json").write_text(json.dumps(meta, indent=2) + "\n")
    source = build / "source"
    for name in ["COPYING", "LICENSE"]:
        original = source / name if (source / name).is_file() else build / name
        if original.is_file():
            shutil.copyfile(original, output / name)
    for name in ["compiler.txt", "toolchain-packages.txt", "configure.log"]:
        original = build / "build" / name
        if not original.is_file():
            original = build / name
        if original.is_file():
            shutil.copyfile(original, output / name)
    return meta


if __name__ == "__main__":
    import argparse

    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--build-root", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    print(json.dumps(runtime_bundle(a.build_root, a.output), indent=2))
