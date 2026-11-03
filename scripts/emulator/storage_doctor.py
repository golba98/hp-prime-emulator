#!/usr/bin/env python3
"""Read-only project storage inventory; never removes or adopts resources."""

import argparse
import json
import os
import stat
from pathlib import Path


def discover(repo, home):
    roots = [repo]
    for parent in [repo.parent / ".cache", home / ".cache"]:
        if parent.is_dir():
            roots.extend(
                p
                for p in sorted(parent.iterdir())
                if p.name.startswith(("prime-", "ppl-", "zzt-"))
            )
    roots.extend(
        [
            home / ".cache/hp-prime-emulator",
            home / ".cache/.cache/prime-emulator-target",
            home / "prime-emulator-backups",
            home / ".local/share/hp-prime-emulator",
            home / ".local/share/hpprimepplemulator&nativeide",
        ]
    )
    for parent, prefixes in [
        (Path("/var/tmp"), ("hp-prime-",)),
        (Path("/tmp"), ("ppl-", "prime-", "zzt-", "ZZT_")),
    ]:
        if parent.is_dir():
            roots.extend(
                p
                for p in sorted(parent.iterdir())
                if p.name.startswith(prefixes)
            )
    return [p for p in roots if p.exists() and not p.is_symlink()]


def classify(root):
    # Names aid discovery; Cargo fingerprints establish project identity.
    fingerprints = any(root.glob("*/.fingerprint/ppl-*")) or any(
        root.glob("*/.fingerprint/prime-emulator-*")
    )
    if fingerprints:
        return (
            "B",
            "Cargo fingerprints; preserve referenced binaries before rebuild",
        )
    if root.name == "prime-emulator-backups":
        return "A", "Protected backup"
    if root.parent.name == "share" and root.name in (
        "hp-prime-emulator",
        "hpprimepplemulator&nativeide",
    ):
        return "A", "User firmware, saves and IDE state"
    return "D", "Mixed/protected or ownership/reference review required"


def scan(roots):
    roots = sorted(
        set(Path(p).absolute() for p in roots),
        key=lambda p: (len(p.parts), str(p)),
    )
    selected = []
    for p in roots:
        if not any(p == r or r in p.parents for r in selected):
            selected.append(p)
    seen = set()
    files = []
    directories = []
    errors = []
    summaries = []
    links = 0
    for root in selected:
        try:
            rs = root.lstat()
            if stat.S_ISLNK(rs.st_mode):
                errors.append(
                    {"path": str(root), "reason": "symlink root refused"}
                )
                continue
        except OSError as e:
            errors.append({"path": str(root), "reason": str(e)})
            continue
        subtotal = {}
        raw = 0
        count = 0
        walk = (
            os.walk(
                root,
                topdown=True,
                followlinks=False,
                onerror=lambda e: errors.append({"reason": str(e)}),
            )
            if root.is_dir()
            else [(str(root.parent), [], [root.name])]
        )
        order = []
        for parent, children, names in walk:
            allowed = []
            for name in children:
                child = Path(parent) / name
                try:
                    cs = child.lstat()
                    if stat.S_ISDIR(cs.st_mode) and cs.st_dev == rs.st_dev:
                        allowed.append(name)
                    elif stat.S_ISDIR(cs.st_mode):
                        errors.append(
                            {
                                "path": str(child),
                                "reason": "different VFS device skipped",
                            }
                        )
                except OSError as e:
                    errors.append({"path": str(child), "reason": str(e)})
            children[:] = allowed
            order.append((parent, list(children)))
            allocated = logical = 0
            for name in names:
                p = Path(parent) / name
                try:
                    s = p.lstat()
                except OSError as e:
                    errors.append({"path": str(p), "reason": str(e)})
                    continue
                if not stat.S_ISREG(s.st_mode):
                    continue
                if s.st_dev != rs.st_dev:
                    errors.append(
                        {
                            "path": str(p),
                            "reason": "different VFS device skipped",
                        }
                    )
                    continue
                raw += s.st_blocks * 512
                key = (s.st_dev, s.st_ino)
                if key in seen:
                    links += 1
                    continue
                seen.add(key)
                count += 1
                allocated += s.st_blocks * 512
                logical += s.st_size
                files.append(
                    {
                        "path": str(p),
                        "allocated_bytes": s.st_blocks * 512,
                        "logical_bytes": s.st_size,
                        "links": s.st_nlink,
                        "device": s.st_dev,
                        "inode": s.st_ino,
                        "sparse_candidate": s.st_blocks * 512 < s.st_size,
                    }
                )
            subtotal[parent] = (allocated, logical)
        for parent, children in reversed(order):
            allocated, logical = subtotal[parent]
            for child in children:
                child_allocated, child_logical = subtotal.pop(
                    str(Path(parent) / child), (0, 0)
                )
                allocated += child_allocated
                logical += child_logical
            subtotal[parent] = (allocated, logical)
            if allocated >= 10**9:
                directories.append(
                    {
                        "path": parent,
                        "allocated_bytes": allocated,
                        "logical_bytes": logical,
                    }
                )
        allocated, logical = subtotal.get(
            str(root if root.is_dir() else root.parent), (0, 0)
        )
        category, reason = (
            classify(root) if root.is_dir() else ("D", "Unclassified file")
        )
        summaries.append(
            {
                "path": str(root),
                "allocated_bytes": allocated,
                "logical_bytes": logical,
                "raw_entry_allocation_bytes": raw,
                "files": count,
                "device": rs.st_dev,
                "category": category,
                "reason": reason,
            }
        )
    return {
        "schema_version": 1,
        "mode": "read_only",
        "units": "bytes",
        "accounting": (
            "unique (st_dev, st_ino), st_blocks*512; "
            "NOT exclusive compressed physical usage"
        ),
        "allocated_bytes": sum(r["allocated_bytes"] for r in summaries),
        "logical_bytes": sum(r["logical_bytes"] for r in summaries),
        "physical_exclusive_bytes": None,
        "physical_uncertainty": (
            "Btrfs compression, reflinks and external snapshot references "
            "require privileged accounting"
        ),
        "hardlink_aliases_avoided": links,
        "roots": summaries,
        "directories_over_1GB": sorted(
            directories, key=lambda x: -x["allocated_bytes"]
        ),
        "largest_20_files": sorted(files, key=lambda x: -x["allocated_bytes"])[
            :20
        ],
        "largest_20_sparse_candidates": sorted(
            (f for f in files if f["sparse_candidate"]),
            key=lambda f: -(f["logical_bytes"] - f["allocated_bytes"]),
        )[:20],
        "sparse_candidate_note": (
            "st_blocks below logical length; compression or holes require "
            "extent inspection to distinguish"
        ),
        "largest_10_roots": sorted(
            summaries, key=lambda x: -x["allocated_bytes"]
        )[:10],
        "nand_bases": [f for f in files if Path(f["path"]).name == "base.raw"],
        "errors": errors,
        "deletion_authorized": False,
        "retained_pending_review_bytes": sum(
            r["allocated_bytes"]
            for r in summaries
            if r["category"] in ("A", "D")
        ),
        "rebuildable_cache_bytes": sum(
            r["allocated_bytes"] for r in summaries if r["category"] == "B"
        ),
        "confirmed_temporary_bytes": 0,
        "orphan_candidates": [],
        "orphan_note": (
            "No inactivity inferred from names or age. Owner release, "
            "references and open files need separate review."
        ),
        "category_allocation_bytes": {
            c: sum(
                r["allocated_bytes"] for r in summaries if r["category"] == c
            )
            for c in ["A", "B", "C", "D"]
        },
        "cleanup_preconditions": [
            "explicit path-scoped authorization",
            "reference and backing-chain preservation",
            "owner release",
            "fresh activity check; permission gaps are not inactivity",
            "pin validated binaries before retiring any target",
        ],
        "budgets_decimal_GB": {
            "minimal": 90,
            "standard": 100,
            "full_research": 115,
        },
        "budget_note": (
            "Planning ceilings AFTER approved cache retirement, including "
            "retained historical data; not physical-space guarantees"
        ),
    }


def markdown(d):
    lines = [
        "# Read-only storage inventory",
        "",
        d["accounting"],
        "",
        f"Allocation: {d['allocated_bytes'] / 1e9:.3f} GB; "
        f"logical: {d['logical_bytes'] / 1e9:.3f} GB; "
        "exclusive physical: unknown.",
        "",
        "| Category | Location | GB | Required? | Physical recovery |",
        "|---|---|---:|---|---|",
    ]
    for r in sorted(d["roots"], key=lambda x: -x["allocated_bytes"]):
        lines.append(
            f"| {r['category']} | {r['path']} | "
            f"{r['allocated_bytes'] / 1e9:.3f} | {r['reason']} | "
            "Unknown; approval required |"
        )
    lines += [
        "",
        f"Rebuildable cache: {d['rebuildable_cache_bytes'] / 1e9:.3f} GB; "
        "reclaimability requires reference and owner review.",
        f"Retained/protected or unknown: "
        f"{d['retained_pending_review_bytes'] / 1e9:.3f} GB.",
        "Confirmed disposable temporary data: 0; "
        "no orphanhood inferred automatically.",
    ]
    if "budget" in d:
        lines += [
            "",
            f"Budget: {d['budget']['limit_bytes'] / 1e9:.3f} GB; "
            f"exceeded: {d['budget']['exceeded']}; report only.",
        ]
    if "growth" in d:
        lines += ["", "| Growth GB | Location |", "|---:|---|"]
        lines += [
            f"| {r['delta_accounted_bytes'] / 1e9:+.3f} | {r['path']} |"
            for r in d["growth"][:10]
        ]
    return "\n".join(lines) + "\n"


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--root",
        type=Path,
        action="append",
        help="explicit roots; otherwise discover known project locations",
    )
    p.add_argument(
        "--budget-gb",
        type=int,
        help="allocation-accounting warning limit; never prunes",
    )
    p.add_argument(
        "--profile",
        choices=["minimal", "standard", "full_research"],
        default="standard",
    )
    p.add_argument(
        "--compare",
        type=Path,
        help="compare allocation with a prior report; never physical savings",
    )
    p.add_argument("--format", choices=["json", "markdown"], default="json")
    args = p.parse_args()
    repo = Path(__file__).resolve().parents[2]
    result = scan(args.root or discover(repo, Path.home()))
    limit = (
        args.budget_gb
        if args.budget_gb is not None
        else result["budgets_decimal_GB"][args.profile]
    )
    if limit <= 0:
        p.error("budget must be positive")
    result["budget"] = {
        "profile": args.profile,
        "limit_bytes": limit * 10**9,
        "exceeded": result["allocated_bytes"] > limit * 10**9,
        "action": "report_only",
    }
    if args.compare:
        st = args.compare.lstat()
        if not stat.S_ISREG(st.st_mode) or st.st_size > 4 * 1024 * 1024:
            p.error("baseline must be a bounded regular file")
        with args.compare.open("rb") as stream:
            previous = json.loads(stream.read(4 * 1024 * 1024 + 1))
        if previous.get("schema_version") != 1:
            p.error("unsupported baseline schema")
        old = {r["path"]: r["allocated_bytes"] for r in previous["roots"]}
        current = {r["path"]: r["allocated_bytes"] for r in result["roots"]}
        result["growth"] = sorted(
            [
                {
                    "path": path,
                    "delta_accounted_bytes": current.get(path, 0)
                    - old.get(path, 0),
                }
                for path in sorted(old.keys() | current.keys())
            ],
            key=lambda x: -x["delta_accounted_bytes"],
        )
    print(
        markdown(result)
        if args.format == "markdown"
        else json.dumps(result, indent=2)
    )


if __name__ == "__main__":
    main()
