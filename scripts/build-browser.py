#!/usr/bin/env python3
"""Build the shared Rustable workspace and its Rust/SQLite WASM worker."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--output', type=Path, required=True, help='Website public/demo directory')
args = parser.parse_args()
destination = args.output.resolve()
# This directory is generated as a unit, never merge it with an older build.
# Refuse unrelated directories rather than removing user-owned contents.
if destination.exists() and any(destination.iterdir()):
    marker = destination / 'BUILD.txt'
    if not marker.is_file() or not marker.read_text().startswith('Rustable browser demo\n'):
        raise SystemExit('Output must be empty or an existing generated Rustable browser demo')
destination.parent.mkdir(parents=True, exist_ok=True)
package = root / 'tools/browser'
# Apple ar cannot index WASM objects. Use LLVM's matching archiver on every host.
sysroot = Path(subprocess.check_output(['mise','exec','--','rustc','--print','sysroot'],text=True).strip())
host = next(line.split(': ',1)[1] for line in subprocess.check_output(['mise','exec','--','rustc','-vV'],text=True).splitlines() if line.startswith('host: '))
archiver = sysroot/'lib/rustlib'/host/'bin'/('llvm-ar.exe' if os.name == 'nt' else 'llvm-ar')
if not archiver.is_file():
    raise SystemExit('Install the matching archiver: mise exec -- rustup component add llvm-tools')
build_env = dict(os.environ, AR_wasm32_unknown_unknown=str(archiver))
subprocess.run(['mise','exec','--','cargo','build','--locked','--release','--target','wasm32-unknown-unknown','--manifest-path',str(package/'Cargo.toml')],check=True,env=build_env)
with tempfile.TemporaryDirectory(prefix='.rustable-demo-', dir=destination.parent) as staging:
    out = Path(staging) / 'demo'
    out.mkdir()
    subprocess.run(['mise','exec','--','wasm-bindgen','--target','web','--out-dir',str(out/'pkg'),str(package/'target/wasm32-unknown-unknown/release/rustable_browser.wasm')],check=True)
    for name in ['index.html','database-worker.js','background-worker.js','boot.js','demo.css']:
        shutil.copy2(package/'web'/name,out/name)
    licenses = out/'Licenses'
    licenses.mkdir(exist_ok=True)
    metadata=json.loads(subprocess.check_output(['mise','exec','--','cargo','metadata','--locked','--format-version','1','--filter-platform','wasm32-unknown-unknown','--manifest-path',str(package/'Cargo.toml')]))
    font_notices=licenses/'NotoEmoji'
    font_notices.mkdir()
    for name in ['NotoEmoji-OFL.txt', 'README.md']:
        shutil.copy2(root/'assets/fonts'/name,font_notices/name)
    inter_notices=licenses/'Inter'
    inter_notices.mkdir(exist_ok=True)
    for name in ['Inter-OFL.txt', 'README.md']:
        shutil.copy2(root/'assets/fonts'/name,inter_notices/name)
    icon_notices=licenses/'Lucide'
    icon_notices.mkdir()
    for name in ['LICENSE', 'README.md']:
        shutil.copy2(root/'assets/icons/lucide'/name, icon_notices/name)
    database_notices=licenses/'Devicon'
    database_notices.mkdir()
    for name in ['LICENSE', 'README.md']:
        shutil.copy2(root/'assets/icons/databases'/name, database_notices/name)
    index=['Inter: OFL-1.1 (see Inter/Inter-OFL.txt)',
           'Devicon database icons: MIT (see Devicon/LICENSE)',
           'Noto Emoji: OFL-1.1 (see NotoEmoji/NotoEmoji-OFL.txt)',
           'Lucide: ISC and MIT (see Lucide/LICENSE)']
    (out/'fonts').mkdir()
    shutil.copy2(root/'assets/fonts/NotoSansCJKjp-Regular.otf',out/'fonts/NotoSansCJKjp-Regular.otf')
    cjk_notices=licenses/'NotoSansCJK'
    cjk_notices.mkdir()
    for name in ['NotoSansCJK-OFL.txt', 'README.md']:
        shutil.copy2(root/'assets/fonts'/name,cjk_notices/name)
    index.append('Noto Sans CJK: OFL-1.1 (see NotoSansCJK/NotoSansCJK-OFL.txt)')
    for dependency in metadata['packages']:
        name=f"{dependency['name']}-{dependency['version']}"
        directory=Path(dependency['manifest_path']).parent
        index.append(f"{name}: {dependency.get('license') or 'See package source'}")
        for license_file in directory.iterdir():
            if license_file.is_file() and license_file.name.upper().startswith(('LICENSE','COPYING','NOTICE')):
                license_destination=licenses/name
                license_destination.mkdir(exist_ok=True)
                shutil.copy2(license_file,license_destination/license_file.name)
    (licenses/'INDEX.txt').write_text('\n'.join(sorted(index))+'\n')
    shutil.copy2(root/'LICENSE',out/'LICENSE')
    revision = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
    version = tomllib.loads((root/'Cargo.toml').read_text())['package']['version']
    dirty = bool(subprocess.check_output(['git', '-C', str(root), 'status', '--porcelain', '--untracked-files=no'], text=True).strip())
    (out/'BUILD.txt').write_text(f'Rustable browser demo\nVersion: {version}\nCommit: {revision}\nModified tracked files: {dirty}\nShared native Rustable UI and SQLite driver, compiled to WASM\nSource: https://github.com/RustableApp/Rustable/tree/{revision}/tools/browser\n')
    # Publish only after compilation, bindings, assets and licenses all succeed.
    # Keep the previous directory available for rollback if installation fails.
    previous = Path(staging) / 'previous'
    if destination.exists():
        destination.rename(previous)
    try:
        out.rename(destination)
    except OSError:
        if previous.exists():
            previous.rename(destination)
        raise
print(destination)
