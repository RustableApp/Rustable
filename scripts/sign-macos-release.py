#!/usr/bin/env python3
"""Sign, notarize, staple and generate a signed Sparkle feed. Never falls back to unsigned."""
import base64
import json
import os
from pathlib import Path
import platform
import secrets
import shlex
import shutil
import subprocess
import tempfile
import tomllib
import xml.etree.ElementTree as ET

root = Path(__file__).resolve().parents[1]
os.chdir(root)
def run(*args, **kwargs):
    result = subprocess.run(args, **kwargs)
    if result.returncode:
        # Do not include argv: security commands contain credential parameters.
        raise SystemExit(f'{args[0]} failed with exit code {result.returncode}')
    return result
def required(name):
    value = os.environ.get(name, '')
    if not value:
        raise SystemExit(f'Missing required release credential: {name}')
    return value
certificate = required('MACOS_CERTIFICATE_P12_BASE64')
password = required('MACOS_CERTIFICATE_PASSWORD')
identity = required('MACOS_SIGNING_IDENTITY')
notary_key = required('APPLE_NOTARY_KEY_P8')
key_id = required('APPLE_NOTARY_KEY_ID')
issuer = required('APPLE_NOTARY_ISSUER_ID')
public_key = required('SPARKLE_PUBLIC_KEY')
private_key = required('SPARKLE_PRIVATE_KEY')
version = tomllib.loads((root / 'Cargo.toml').read_text())['package']['version']
tag = required('RELEASE_TAG')
if tag != f'v{version}':
    raise SystemExit('Release tag must match Cargo package version')
release_notes = (root / 'docs' / f'RELEASE-{version}.md').read_text(encoding='utf-8').strip()
if not release_notes:
    raise SystemExit('Release notes must not be empty')
arch = platform.machine()
if arch not in ('arm64', 'x86_64'):
    raise SystemExit(f'Unsupported macOS architecture: {arch}')
app = root / f'target/packages/Rustable-darwin-{arch}/Rustable.app'
dist = root / 'dist'
dist.mkdir(exist_ok=True)
archive = dist / f'Rustable-macos-{arch}.zip'
feed_name = f'appcast-macos-{arch}.xml'
feed = f'https://github.com/RustableApp/Rustable/releases/latest/download/{feed_name}'
run('mise', 'exec', '--', 'cargo', 'run', '--locked', '--manifest-path', 'tools/package/Cargo.toml', '--', 'configure-updates', str(app), str(root/'target/sparkle/Sparkle.framework'), feed, public_key)
with tempfile.TemporaryDirectory(prefix='rustable-signing-') as temporary:
    private = Path(temporary)
    p12 = private / 'identity.p12'
    p12.write_bytes(base64.b64decode(certificate, validate=True)); p12.chmod(0o600)
    p8 = private / 'notary.p8'
    p8.write_text(notary_key); p8.chmod(0o600)
    keychain = private / 'release.keychain-db'
    keychain_password = secrets.token_urlsafe(32)
    original_keychains = shlex.split(run('security', 'list-keychains', '-d', 'user', capture_output=True, text=True).stdout)
    try:
        run('security', 'create-keychain', '-p', keychain_password, str(keychain), stdout=subprocess.DEVNULL)
        run('security', 'set-keychain-settings', '-lut', '21600', str(keychain))
        run('security', 'unlock-keychain', '-p', keychain_password, str(keychain))
        run('security', 'import', str(p12), '-k', str(keychain), '-P', password, '-T', '/usr/bin/codesign', stdout=subprocess.DEVNULL)
        run('security', 'list-keychains', '-d', 'user', '-s', str(keychain), *original_keychains)
        identities = run('security', 'find-identity', '-v', '-p', 'codesigning', str(keychain), capture_output=True, text=True).stdout
        if identity not in identities:
            raise SystemExit('Signing identity is not available in the imported release keychain')
        run('security', 'set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:', '-s', '-k', keychain_password, str(keychain), stdout=subprocess.DEVNULL)
        def sign(path):
            run('codesign', '--force', '--timestamp', '--options', 'runtime', '--preserve-metadata=entitlements', '--keychain', str(keychain), '--sign', identity, str(path))
        # Sign nested code inside-out, without codesign --deep as a signing shortcut.
        paths = sorted(app.rglob('*'), key=lambda path: len(path.parts), reverse=True)
        for path in paths:
            if path.is_symlink():
                continue
            if path.is_file():
                kind = run('file', '-b', str(path), capture_output=True, text=True).stdout
                if 'Mach-O' in kind:
                    sign(path)
            elif path.suffix in ('.app', '.xpc', '.framework'):
                sign(path)
        sign(app)
        run('codesign', '--verify', '--deep', '--strict', '--verbose=2', str(app))
        run('ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', str(app), str(archive))
        result = run('xcrun', 'notarytool', 'submit', str(archive), '--key', str(p8), '--key-id', key_id, '--issuer', issuer, '--wait', '--timeout', '30m', '--output-format', 'json', capture_output=True, text=True)
        response = json.loads(result.stdout)
        if response.get('status') != 'Accepted':
            raise SystemExit(f'Notarization was not accepted: {response.get("status")} (submission {response.get("id")})')
        run('xcrun', 'stapler', 'staple', str(app))
        run('xcrun', 'stapler', 'validate', str(app))
        run('spctl', '--assess', '--type', 'execute', '--verbose=2', str(app))
        archive.unlink()
        run('ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', str(app), str(archive))
        with tempfile.TemporaryDirectory(prefix='rustable-feed-') as feed_dir:
            shutil.copy2(archive, Path(feed_dir) / archive.name)
            # Sparkle discovers notes by the archive basename. Embed Markdown
            # so the update prompt needs no separately hosted release-notes page.
            (Path(feed_dir) / archive.with_suffix('.md').name).write_text(release_notes, encoding='utf-8')
            run(str(root/'target/sparkle/bin/generate_appcast'), '--ed-key-file', '-', '--maximum-deltas', '0', '--embed-release-notes', '--download-url-prefix', f'https://github.com/RustableApp/Rustable/releases/download/{tag}/', '-o', str(dist/feed_name), feed_dir, input=private_key+'\n', text=True)
            description = ET.parse(dist / feed_name).find('./channel/item/description')
            if (description is None or (description.text or '').strip() != release_notes
                    or description.get('{http://www.andymatuschak.org/xml-namespaces/sparkle}format') != 'markdown'):
                raise SystemExit('Generated Sparkle feed is missing the release notes')
        # Keep Sparkle's ZIP feed separate from the human-facing DMG installer.
        dmg = dist / f'Rustable-macos-{arch}.dmg'
        run('python', 'scripts/create-dmg.py', str(app), str(dmg))
        run('codesign', '--force', '--timestamp', '--keychain', str(keychain), '--sign', identity, str(dmg))
        result = run('xcrun', 'notarytool', 'submit', str(dmg), '--key', str(p8), '--key-id', key_id, '--issuer', issuer, '--wait', '--timeout', '30m', '--output-format', 'json', capture_output=True, text=True)
        response = json.loads(result.stdout)
        if response.get('status') != 'Accepted':
            raise SystemExit(f'DMG notarization was not accepted: {response.get("status")}')
        run('xcrun', 'stapler', 'staple', str(dmg))
        run('xcrun', 'stapler', 'validate', str(dmg))
        run('codesign', '--verify', '--verbose=2', str(dmg))
    finally:
        run('security', 'list-keychains', '-d', 'user', '-s', *original_keychains)
        if keychain.exists():
            run('security', 'delete-keychain', str(keychain))
