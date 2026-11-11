#!/usr/bin/env python3
"""Assembles a frontend from the two wasm bundles and its page sources, under content-hashed names.

`site` builds the website into dist/. `wallpaper DIR` builds the copy a wallpaper host opens from
disk, with each wasm module carried inside a script, since such a page may not fetch files.
"""

import argparse
import base64
import gzip
import hashlib
import json
import re
import shutil
import subprocess
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


def bindings(assets: Path, backend: str) -> str:
    """The wasm-bindgen script as a factory, so each call makes fresh bindings and a page can
    start a backend again after a failure; the script itself declares a global only once."""
    glue = (BUNDLES / backend / "currents.js").read_text()
    script = f"(globalThis.currentsBundles ??= {{}})[{json.dumps(backend)}] = () => {{\n{glue}\nreturn wasm_bindgen;\n}};\n"
    return emit_bytes(assets, script.encode(), backend, ".js")


def inline(assets: Path, backend: str) -> str:
    """The wasm module as a script leaving it in base64 under `currentsWasm[backend]`."""
    data = base64.b64encode((BUNDLES / backend / "currents_bg.wasm").read_bytes()).decode()
    script = f"(globalThis.currentsWasm ??= {{}})[{json.dumps(backend)}] = \"{data}\";\n"
    return emit_bytes(assets, script.encode(), f"{backend}.wasm", ".js")


def bundle(assets: Path, entry: Path) -> str:
    """The frontend's script with its imports, as one classic script that runs from disk too."""
    out = subprocess.run(
        ["mise", "exec", "--", "bun", "build", str(entry), "--format=iife", "--target=browser", "--minify"],
        check=True,
        capture_output=True,
    ).stdout
    return emit_bytes(assets, out, "main", ".js")


def fill(template: str, values: dict[str, str]) -> str:
    return re.sub(r"%%(\w+)%%", lambda m: values[m.group(1)], template)


def fresh(dist: Path) -> Path:
    assets = dist / "assets"
    shutil.rmtree(dist, ignore_errors=True)
    assets.mkdir(parents=True)
    return assets


def site(dist: Path) -> None:
    src = WEB / "site"
    assets = fresh(dist)
    manifest = {}
    for backend in BACKENDS:
        wasm = BUNDLES / backend / "currents_bg.wasm"
        manifest[backend] = {
            "js": bindings(assets, backend),
            "wasm": emit(assets, wasm, backend),
            "size": wasm.stat().st_size,
        }
    poster = "data:image/jpeg;base64," + base64.b64encode((src / "poster.jpg").read_bytes()).decode()
    css = fill((src / "style.css").read_text(), {"POSTER": poster})
    css_name = emit_bytes(assets, css.encode(), "style", ".css")
    js_name = bundle(assets, src / "main.js")
    preload = [
        f'    <link rel="preload" href="assets/{js_name}" as="script" />',
    ]
    html = fill(
        (src / "index.html").read_text(),
        {
            "PRELOAD": "\n".join(preload),
            "CSS": css_name,
            "JS": js_name,
            "MANIFEST": json.dumps(manifest, separators=(",", ":")),
            "SITE": SITE,
            "DESCRIPTION": DESCRIPTION,
            "IMAGE_ALT": IMAGE_ALT,
            "OG": emit(assets, src / "og.jpg", "og"),
            "ICON32": emit(assets, src / "icon-32.png", "icon-32"),
            "ICON180": emit(assets, src / "icon-180.png", "icon-180"),
        },
    )
    (dist / "index.html").write_text(html)
    shutil.copy(src / "_headers", dist / "_headers")
    report(assets, manifest)


def wallpaper(dist: Path) -> None:
    src = WEB / "wallpaper"
    assets = fresh(dist)
    manifest = {backend: {"js": bindings(assets, backend), "data": inline(assets, backend)} for backend in BACKENDS}
    html = fill(
        (src / "index.html").read_text(),
        {
            "CSS": emit(assets, src / "style.css", "style"),
            "JS": bundle(assets, src / "main.js"),
            "POSTER": emit(assets, src / "poster.jpg", "poster"),
            "MANIFEST": json.dumps(manifest, separators=(",", ":")),
        },
    )
    (dist / "index.html").write_text(html)
    shutil.copy(WEB / "site" / "og.jpg", dist / "preview.jpg")


def report(assets: Path, manifest: dict) -> None:
    print(f"{'bundle':<12} {'raw':>10} {'gzip':>10}", file=sys.stderr)
    for backend in BACKENDS:
        data = (assets / manifest[backend]["wasm"]).read_bytes()
        print(f"{backend:<12} {len(data):>10} {len(gzip.compress(data, 9)):>10}", file=sys.stderr)


def main() -> None:
    parser = argparse.ArgumentParser()
    targets = parser.add_subparsers(dest="target", required=True)
    targets.add_parser("site", help="the website, into dist/")
    paper = targets.add_parser("wallpaper", help="a wallpaper host's copy")
    paper.add_argument("dir", type=Path)
    args = parser.parse_args()
    if args.target == "site":
        site(ROOT / "dist")
    else:
        wallpaper(args.dir)


if __name__ == "__main__":
    main()
