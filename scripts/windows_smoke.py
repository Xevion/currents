"""Renders a frame with the Windows exe under Proton and with the native build, and checks the
two look alike. Proton runs on a virtual X display, so nothing opens on the desktop.

Usage: windows_smoke.py [PROTON_DIR]; without one, the newest GE-Proton, else Proton, is used.
"""

import os
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "target" / "windows-smoke"
EXE = ROOT / "target" / "x86_64-pc-windows-gnu" / "release" / "currents.exe"
STEAM = Path.home() / ".local" / "share" / "Steam"
RENDER = ["--shaders", "shaders", "render", "--size", "480x300", "--time", "3"]
# Mean difference per channel, out of 255, under which the two frames count as the same picture.
SAME = 3.0


def find_proton() -> Path:
    candidates = sorted((STEAM / "compatibilitytools.d").glob("GE-Proton*"), reverse=True)
    candidates += sorted((STEAM / "steamapps" / "common").glob("Proton*"), reverse=True)
    for candidate in candidates:
        if (candidate / "proton").is_file():
            return candidate
    sys.exit("no Proton found under ~/.local/share/Steam; pass its directory")


def windows_path(path: Path) -> str:
    return "Z:" + str(path).replace("/", "\\")


def main() -> None:
    proton = Path(sys.argv[1]) if len(sys.argv) > 1 else find_proton()
    (WORK / "compat").mkdir(parents=True, exist_ok=True)
    native, windows = WORK / "native.png", WORK / "windows.png"
    for stale in (native, windows):
        stale.unlink(missing_ok=True)

    subprocess.run(
        ["cargo", "run", "-q", "--release", "-p", "currents", "--", *RENDER, "--out", str(native)],
        cwd=ROOT,
        check=True,
    )
    env = {
        **os.environ,
        "STEAM_COMPAT_DATA_PATH": str(WORK / "compat"),
        "STEAM_COMPAT_CLIENT_INSTALL_PATH": str(STEAM),
        "WGPU_BACKEND": "vulkan",
        # Its accessibility helper stalls a new prefix on a display with no desktop.
        "PROTON_USE_XALIA": "0",
    }
    print(f"running under {proton.name}", file=sys.stderr)
    try:
        subprocess.run(
            ["xvfb-run", "-a", "-s", "-screen 0 1280x720x24", str(proton / "proton"), "run", str(EXE), *RENDER]
            + ["--out", windows_path(windows)],
            cwd=ROOT,
            env=env,
            check=True,
            timeout=600,
        )
    finally:
        # The prefix's server outlives the program; stop it so nothing is left running.
        prefix = {**env, "WINEPREFIX": str(WORK / "compat" / "pfx")}
        subprocess.run([str(proton / "files" / "bin" / "wineserver"), "-k"], env=prefix, check=False)
    if not windows.is_file():
        sys.exit("the Windows build wrote no frame")
    a = np.asarray(Image.open(native).convert("RGB"), dtype=np.float32)
    b = np.asarray(Image.open(windows).convert("RGB"), dtype=np.float32)
    if a.shape != b.shape:
        sys.exit(f"frames differ in size: {a.shape} native, {b.shape} Windows")
    diff = float(np.abs(a - b).mean())
    print(f"mean difference {diff:.2f} of 255 per channel")
    if diff > SAME:
        sys.exit(f"the Windows frame does not match the native one (over {SAME})")


if __name__ == "__main__":
    main()
