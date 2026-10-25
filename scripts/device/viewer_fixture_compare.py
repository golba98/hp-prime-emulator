#!/usr/bin/env python3
"""Offline independent RGB565 source/decoder/renderer acceptance (Pillow)."""
import argparse
import json
from pathlib import Path
from PIL import Image


def compare(raw, rendered):
    with Image.open(raw) as image:
        width, height = image.size
        # Pillow independently decompresses and unfilters the grayscale16 PNG.
        samples = list(image.getdata())
    expected = []
    for value in samples:
        r, g, b = (value >> 11) & 31, (value >> 5) & 63, value & 31
        expected.append(((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)))
    with Image.open(rendered / 'decoded.png') as image:
        decoded = list(image.convert('RGB').getdata())
    mismatch = sum(a != b for a, b in zip(expected, decoded, strict=True))
    if mismatch:
        raise ValueError(f'{mismatch} source/decoder mismatches')
    with Image.open(rendered / 'after.png') as image:
        actual = image.convert('RGB')
        w, h = actual.size
        native_scale = min(w // 320, h // 240)
        vw, vh = 320 * native_scale, 240 * native_scale
        left, top = (w - vw) // 2, (h - vh) // 2
        predicted = Image.new('RGB', (w, h), (16, 16, 20))
        source = Image.new('RGB', (width, height))
        source.putdata(expected)
        predicted.paste(source.resize((vw, vh), Image.Resampling.NEAREST), (left, top))
        render_mismatch = sum(a != b for a, b in zip(actual.getdata(), predicted.getdata(), strict=True))
        if render_mismatch:
            raise ValueError(f'{render_mismatch} renderer mismatches')
        source.save(rendered / 'independent-source.png')
        predicted.save(rendered / 'independent-expected.png')
    return {'source': str(raw), 'dimensions': [width, height], 'source_decoder_mismatches': mismatch,
            'renderer_mismatches': render_mismatch, 'viewport': [left, top, vw, vh]}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('raw', type=Path)
    parser.add_argument('rendered', type=Path)
    args = parser.parse_args()
    print(json.dumps(compare(args.raw, args.rendered), indent=2))
