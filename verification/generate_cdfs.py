#!/usr/bin/env python3
"""Regenerate normative tables from a local pinned AV1 specification checkout.

Compatibility entry point. No network requests or evaluation of fetched code.
Usage: python3 verification/generate_cdfs.py /path/to/av1-spec
"""
import sys
from phase1_tables import main

if __name__ == '__main__':
    sys.argv.append('--write')
    main()
