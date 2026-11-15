#!/usr/bin/env python3
"""Validate optional installed fonts through both production Native CLI paths.

Reference hashes were captured from the original G1 firmware. No calculator,
firmware write, vendor-font distribution or private capture file is needed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
from PIL import Image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ppl', type=Path, required=True)
    parser.add_argument('--resource', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    fixture = Path(__file__).resolve().parents[2] / 'tests/fixtures/native-font-controls.json'
    controls = json.loads(fixture.read_text())
    config = args.out / 'fonts.toml'
    factory = args.out / 'installed-factory'
    archive = factory / 'programs/misc/armfir.dat'
    archive.parent.mkdir(parents=True)
    archive.symlink_to(args.resource.resolve())
    config.write_text('factory_a = ' + json.dumps(str(factory.resolve())) + '\n')
    empty = args.out / 'empty.toml'
    empty.write_text('')
    env = os.environ.copy()
    for name in ('PPL_PRIME_FONT_RESOURCE', 'PPL_FACTORY_A', 'PPL_FIRMWARE_INIT_DATA'):
        env.pop(name, None)
    reports = []
    for selection in ('explicit', 'configured'):
        env['PPL_CONFIG'] = str(empty if selection == 'explicit' else config)
        if selection == 'explicit':
            env['PPL_PRIME_FONT_RESOURCE'] = str(args.resource.resolve())
        else:
            env.pop('PPL_PRIME_FONT_RESOURCE', None)
        for case in controls['cases']:
            source = args.out / (case['name'] + '.hpprgm')
            source.write_text(case['source'])
            for mode in ('legacy', 'unified'):
                label = f"{selection}-{mode}-{case['name']}"
                image = args.out / (label + '.png')
                command = [str(args.ppl.resolve()), 'run', str(source), '--keys', '30', '--png', str(image)]
                if mode == 'unified':
                    command += ['--engine', 'native', '--json']
                result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=30)
                (args.out / (label + '.stdout')).write_text(result.stdout)
                (args.out / (label + '.stderr')).write_text(result.stderr)
                assert result.returncode == 0, (label, result.returncode, result.stderr)
                if mode == 'unified':
                    report = json.loads(result.stdout)
                    assert report['backend']['font_family'] == 'Prime Sans', report
                raster = Image.open(image).convert('RGB')
                assert raster.size == (320, 240)
                digest = hashlib.sha256(raster.tobytes()).hexdigest()
                assert digest == case['rgb_sha256'], (label, digest, case['rgb_sha256'])
                reports.append(dict(case=label, rgb_sha256=digest))
    # A bad explicitly selected archive must not publish a substitute image.
    env['PPL_CONFIG'] = str(empty)
    env['PPL_PRIME_FONT_RESOURCE'] = str(args.out / 'missing-resource.dat')
    source = args.out / 'selected-missing.hpprgm'
    source.write_text('EXPORT FONTMISS() BEGIN RETURN 1; END;')
    sentinel = args.out / 'preserved.png'
    sentinel.write_bytes(b'prior caller-owned image')
    for mode in ('legacy', 'unified'):
        command = [str(args.ppl.resolve()), 'run', str(source), '--png', str(sentinel)]
        if mode == 'unified':
            command += ['--engine', 'native', '--json']
        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=30)
        assert result.returncode != 0, (mode, result.stdout, result.stderr)
        assert sentinel.read_bytes() == b'prior caller-owned image'
        # Syntax checking remains independent of optional drawing resources.
        check = [str(args.ppl.resolve()), 'check', str(source)]
        if mode == 'unified':
            check += ['--engine', 'native', '--json']
        result = subprocess.run(check, env=env, capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, (mode, result.stdout, result.stderr)
    (args.out / 'report.json').write_text(json.dumps(reports, indent=2) + '\n')
    print(f"Native fonts: {len(reports)} exact complete G1 framebuffer comparisons; invalid-selection output preservation and independent Check passed")


if __name__ == '__main__':
    main()
