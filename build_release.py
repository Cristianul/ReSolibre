#!/usr/bin/env python3
"""Build the optimized native desktop player; optionally run it locally."""

from tools.build_support import main


if __name__ == "__main__":
    raise SystemExit(main("release"))
