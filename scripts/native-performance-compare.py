#!/usr/bin/env python3
"""Build and serially profile identical disposable native fixtures on two revisions."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tempfile


def command(args, cwd, **kwargs):
    return subprocess.run(args, cwd=cwd, check=True, text=True, **kwargs)


def build(source, target, output, label):
    with (output / f'{label}-build.log').open('w') as log:
        result = command(['mise', 'exec', '--', 'cargo', 'build', '--locked', '--release',
                          '--features', 'qa', '--bin', 'rustable', '--message-format=json'], source,
                         env=dict(os.environ, CARGO_TARGET_DIR=str(target)), stdout=subprocess.PIPE, stderr=log)
    binary = None
    for line in result.stdout.splitlines():
        record = json.loads(line)
        if record.get('reason') == 'compiler-artifact' and record.get('executable'):
            binary = Path(record['executable'])
    if binary is None:
        raise RuntimeError('No native executable produced')
    destination = output / f'{label}-native'
    shutil.copy2(binary, destination)
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--seconds', type=int, default=8)
    args = parser.parse_args()
    if args.runs < 3:
        parser.error('At least three native process pairs required')
    root = Path(subprocess.check_output(['git', 'rev-parse', '--show-toplevel'], text=True).strip())
    output = args.output.resolve()
    if output.exists() and any(output.iterdir()):
        parser.error('Output must be empty')
    output.mkdir(parents=True, exist_ok=True)
    base_sha = subprocess.check_output(['git', 'rev-parse', args.base + '^{commit}'], text=True).strip()
    metadata = dict(base=base_sha, head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                    dirty=subprocess.check_output(['git', 'status', '--short'], text=True),
                    platform=platform.platform(), profile='release, qa feature', runs=args.runs,
                    rustc=subprocess.check_output(['mise', 'exec', '--', 'rustc', '-Vv'], text=True),
                    started_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    fixture='500 rows, 16 KiB text per row per result; one, twenty and eleven live results; 1280x820 session windows',
                    limitations='CPU/wakeups/memory and callback startup; not input-to-present, frame intervals, GPU or display validation')
    metadata['fixtures'] = {name: (root / 'src' / name).read_text() for name in ['native_performance.rs', 'native_performance_fixture.rs']}
    (output / 'environment.json').write_text(json.dumps(metadata, indent=2))
    with tempfile.TemporaryDirectory(prefix='rustable-native-base-') as temp:
        base = Path(temp) / 'base'
        command(['git', 'worktree', 'add', '--detach', str(base), base_sha], root)
        try:
            for name in metadata['fixtures']:
                shutil.copy2(root / 'src' / name, base / 'src' / name)
            additions = {
                'app.rs': '\n#[cfg(all(feature = "qa", not(target_arch = "wasm32")))] #[path = "native_performance_fixture.rs"] mod native_performance_fixture;\n',
                'session_windows.rs': '\n#[cfg(feature = "qa")] #[path = "native_performance.rs"] pub(crate) mod native_performance;\n',
            }
            for name, addition in additions.items():
                path = base / 'src' / name
                if addition.split('mod ')[-1].strip() not in path.read_text():
                    with path.open('a') as file:
                        file.write(addition)
            lib = base / 'src/lib.rs'
            source = lib.read_text()
            if 'RUSTABLE_NATIVE_PROFILE' not in source:
                needle = 'pub fn run_native() -> eframe::Result {'
                if needle not in source:
                    raise RuntimeError('Baseline native entry point is incompatible')
                source = source.replace(needle, needle + '\n    #[cfg(feature = "qa")]\n    if std::env::var_os("RUSTABLE_NATIVE_PROFILE").is_some() { return session_windows::native_performance::run(); }', 1)
                lib.write_text(source)
            target = root / 'target/native-performance-build'
            binaries = {side: build(directory, target / side, output, side) for side, directory in [('base', base), ('head', root)]}
            metadata['binaries'] = {side: dict(path=str(path), sha256=hashlib.sha256(path.read_bytes()).hexdigest()) for side, path in binaries.items()}
            (output / 'environment.json').write_text(json.dumps(metadata, indent=2))
            reports = {'base': [], 'head': []}
            for iteration in range(args.runs):
                for side in (['base', 'head'] if iteration % 2 == 0 else ['head', 'base']):
                    destination = output / f'{side}-{iteration}'
                    print(f'Native pair {iteration+1}/{args.runs}: {side}', flush=True)
                    cmd = ['python3', str(root / 'scripts/native-performance.py'), '--binary', str(binaries[side]),
                           '--output', str(destination), '--seconds', str(args.seconds)]
                    command(cmd, root)
                    reports[side].append(json.loads((destination / 'results.json').read_text()))
            expected = {'launcher-idle': 0, 'one-tab': 1, 'tabs20': 20, 'close-tabs': 1,
                        'three-windows': 11, 'close-windows': 1, 'close-last': 0, 'close-all-windows': 0}
            for side in reports:
                for report in reports[side]:
                    if set(expected) != {p['phase'] for p in report['phases']}:
                        raise RuntimeError('Missing native phase')
                    for phase in report['phases']:
                        if phase['after']['live_result_grids'] != expected[phase['phase']]:
                            raise RuntimeError(f'Unexpected retained results: {side}: {phase}')
            (output / 'results.json').write_text(json.dumps(reports, indent=2))
            lines = ['# Native process profile', '', 'Three-pair medians unless a different run count is recorded. No native latency/FPS claim.', '',
                     '| Phase | CPU % base → head | RSS MiB base → head | Footprint MiB base → head | Interrupt wakeups/s base → head |',
                     '| --- | ---: | ---: | ---: | ---: |']
            for name in expected:
                values = []
                for metric, divisor in [('cpu_percent', 1), ('rss_max', 2**20), ('footprint_max', 2**20), ('interrupt_wakeups_per_s', 1)]:
                    pair = []
                    for side in reports:
                        pair.append(statistics.median(next(p for p in r['phases'] if p['phase'] == name)[metric] for r in reports[side]) / divisor)
                    values.append(f'{pair[0]:.3f} → {pair[1]:.3f}')
                lines.append('| ' + name + ' | ' + ' | '.join(values) + ' |')
            (output / 'summary.md').write_text('\n'.join(lines) + '\n')
            print('\n'.join(lines))
        finally:
            command(['git', 'worktree', 'remove', '--force', str(base)], root)

if __name__ == '__main__':
    main()
