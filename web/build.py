#!/usr/bin/env python3
"""Assembles dist/ from the two wasm bundles and the page sources, under content-hashed names."""

import gzip
import hashlib
import json
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WEB = ROOT / "web"
DIST = ROOT / "dist"
ASSETS = DIST / "assets"
BUNDLES = ROOT / "target" / "web"

BACKENDS = ("webgpu", "webgl")

# Link previews need absolute URLs, so the page names its own origin.
SITE = "https://currents.xevion.dev"
DESCRIPTION = (
    "The Currents album cover by Tame Impala, recreated as a real-time fluid simulation"
    " you can pan, zoom and play with in your browser."
)
IMAGE_ALT = (
    "A recreation of the Tame Impala Currents album cover: purple and black stripes flowing"
    " around a chrome sphere, with a red stream trailing behind it."
)


def emit(src: Path, name: str) -> str:
    """Copies `src` into assets/ under a content-hashed name and returns that name."""
    data = src.read_bytes()
    digest = hashlib.sha256(data).hexdigest()[:10]
    out = f"{name}.{digest}{src.suffix}"
    (ASSETS / out).write_bytes(data)
    return out


def fill(template: str, values: dict[str, str]) -> str:
    return re.sub(r"%%(\w+)%%", lambda m: values[m.group(1)], template)


def main() -> None:
    shutil.rmtree(DIST, ignore_errors=True)
    ASSETS.mkdir(parents=True)

    manifest = {}
    for backend in BACKENDS:
        out = BUNDLES / backend
        manifest[backend] = {
            "js": emit(out / "currents.js", backend),
            "wasm": emit(out / "currents_bg.wasm", backend),
        }

    css_name = emit(WEB / "style.css", "style")
    js_name = emit(WEB / "main.js", "main")
    preload = [
        f'    <link rel="modulepreload" href="assets/{js_name}" />',
        f'    <link rel="preload" href="assets/{css_name}" as="style" />',
    ]
    html = fill(
        (WEB / "index.html").read_text(),
        {
            "PRELOAD": "\n".join(preload),
            "CSS": css_name,
            "JS": js_name,
            "MANIFEST": json.dumps(manifest, separators=(",", ":")),
            "SITE": SITE,
            "DESCRIPTION": DESCRIPTION,
            "IMAGE_ALT": IMAGE_ALT,
            "OG": emit(WEB / "og.jpg", "og"),
            "ICON32": emit(WEB / "icon-32.png", "icon-32"),
            "ICON180": emit(WEB / "icon-180.png", "icon-180"),
        },
    )
    (DIST / "index.html").write_text(html)
    shutil.copy(WEB / "_headers", DIST / "_headers")

    report(manifest)


def report(manifest: dict) -> None:
    print(f"{'bundle':<12} {'raw':>10} {'gzip':>10}", file=sys.stderr)
    for backend in BACKENDS:
        data = (ASSETS / manifest[backend]["wasm"]).read_bytes()
        print(f"{backend:<12} {len(data):>10} {len(gzip.compress(data, 9)):>10}", file=sys.stderr)


if __name__ == "__main__":
    main()
