#!/usr/bin/env python3
"""Compare identical word/line drag input using the UI runner's release egui builds.

Run ui-performance.py first. This avoids rebuilding the app for the standalone
workload and uses exactly the same baseline/candidate dependency artifacts.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build-root', type=Path, default=Path('target/ui-performance-build'))
    parser.add_argument('--output', type=Path, default=Path('target/native-click-drag-performance'))
    args = parser.parse_args()
    if args.output.exists() and any(args.output.iterdir()):
        parser.error('output directory must be empty (preserve previous results)')
    args.output.mkdir(parents=True, exist_ok=True)
    spec = importlib.util.spec_from_file_location('ui_performance', Path(__file__).with_name('ui-performance.py'))
    perf = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(perf)
    perf.EXPECTED = {'click/drag/2', 'click/drag/3'}
    runs = {'base': [], 'head': []}
    artifacts = {}
    for side in runs:
        deps = args.build_root / side / 'release/deps'
        libraries = list(deps.glob('libegui-*.rlib'))
        if len(libraries) != 1:
            raise RuntimeError(f'Expected exactly one egui artifact in {deps}: {libraries}')
        artifacts[side] = str(libraries[0].resolve())
        subprocess.run([
            'mise', 'exec', '--', 'rustc', '--edition=2024', '-C', 'opt-level=3',
            '-C', 'lto=thin', 'tools/click-drag-benchmark.rs',
            '--extern', f'egui={libraries[0]}', '-L', f'dependency={deps}',
            '-o', str(args.output / f'drag-{side}'),
        ], check=True)
    for index in range(5):
        for side in (['base', 'head'] if index % 2 == 0 else ['head', 'base']):
            output = subprocess.check_output([str(args.output / f'drag-{side}')], text=True)
            (args.output / f'{side}-{index}.log').write_text(output)
            runs[side].append(perf.parse_output(output))
    result = dict(artifacts=artifacts, runs=runs, comparison=perf.compare(runs['base'], runs['head']))
    (args.output / 'results.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result['comparison'], indent=2))
    return int(any(row['status'] == 'regression' for row in result['comparison']))


if __name__ == '__main__':
    raise SystemExit(main())
