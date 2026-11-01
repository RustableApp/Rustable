#!/usr/bin/env python3
"""Create the Finder drag-to-Applications disk image from a signed app bundle.

Build-only dependencies: ds-store==1.3.3 and mac-alias==2.2.3.
Signing/notarization of the resulting DMG is performed by sign-macos-release.py.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile
from ds_store import DSStore

parser = argparse.ArgumentParser()
parser.add_argument('app', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
app = args.app.resolve()
output = args.output.resolve()
if not app.is_dir() or app.name != 'Rustable.app':
    raise SystemExit('Expected an existing Rustable.app bundle')
output.parent.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='rustable-dmg-') as tmp:
    stage = Path(tmp) / 'stage'
    stage.mkdir()
    subprocess.run(['ditto', str(app), str(stage / app.name)], check=True)
    (stage / 'Applications').symlink_to('/Applications', target_is_directory=True)
    with DSStore.open(str(stage / '.DS_Store'), 'w+') as store:
        store['.']['bwsp'] = {'ShowStatusBar': False, 'ShowToolbar': False,
            'ShowSidebar': False, 'ShowPathbar': False, 'ShowTabView': False,
            'WindowBounds': '{{180, 180}, {600, 340}}', 'ContainerShowSidebar': False}
        store['.']['icvp'] = {'viewOptionsVersion': 1, 'backgroundType': 0,
            'iconSize': 96.0, 'textSize': 14.0, 'labelOnBottom': True,
            'showItemInfo': False, 'showIconPreview': True, 'arrangeBy': 'none',
            'gridOffsetX': 0.0, 'gridOffsetY': 0.0, 'gridSpacing': 100.0}
        store['.']['vSrn'] = ('long', 1)
        store['.']['vstl'] = ('type', b'icnv')
        store['Rustable.app']['Iloc'] = (155, 155)
        store['Applications']['Iloc'] = (445, 155)
    subprocess.run(['hdiutil', 'create', '-volname', 'Rustable — Drag to Applications',
        '-srcfolder', str(stage), '-format', 'UDZO', '-fs', 'HFS+', '-ov', str(output)], check=True)
    subprocess.run(['hdiutil', 'verify', str(output)], check=True)
