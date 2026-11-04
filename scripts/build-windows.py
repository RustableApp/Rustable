#!/usr/bin/env python3
"""Cross-build the app executable on macOS using cargo-xwin's Windows SDK."""
import argparse
import os
from pathlib import Path
import platform
import shlex
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--arch', choices=['arm64', 'x64'], default='arm64')
args = parser.parse_args()
if platform.system() != 'Darwin':
    parser.error('This helper is for macOS; on Windows use cargo build --release.')

root = Path(__file__).resolve().parents[1]
target = {'arm64': 'aarch64', 'x64': 'x86_64'}[args.arch] + '-pc-windows-msvc'
env_target = target.replace('-', '_')
output = subprocess.check_output(
    ['cargo', 'xwin', 'env', '--target', target], cwd=root, text=True
)
env = os.environ.copy()
for line in output.splitlines():
    if not line.startswith('export ') or not line.endswith(';'):
        raise SystemExit(f'Unexpected cargo-xwin environment line: {line!r}')
    assignment, = shlex.split(line[len('export '):-1])
    key, value = assignment.split('=', 1)
    env[key] = value

# ring requires GNU-driver Clang on Windows ARM64, even when cargo-xwin
# selected clang-cl. Use that driver consistently, including SDK include flags.
for prefix, compiler in [('CC', 'clang'), ('CXX', 'clang++')]:
    env[f'{prefix}_{env_target}'] = compiler
    env[f'TARGET_{prefix}'] = compiler
for prefix in ['CFLAGS', 'CXXFLAGS']:
    key = f'{prefix}_{env_target}'
    env[key] = env[key].replace('/imsvc', '-isystem').replace('/EHsc', '-fexceptions')

subprocess.run(
    ['cargo', 'build', '--locked', '--release', '--target', target],
    cwd=root, env=env, check=True,
)
print(root / 'target' / target / 'release' / 'rustable.exe')
