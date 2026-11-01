#!/usr/bin/env python3
"""Download the pinned official Sparkle distribution and verify its checksum."""
import hashlib
from pathlib import Path
import subprocess
import urllib.request

root = Path(__file__).resolve().parents[1]
output = root / 'target/sparkle'
output.mkdir(parents=True, exist_ok=True)
archive = output / 'Sparkle-2.10.1.tar.xz'
url = 'https://github.com/sparkle-project/Sparkle/releases/download/2.10.1/Sparkle-2.10.1.tar.xz'
expected = '1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27'
if not archive.exists():
    urllib.request.urlretrieve(url, archive)
if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
    raise SystemExit('Sparkle checksum mismatch')
subprocess.run(['tar', '-xJf', str(archive), '-C', str(output)], check=True)
