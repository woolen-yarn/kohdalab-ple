#!/usr/bin/env python3
"""Explicit launcher for the existing CONEX-CC reference adapter.

This is only an integration placeholder for the future controller-PC service.
Calling it with ``--help`` never imports a vendor SDK, opens a socket, or moves
hardware.  Starting the real adapter requires an explicit acknowledgement.
"""
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Launcher for the existing bridge/conex_server.py reference adapter."
    )
    parser.add_argument(
        "--run-reference",
        action="store_true",
        help="explicitly start the existing reference adapter on this controller PC",
    )
    parsed, reference_args = parser.parse_known_args()
    if not parsed.run_reference:
        parser.print_help()
        return 0

    reference = Path(__file__).resolve().parents[2] / "bridge" / "conex_server.py"
    if not reference.is_file():
        parser.error(f"reference adapter is unavailable: {reference}")
    return subprocess.call([sys.executable, str(reference), *reference_args])


if __name__ == "__main__":
    raise SystemExit(main())
