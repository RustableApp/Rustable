#!/usr/bin/env python3
"""Render the macOS icon from our vector mark (requires resvg and iconutil)."""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

with tempfile.TemporaryDirectory(prefix="rustable-icon-") as temporary:
    directory = Path(temporary)
    # The mark spans x/y 1..127, including its stroke. Fit that visible tile
    # into 824px, centered on a transparent 1024px canvas for legacy ICNS.
    scale = 824 / 126
    offset = 100 - scale
    mark = (ROOT / "assets/rustable-mark.svg").read_text()
    body = mark.split(">", 1)[1].rsplit("</svg>", 1)[0]
    svg = directory / "macos.svg"
    svg.write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">'
        f'<g transform="translate({offset} {offset}) scale({scale})">'
        f'{body}</g></svg>'
    )
    iconset = directory / "Rustable.iconset"
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        for density in (1, 2):
            pixels = size * density
            suffix = "@2x" if density == 2 else ""
            output = iconset / f"icon_{size}x{size}{suffix}.png"
            subprocess.run(["resvg", "--width", str(pixels), str(svg), str(output)], check=True)
    (ROOT / "assets/rustable-macos-1024.png").write_bytes(
        (iconset / "icon_512x512@2x.png").read_bytes()
    )
    subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o",
                    str(ROOT / "assets/Rustable.icns")], check=True)
