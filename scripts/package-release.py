#!/usr/bin/env python3
"""Build complete portable packages; macOS signing is a separate mandatory release step."""
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
os.chdir(ROOT)
version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
system = platform.system()
arch = {'aarch64': 'arm64', 'AMD64': 'x86_64', 'arm64': 'arm64'}.get(platform.machine(), platform.machine())
name = f'Rustable-{system.lower()}-{arch}'
stage = ROOT / 'target' / 'packages' / name
stage.mkdir(parents=True, exist_ok=True)
dist = ROOT / 'dist'
dist.mkdir(exist_ok=True)
def run(*args):
    subprocess.run(args, check=True)
helper = ['mise', 'exec', '--', 'cargo', 'run', '--locked', '--manifest-path', 'tools/package/Cargo.toml', '--']
if system == 'Darwin':
    app = stage / 'Rustable.app'
    run(*helper, 'bundle', 'target/release/rustable', str(app))
    path = app / 'Contents/Info.plist'
    with path.open('rb') as f:
        info = plistlib.load(f)
    info['CFBundleShortVersionString'] = version
    info['CFBundleVersion'] = version
    info['CFBundleIconFile'] = 'Rustable.icns'
    shutil.copy2(ROOT / 'assets/Rustable.icns', app / 'Contents/Resources/Rustable.icns')
    with path.open('wb') as f:
        plistlib.dump(info, f)
    # Signed/notarized distribution archives are created only after signing.
    print(app)
else:
    run(*helper, 'licenses')
    suffix = '.exe' if system == 'Windows' else ''
    for source in [ROOT / f'target/release/rustable{suffix}', ROOT / f'target/lsp-tools/bin/syntaqlite{suffix}', ROOT / f'target/pg-language-server/release/rustable-postgres-language-server{suffix}']:
        if not source.is_file():
            raise SystemExit(f'Missing required executable: {source}')
        shutil.copy2(source, stage / source.name)
    shutil.copytree(ROOT / 'target/bundle-licenses', stage / 'Licenses', dirs_exist_ok=True)
    shutil.copy2(ROOT / 'LICENSE', stage / 'LICENSE')
    shutil.make_archive(str(dist / name), 'zip' if system == 'Windows' else 'gztar', stage.parent, stage.name)
