#!/usr/bin/env python3
"""Build the Rust/egui demo and stage self-hosted SQLite WASM. No runtime CDN."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--output', type=Path, required=True, help='Website public/demo directory')
args = parser.parse_args()
package = root / 'tools/browser'
subprocess.run(['mise','exec','--','cargo','build','--locked','--release','--target','wasm32-unknown-unknown','--manifest-path',str(package/'Cargo.toml')],check=True)
out = args.output.resolve()
out.mkdir(parents=True,exist_ok=True)
subprocess.run(['mise','exec','--','wasm-bindgen','--target','web','--out-dir',str(out/'pkg'),str(package/'target/wasm32-unknown-unknown/release/rustable_browser.wasm')],check=True)
for name in ['index.html','sqlite-worker.js','boot.js','demo.css']:
    shutil.copy2(package/'web'/name,out/name)
shutil.copy2(root/'src/demo.sql',out/'sample.sql')
url = 'https://registry.npmjs.org/@sqlite.org/sqlite-wasm/-/sqlite-wasm-3.54.1-build1.tgz'
archive = urllib.request.urlopen(url,timeout=30).read()
assert hashlib.sha256(archive).hexdigest() == '06aaad2dd0b58a46631000a31a4469afff4abeffbc383f37372b6590396d9bbd', 'SQLite package checksum mismatch'
(out/'sqlite').mkdir(exist_ok=True)
with tarfile.open(fileobj=io.BytesIO(archive)) as source:
    for name in ['index.mjs','sqlite3.wasm']:
        (out/'sqlite'/name).write_bytes(source.extractfile('package/dist/'+name).read())
    cache = package/'target/sqlite'
    cache.mkdir(parents=True,exist_ok=True)
    for name in ['node.mjs','sqlite3.wasm']:
        (cache/name).write_bytes(source.extractfile('package/dist/'+name).read())
licenses = out/'Licenses'
licenses.mkdir(exist_ok=True)
metadata=json.loads(subprocess.check_output(['mise','exec','--','cargo','metadata','--locked','--format-version','1','--filter-platform','wasm32-unknown-unknown','--manifest-path',str(package/'Cargo.toml')]))
index=[]
for dependency in metadata['packages']:
    name=f"{dependency['name']}-{dependency['version']}"
    directory=Path(dependency['manifest_path']).parent
    index.append(f"{name}: {dependency.get('license') or 'See package source'}")
    for license_file in directory.iterdir():
        if license_file.is_file() and license_file.name.upper().startswith(('LICENSE','COPYING','NOTICE')):
            destination=licenses/name
            destination.mkdir(exist_ok=True)
            shutil.copy2(license_file,destination/license_file.name)
(licenses/'INDEX.txt').write_text('\n'.join(sorted(index))+'\n')
for name,url in [('Apache-2.0.txt','https://www.apache.org/licenses/LICENSE-2.0.txt'),('Emscripten-LICENSE.txt','https://raw.githubusercontent.com/emscripten-core/emscripten/5.0.5/LICENSE')]:
    (licenses/name).write_bytes(urllib.request.urlopen(url,timeout=30).read())
shutil.copy2(root/'LICENSE',out/'LICENSE')
(out/'BUILD.txt').write_text('Rustable browser demo\nSQLite WASM npm package: 3.54.1-build1 (Apache-2.0 package; SQLite public domain)\nSource: https://github.com/RustableApp/Rustable/tree/development/browser-demo/tools/browser\n')
print(out)
