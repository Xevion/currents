#!/usr/bin/env python3
"""Serves dist/ with the same response headers Cloudflare applies from dist/_headers."""

import fnmatch
import sys
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


def parse_headers(path: Path) -> list[tuple[str, list[tuple[str, str]]]]:
    rules: list[tuple[str, list[tuple[str, str]]]] = []
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        if not line.startswith((" ", "\t")):
            rules.append((line.strip(), []))
        else:
            name, _, value = line.strip().partition(":")
            rules[-1][1].append((name, value.strip()))
    return rules


class Handler(SimpleHTTPRequestHandler):
    extensions_map = {**SimpleHTTPRequestHandler.extensions_map, ".wasm": "application/wasm"}

    def __init__(self, *args, rules, **kwargs):
        self.rules = rules
        super().__init__(*args, **kwargs)

    def end_headers(self):
        path = self.path.split("?", 1)[0]
        for pattern, headers in self.rules:
            if fnmatch.fnmatch(path, pattern):
                for name, value in headers:
                    self.send_header(name, value)
        super().end_headers()


def main() -> None:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else "dist")
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 8080
    handler = partial(Handler, directory=str(root), rules=parse_headers(root / "_headers"))
    print(f"serving {root} on http://localhost:{port}")
    ThreadingHTTPServer(("127.0.0.1", port), handler).serve_forever()


if __name__ == "__main__":
    main()
