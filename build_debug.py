#!/usr/bin/env python3
"""Build the native desktop player with debugging symbols; optionally run it."""

from tools.build_support import main


if __name__ == "__main__":
    raise SystemExit(main("debug"))
