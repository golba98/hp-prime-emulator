# Pixel geometry fixture

`g1_pixel_geometry.ppl` draws a red inclusive 20×20 rectangle, a blue
21-pixel horizontal line, and one green pixel on a white 320×240 screen.
It exercises RGB, RECT_P, LINE_P and PIXON_P, including unchanged neighbors.

Run without any physical device:

```sh
ppl run --engine firmware --entry PIXELPROBE --png firmware.png tests/compat/programs/g1_pixel_geometry.ppl
ppl run --engine native --entry PIXELPROBE --png native.png tests/compat/programs/g1_pixel_geometry.ppl
python3 scripts/compat/check_pixel_geometry.py firmware.png
python3 scripts/compat/check_pixel_geometry.py native.png
python3 scripts/compat/test_pixel_geometry.py
```

On 2026-11-02 both Native and local G1 firmware 15751 matched all 76,800
expected pixels. The local firmware capture was visually inspected. The
checker detects a one-pixel translation and extra background ink, and rejects
scaled captures. Source SHA-256:
`1f4921750133fee68286c3241e56d0109d46eed58185c85ddd9d5a724146b61c`.

Physical G1: **untested**. A future manually captured, unscaled device screen
can use the same checker. Record calculator model, firmware version, source
hash, capture time and raw capture alongside that result. Do not relabel local
firmware output as device evidence. This fixture contains no upload, deletion,
remote keys or other hardware automation.
