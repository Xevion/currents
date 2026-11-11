#!/usr/bin/env python3
"""Assembles dist/ from the two wasm bundles and the page sources, under content-hashed names.

With `--package DIR`, assembles a wallpaper host's copy in DIR instead, each wasm module carried
inside a script so the page works when a host opens it from disk.
"""

import argparse
import base64
import gzip
import hashlib
import json
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WEB = ROOT / "web"
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


def emit(assets: Path, src: Path, name: str) -> str:
    """Copies `src` into `assets` under a content-hashed name and returns that name."""
    return emit_bytes(assets, src.read_bytes(), name, src.suffix)


def emit_bytes(assets: Path, data: bytes, name: str, suffix: str) -> str:
    digest = hashlib.sha256(data).hexdigest()[:10]
    out = f"{name}.{digest}{suffix}"
    (assets / out).write_bytes(data)
    return out


def inline(assets: Path, wasm: Path, name: str) -> str:
    """Writes `wasm` as a script that leaves it in `window.currentsWasm`, in base64."""
    script = f'window.currentsWasm = "{base64.b64encode(wasm.read_bytes()).decode()}";\n'
    return emit_bytes(assets, script.encode(), f"{name}.wasm", ".js")


def fill(template: str, values: dict[str, str]) -> str:
    return re.sub(r"%%(\w+)%%", lambda m: values[m.group(1)], template)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--package", type=Path, help="assemble a wallpaper host's copy here")
    args = parser.parse_args()
    dist = args.package or ROOT / "dist"
    assets = dist / "assets"
    shutil.rmtree(dist, ignore_errors=True)
    assets.mkdir(parents=True)

    manifest = {}
    for backend in BACKENDS:
        out = BUNDLES / backend
        manifest[backend] = {"js": emit(assets, out / "currents.js", backend)}
        if args.package:
            manifest[backend]["data"] = inline(assets, out / "currents_bg.wasm", backend)
        else:
            manifest[backend]["wasm"] = emit(assets, out / "currents_bg.wasm", backend)

    css_name = emit(assets, WEB / "style.css", "style")
    js_name = emit(assets, WEB / "main.js", "main")
    host_name = emit(assets, WEB / "host.js", "host")
    preload = [
        f'    <link rel="preload" href="assets/{js_name}" as="script" />',
        f'    <link rel="preload" href="assets/{css_name}" as="style" />',
    ]
    html = fill(
        (WEB / "index.html").read_text(),
        {
            "PRELOAD": "\n".join(preload),
            "CSS": css_name,
            "JS": js_name,
            "HOST": host_name,
            "MANIFEST": json.dumps(manifest, separators=(",", ":")),
            "SITE": SITE,
            "DESCRIPTION": DESCRIPTION,
            "IMAGE_ALT": IMAGE_ALT,
            "OG": emit(assets, WEB / "og.jpg", "og"),
            "ICON32": emit(assets, WEB / "icon-32.png", "icon-32"),
            "ICON180": emit(assets, WEB / "icon-180.png", "icon-180"),
        },
    )
    (dist / "index.html").write_text(html)
    if args.package:
        shutil.copy(WEB / "og.jpg", dist / "preview.jpg")
    else:
        shutil.copy(WEB / "_headers", dist / "_headers")
        report(assets, manifest)


def report(assets: Path, manifest: dict) -> None:
    print(f"{'bundle':<12} {'raw':>10} {'gzip':>10}", file=sys.stderr)
    for backend in BACKENDS:
        data = (assets / manifest[backend]["wasm"]).read_bytes()
        print(f"{backend:<12} {len(data):>10} {len(gzip.compress(data, 9)):>10}", file=sys.stderr)


if __name__ == "__main__":
    main()
