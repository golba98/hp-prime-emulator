"""Relocate a legacy GUI scenario's outputs into a new owned run directory."""

import json
import sys
from pathlib import Path


def relocate(source, destination):
    data = json.loads(source.read_text())
    old = Path(data["dump"]).parent
    if not old.is_absolute() or Path("/tmp/ppl-ide-e2e") not in old.parents:
        raise ValueError("scenario dump is outside the legacy scratch root")
    prefix = str(old) + "/"
    target = str(destination.parent.resolve()) + "/"

    def rewrite(value):
        if isinstance(value, str) and value.startswith(prefix):
            relative = Path(value[len(prefix) :])
            if ".." in relative.parts:
                raise ValueError("output path escapes the owned run directory")
            return target + str(relative)
        if isinstance(value, list):
            return [rewrite(v) for v in value]
        if isinstance(value, dict):
            return {k: rewrite(v) for k, v in value.items()}
        return value

    relocated = rewrite(data)
    with destination.open("x") as stream:
        json.dump(relocated, stream, indent=2)
        stream.write("\n")
    return relocated


if __name__ == "__main__":
    relocate(Path(sys.argv[1]), Path(sys.argv[2]))
