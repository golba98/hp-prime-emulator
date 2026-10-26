#!/usr/bin/env python3
"""Offline PE32/PE32+ inspection. Never loads or executes the inspected image.

Disassembly requires Capstone. Xrefs are linear-disassembly candidates, not a
complete call graph; pointer matches are raw bytes and may be data or code.
Input/output addresses are preferred-image virtual addresses.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
import sys
from collections.abc import Iterable, Iterator, Sequence
from pathlib import Path
from typing import TYPE_CHECKING, TypedDict

if TYPE_CHECKING:
    from capstone import Cs


class Section(TypedDict):
    name: str
    va: int
    virtual_size: int
    size: int
    offset: int
    flags: int


class PE:
    """Read-only PE image shared by inspection and pinned trace generation.

    Section and import mappings intentionally retain their original keys for
    callers and provenance manifests. Virtual addresses refer to the preferred
    image base; this tool does not perform relocation or execute vendor code.
    """

    def __init__(self, path: str | Path):
        self.path = Path(path)
        self.data = self.path.read_bytes()
        if self.data[:2] != b"MZ":
            raise ValueError("not an MZ image")
        header = self.u32(0x3C)
        if self.data[header : header + 4] != b"PE\0\0":
            raise ValueError("not a PE image")
        self.machine, section_count = self.unpack("<HH", header + 4)
        optional_header = header + 24
        magic = self.u16(optional_header)
        if magic not in (0x10B, 0x20B):
            raise ValueError(
                f"unsupported PE optional-header magic {magic:#x}"
            )
        self.bits = 32 if magic == 0x10B else 64
        self.width = self.bits // 8
        self.base = (
            self.u32(optional_header + 28)
            if self.bits == 32
            else self.u64(optional_header + 24)
        )
        directories = optional_header + (96 if self.bits == 32 else 112)
        self.import_rva = self.u32(directories + 8)
        section_table = optional_header + self.u16(header + 20)
        self.sections = self.read_sections(section_table, section_count)
        self.imports = self.read_imports()

    def unpack(self, format_string: str, offset: int) -> tuple[int, ...]:
        if offset < 0:
            raise ValueError(f"negative file offset {offset}")
        try:
            return struct.unpack_from(format_string, self.data, offset)
        except struct.error as error:
            raise ValueError(
                f"truncated PE data at file offset {offset:#x}"
            ) from error

    def u16(self, pos: int) -> int:
        return self.unpack("<H", pos)[0]

    def u32(self, pos: int) -> int:
        return self.unpack("<I", pos)[0]

    def u64(self, pos: int) -> int:
        return self.unpack("<Q", pos)[0]

    def read_sections(self, table: int, count: int) -> list[Section]:
        sections = []
        for index in range(count):
            position = table + index * 40
            name = (
                self.data[position : position + 8]
                .rstrip(b"\0")
                .decode("ascii", "replace")
            )
            virtual_size, rva, size, offset = self.unpack(
                "<IIII", position + 8
            )
            sections.append(
                Section(
                    name=name,
                    va=self.base + rva,
                    virtual_size=virtual_size,
                    size=size,
                    offset=offset,
                    flags=self.u32(position + 36),
                )
            )
        return sections

    def offset(self, va: int) -> int:
        """Map a preferred virtual address to a file-backed section byte."""
        for section in self.sections:
            if section["va"] <= va < section["va"] + section["size"]:
                return va - section["va"] + section["offset"]
        raise ValueError(f"VA {va:#x} is not backed by a section")

    def va(self, offset: int) -> int | None:
        for section in self.sections:
            if (
                section["offset"]
                <= offset
                < section["offset"] + section["size"]
            ):
                return offset - section["offset"] + section["va"]
        return None

    def cstr(self, offset: int) -> str:
        if offset < 0:
            raise ValueError(f"negative string offset {offset}")
        end = self.data.find(b"\0", offset)
        if end < 0:
            raise ValueError("unterminated string")
        return self.data[offset:end].decode("ascii", "replace")

    def read_imports(self) -> dict[int, str]:
        imports = {}
        if not self.import_rva:
            return imports
        position = self.offset(self.base + self.import_rva)
        while self.data[position : position + 20] != bytes(20):
            original, _, _, name, import_table = self.unpack(
                "<IIIII", position
            )
            dll = self.cstr(self.offset(self.base + name))
            thunk = self.offset(self.base + (original or import_table))
            index = 0
            while True:
                value = self.u32(thunk) if self.width == 4 else self.u64(thunk)
                if not value:
                    break
                label = (
                    f"ordinal:{value & 0xFFFF}"
                    if value >> (self.bits - 1)
                    else self.cstr(self.offset(self.base + value) + 2)
                )
                imports[self.base + import_table + index * self.width] = (
                    f"{dll}!{label}"
                )
                index += 1
                thunk += self.width
            position += 20
        return imports

    def decoder(self) -> Cs:
        try:
            from capstone import CS_ARCH_X86, CS_MODE_32, CS_MODE_64, Cs
        except ImportError as error:
            raise ValueError(
                "Capstone is required for disassembly; install the capstone "
                "package"
            ) from error
        decoder = Cs(
            CS_ARCH_X86, CS_MODE_32 if self.bits == 32 else CS_MODE_64
        )
        decoder.skipdata = True
        return decoder

    def annotation(self, operands: str) -> str:
        labels = []
        for token in re.findall(r"0x[0-9a-f]+", operands):
            address = int(token, 16)
            if address in self.imports:
                labels.append(self.imports[address])
                continue
            try:
                offset = self.offset(address)
            except ValueError:
                continue
            raw = self.data[offset : offset + 100]
            match = re.match(rb"[\x20-\x7e]{5,}", raw)
            if match:
                labels.append(repr(match[0].decode()))
            elif re.match(rb"(?:[\x20-\x7e]\x00){5,}", raw):
                end = next(
                    (
                        index
                        for index in range(0, len(raw) - 1, 2)
                        if raw[index : index + 2] == b"\0\0"
                    ),
                    len(raw) & ~1,
                )
                labels.append(repr(raw[:end].decode("utf-16le", "replace")))
        return " ; " + " | ".join(labels) if labels else ""

    def disasm(self, start: int, end: int) -> None:
        offset = self.offset(start)
        # Deliberately does not guess boundaries or follow branches.
        data = self.data[offset : offset + end - start]
        for address, size, mnemonic, operands in self.decoder().disasm_lite(
            data, start
        ):
            print(
                f"{address:08x} {mnemonic:8s} "
                f"{operands}{self.annotation(operands)}"
                f"{self.rip_annotation(address, size, operands)}"
            )

    def rip_target(self, address: int, size: int, operands: str) -> int | None:
        """Resolve x64 RIP-relative memory, without inferring a call edge."""
        if self.bits != 64:
            return None
        match = re.search(r"\[rip(?: ([+-]) (0x[0-9a-f]+))?\]", operands)
        if not match:
            return None
        displacement = int(match[2], 16) if match[2] else 0
        if match[1] == "-":
            displacement = -displacement
        return address + size + displacement

    def rip_annotation(self, address: int, size: int, operands: str) -> str:
        target = self.rip_target(address, size, operands)
        if target is None:
            return ""
        return f" ; RIP target {target:#x}{self.annotation(hex(target))}"

    def xrefs(self, targets: Iterable[int]) -> None:
        targets = set(targets)
        for section in self.sections:
            if not section["flags"] & 0x20000000:
                continue
            size = min(section["size"], section["virtual_size"])
            code = self.data[section["offset"] : section["offset"] + size]
            for (
                address,
                size,
                mnemonic,
                operands,
            ) in self.decoder().disasm_lite(code, section["va"]):
                values = {
                    int(token, 16)
                    for token in re.findall(r"0x[0-9a-f]+", operands)
                }
                relative = self.rip_target(address, size, operands)
                if relative is not None:
                    values.add(relative)
                if values & targets:
                    print(
                        f"{address:08x} {mnemonic:8s} "
                        f"{operands}{self.annotation(operands)}"
                        f"{self.rip_annotation(address, size, operands)}"
                    )

    def pointers(self, address: int) -> None:
        needle = address.to_bytes(self.width, "little")
        position = self.data.find(needle)
        while position >= 0:
            virtual_address = self.va(position)
            if virtual_address is not None:
                print(
                    f"{virtual_address:08x} file={position:#x} -> "
                    f"{address:#x} (raw pointer candidate)"
                )
            position = self.data.find(needle, position + 1)

    def strings(self, pattern: str) -> None:
        regex = re.compile(pattern, re.I)
        for encoding, expression in [
            ("ascii", rb"[\x20-\x7e]{4,}"),
            ("utf-16le", rb"(?:[\x20-\x7e]\x00){4,}"),
        ]:
            for match in re.finditer(expression, self.data):
                value = match[0].decode(encoding)
                if regex.search(value):
                    print(
                        f"{self.va(match.start()) or 0:08x} "
                        f"file={match.start():#x} {encoding} {value}"
                    )

    def words(self, address: int, count: int) -> None:
        position = self.offset(address)
        for index in range(count):
            start = position + index * self.width
            value = int.from_bytes(
                self.data[start : start + self.width], "little"
            )
            label = self.annotation(hex(value))
            try:
                target = self.offset(value)
                if (
                    self.bits == 32
                    and self.data[target : target + 2] == b"\xff\x25"
                ):
                    label += self.annotation(hex(self.u32(target + 2)))
            except ValueError:
                pass  # An arbitrary word need not point into this image.
            print(
                f"{address + index * self.width:08x} "
                f"+{index * self.width:03x}: {value:08x}{label}"
            )

    def vtables(self, descriptor: int) -> Iterator[tuple[int, int, int]]:
        """Yield (COL, object offset, vtable) candidates for one MSVC type."""
        needle = (
            descriptor if self.bits == 32 else descriptor - self.base
        ).to_bytes(4, "little")
        position = self.data.find(needle)
        while position >= 0:
            locator = self.va(position - 12) if position >= 12 else None
            signature = 0 if self.bits == 32 else 1
            valid = (
                locator is not None and self.u32(position - 12) == signature
            )
            if valid and self.bits == 64:
                valid = (
                    position + 12 <= len(self.data)
                    and self.u32(position + 8) == locator - self.base
                )
            if valid:
                pointer = locator.to_bytes(self.width, "little")
                reference = self.data.find(pointer)
                while reference >= 0:
                    table = self.va(reference + self.width)
                    if table is not None:
                        yield locator, self.u32(position - 8), table
                    reference = self.data.find(pointer, reference + 1)
            position = self.data.find(needle, position + 1)

    def rtti(self, pattern: str) -> None:
        for match in re.finditer(rb"\.\?AV[^\x00]{1,150}@@\x00", self.data):
            name = match[0][:-1].decode("ascii", "replace")
            if not re.search(pattern, name, re.I):
                continue
            descriptor = self.va(match.start() - 2 * self.width)
            if descriptor is None:
                continue
            for locator, object_offset, table in self.vtables(descriptor):
                print(
                    f"{name} descriptor={descriptor:#x} COL={locator:#x}"
                    f" object_offset={object_offset:#x} vtable={table:#x}"
                )
                self.words(table, 24)

    def qt_string(self, strings: int, index: int) -> str:
        header = strings + (16 if self.bits == 32 else 24) * index
        offset = self.offset(header)
        length = self.u32(offset + 4)
        relative = self.unpack(
            "<i" if self.bits == 32 else "<q",
            offset + (12 if self.bits == 32 else 16),
        )[0]
        raw = self.offset(header + relative)
        return self.data[raw : raw + length].decode("utf-8", "replace")

    def qt(self, address: int) -> None:
        """Inspect Qt5 QMetaObject revision-7/8 method metadata."""
        position = self.offset(address)
        read_pointer = self.u32 if self.bits == 32 else self.u64
        strings = read_pointer(position + self.width)
        table = read_pointer(position + 2 * self.width)
        data = self.offset(table)
        if self.u32(data) not in (7, 8):
            raise ValueError("unsupported Qt metadata revision")
        count, first = self.u32(data + 16), self.u32(data + 20)
        print(
            f"Qt {self.qt_string(strings, self.u32(data + 4))} "
            f"meta={address:#x}"
            f" static_metacall={read_pointer(position + 3 * self.width):#x}"
            f" methods={count}"
        )
        for index in range(count):
            entry = data + (first + 5 * index) * 4
            name, argument_count, parameters, _tag, flags = self.unpack(
                "<IIIII", entry
            )
            print(
                f"  method {index:2d}: {self.qt_string(strings, name)} "
                f"argc={argument_count} flags={flags:#x}"
                f" params_va={table + 4 * parameters:#x}"
            )


def number(value: str) -> int:
    return int(value, 0)


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--provenance", action="store_true")
    parser.add_argument("--imports", metavar="REGEX")
    parser.add_argument("--strings", metavar="REGEX")
    parser.add_argument(
        "--disasm", nargs=2, type=number, action="append", default=[]
    )
    parser.add_argument("--xrefs", nargs="+", type=number, default=[])
    parser.add_argument("--pointers", nargs="+", type=number, default=[])
    parser.add_argument(
        "--words", nargs=2, type=number, action="append", default=[]
    )
    parser.add_argument("--rtti", metavar="REGEX")
    parser.add_argument("--qt", nargs="+", type=number, default=[])
    return parser.parse_args(argv)


def run_inspection(args: argparse.Namespace) -> None:
    pe = PE(args.binary)
    if args.provenance:
        print(
            json.dumps(
                dict(
                    path=str(pe.path),
                    sha256=hashlib.sha256(pe.data).hexdigest(),
                    bits=pe.bits,
                    machine=hex(pe.machine),
                    image_base=hex(pe.base),
                    sections=pe.sections,
                ),
                indent=2,
            )
        )
    if args.imports is not None:
        for address, name in pe.imports.items():
            if re.search(args.imports, name, re.I):
                print(f"{address:08x} {name}")
    if args.strings is not None:
        pe.strings(args.strings)
    for start, end in args.disasm:
        print(f"\nDISASM {start:#x}:{end:#x} (caller-specified boundaries)")
        pe.disasm(start, end)
    if args.xrefs:
        print("\nLINEAR XREF CANDIDATES (verify control flow)")
        pe.xrefs(args.xrefs)
    for address in args.pointers:
        pe.pointers(address)
    for address, count in args.words:
        pe.words(address, count)
    if args.rtti is not None:
        pe.rtti(args.rtti)
    for address in args.qt:
        pe.qt(address)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        run_inspection(args)
    except (OSError, ValueError, re.error, OverflowError) as error:
        print(f"error: {args.binary}: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
