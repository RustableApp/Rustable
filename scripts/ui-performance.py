#!/usr/bin/env python3
"""Compare opt-in CPU UI benchmarks on one machine; never accesses a user database."""
import argparse
import datetime
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import tempfile

BENCHMARKS = (
    'wide_grid_scroll_frame_benchmark',
    'wide_workspace_selection_frame_benchmark',
    'populated_sidebar_frame_benchmark',
)
EXPECTED = {
    *(f'grid/{direction} {device}/{metric}' for direction in ('vertical', 'horizontal')
      for device in ('precise', 'mouse') for metric in ('layout', 'wheel_layout', 'tessellation')),
    *(f'workspace/{columns}/{inspector}' for columns in (12, 120) for inspector in ('false', 'true')),
    *(f'sidebar/{count}' for count in (20, 200, 2000)),
}
NUMBER = r'([0-9]+(?:\.[0-9]+)?)'


def parse_output(output):
    structured, legacy = {}, {}
    grid = None
    for line in output.splitlines():
        if 'RUSTABLE_UI_BENCH ' in line:
            # libtest may prefix the first output line with the test name.
            record = json.loads(line.split('RUSTABLE_UI_BENCH ', 1)[1])
            name = record['name']
            if name in structured:
                raise ValueError(f'duplicate benchmark: {name}')
            structured[name] = record
        match = re.search(r'wide grid (.+) CPU frame ms: p50=' + NUMBER + r' p95=' + NUMBER, line)
        if match:
            grid = match[1]
            legacy[f'grid/{grid}/layout'] = {'p50': float(match[2]), 'p95': float(match[3])}
        match = re.search(r'wheel-event layout p95=' + NUMBER + r' ms; tessellation p95=' + NUMBER, line)
        if match:
            if grid is None:
                raise ValueError('grid detail without grid name')
            legacy[f'grid/{grid}/wheel_layout'] = {'p95': float(match[1])}
            legacy[f'grid/{grid}/tessellation'] = {'p95': float(match[2])}
        match = re.search(r'workspace selection (\d+) columns inspector=(true|false): CPU layout\+tessellation p50=' + NUMBER + r' p95=' + NUMBER, line)
        if match:
            legacy[f'workspace/{match[1]}/{match[2]}'] = {'p50': float(match[3]), 'p95': float(match[4])}
        match = re.search(r'sidebar (\d+) objects p50=' + NUMBER + r' p95=' + NUMBER, line)
        if match:
            legacy[f'sidebar/{match[1]}'] = {'p50': float(match[2]), 'p95': float(match[3])}
    records = structured or legacy
    # An old revision has only rounded p50/p95 logs. Do not invent tail samples.
    for name, record in records.items():
        for metric in ('p50', 'p95', 'p99', 'max'):
            if metric in record and (not math.isfinite(record[metric]) or record[metric] <= 0):
                raise ValueError(f'invalid or insufficiently precise {metric}: {name}')
    return records


def compare(base_runs, head_runs):
    if len(base_runs) != len(head_runs) or len(base_runs) < 5:
        raise ValueError('at least five paired runs required')
    for run in base_runs + head_runs:
        if set(run) != EXPECTED:
            raise ValueError(f'benchmark coverage changed: missing={EXPECTED-set(run)}, extra={set(run)-EXPECTED}')
    result = []
    for name in sorted(EXPECTED):
        base = [run[name]['p95'] for run in base_runs]
        head = [run[name]['p95'] for run in head_runs]
        b, h = statistics.median(base), statistics.median(head)
        ratios = [after / before for before, after in zip(base, head)]
        def spread(values):
            median = statistics.median(values)
            return statistics.median(abs(value - median) for value in values) / median
        noisy = max(spread(base), spread(head), spread(ratios)) > 0.10
        slower = sum(ratio > 1.10 for ratio in ratios)
        if h > b * 1.10 and slower >= math.ceil(len(ratios) * 0.8) and not noisy:
            status = 'regression'
        elif noisy or h > b * 1.10:
            status = 'inconclusive'
        else:
            status = 'pass'
        result.append(dict(name=name, base_p95_ms=b, head_p95_ms=h,
                           change_percent=(h / b - 1) * 100, status=status,
                           slower_pairs=slower, paired_ratios=ratios))
    return result


def run(command, cwd, **kwargs):
    return subprocess.run(command, cwd=cwd, check=True, text=True, **kwargs)


def build(source, target, logs, label):
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    log = logs / f'{label}-build.log'
    with log.open('w') as output:
        completed = run(['mise', 'exec', '--', 'cargo', 'test', '--locked', '--release', '--lib',
                         '--no-run', '--message-format=json'], source, env=env,
                        stdout=subprocess.PIPE, stderr=output)
    executable = None
    for line in completed.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (message.get('reason') == 'compiler-artifact' and message.get('executable')
                and message.get('target', {}).get('name') == 'rustable'
                and message.get('profile', {}).get('test')):
            executable = message['executable']
    if not executable:
        raise RuntimeError(f'no library test executable found; see {log}')
    destination = logs / (label + ('-tests.exe' if os.name == 'nt' else '-tests'))
    shutil.copy2(executable, destination)
    return destination


def measure(binary, source, logs, label, iteration):
    records = {}
    for benchmark in BENCHMARKS:
        completed = run([str(binary), benchmark, '--ignored', '--nocapture', '--test-threads=1'],
                        source, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        (logs / f'{label}-{iteration}-{benchmark}.log').write_text(completed.stdout)
        parsed = parse_output(completed.stdout)
        if records.keys() & parsed.keys():
            raise ValueError('duplicate benchmark cases')
        records.update(parsed)
    if set(records) != EXPECTED:
        raise ValueError(f'{label}: expected all {len(EXPECTED)} benchmark cases; got {len(records)}')
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--redis', action='store_true', help='Compare the same Redis grid fixture on both revisions')
    parser.add_argument('--redis-connections', action='store_true', help='Include the shared Redis connection profile workload')
    parser.add_argument('--redis-pages', action='store_true', help='Include Search, Analyze and Pub/Sub page workloads')
    parser.add_argument('--redis-editor', action='store_true', help='Include Redis command editor idle/edit workloads')
    parser.add_argument('--redis-completion', action='store_true', help='Compare focused command completion frames using the identical fixture')
    parser.add_argument('--redis-tools', action='store_true', help='Include shared Redis diagnostics and topology workloads')
    parser.add_argument('--base', required=True, help='Git revision to compare against')
    parser.add_argument('--output', default='target/ui-performance')
    parser.add_argument('--runs', type=int, default=5)
    args = parser.parse_args()
    global BENCHMARKS
    if args.redis:
        BENCHMARKS += ('redis_key_grid_frame_benchmark', 'redis_hash_inspector_frame_benchmark', 'redis_string_inspector_frame_benchmark', 'redis_browser_chrome_frame_benchmark')
        EXPECTED.update(('redis/grid', 'redis/hash-inspector', 'redis/string-inspector', 'redis/browser-chrome'))
    if args.redis_pages:
        BENCHMARKS += ('redis_tool_pages_frame_benchmark',)
        EXPECTED.update(f'redis/page/{page}' for page in ('search', 'analysis', 'pubsub', 'workbench-history', 'json-edit'))
    if args.redis_editor:
        BENCHMARKS += ('redis_command_pane_frame_benchmark',)
        EXPECTED.update(f'redis/command-editor/{count}/{editing}' for count in (20, 200) for editing in ('false', 'true'))
    if args.redis_completion:
        BENCHMARKS += ('redis_completion_frame_benchmark',)
        EXPECTED.update(f'redis/completion/{case}' for case in ('commands', 'arguments', 'after-idle'))
    if args.runs < 5:
        parser.error('--runs must be at least 5')
    root = Path(run(['git', 'rev-parse', '--show-toplevel'], Path.cwd(), capture_output=True).stdout.strip())
    if args.redis_connections:
        BENCHMARKS += ('redis_connection_profile_frame_benchmark',)
        EXPECTED.add('redis/connection-profile')
    if args.redis_tools:
        BENCHMARKS += ('redis_diagnostics_frame_benchmark', 'redis_topology_frame_benchmark')
        EXPECTED.update(('redis/diagnostics', 'redis/topology'))
    base_sha = run(['git', 'rev-parse', '--verify', args.base + '^{commit}'], root, capture_output=True).stdout.strip()
    head_sha = run(['git', 'rev-parse', 'HEAD'], root, capture_output=True).stdout.strip()
    logs = (root / args.output).resolve()
    if logs.exists() and any(logs.iterdir()):
        parser.error('output directory must be empty (preserve previous results)')
    logs.mkdir(parents=True, exist_ok=True)
    metadata = dict(base_commit=base_sha, head_commit=head_sha,
                    working_tree=run(['git', 'status', '--porcelain'], root, capture_output=True).stdout,
                    platform=platform.platform(), machine=platform.machine(),
                    cpu=(run(['sysctl', '-n', 'machdep.cpu.brand_string'], root, capture_output=True).stdout.strip()
                         if platform.system() == 'Darwin' else platform.processor()),
                    rustc=run(['mise', 'exec', '--', 'rustc', '-Vv'], root, capture_output=True).stdout,
                    started_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    build_profile='release', runs=args.runs, p95_threshold_percent=10,
                    scope='CPU layout/tessellation only; excludes event loop, GPU and presentation')
    (logs / 'environment.json').write_text(json.dumps(metadata, indent=2))
    base_runs, head_runs = [], []
    with tempfile.TemporaryDirectory(prefix='rustable-ui-base-') as temp:
        base_path = Path(temp) / 'base'
        run(['git', 'worktree', 'add', '--detach', str(base_path), base_sha], root)
        try:
            if args.redis or args.redis_editor or args.redis_pages or args.redis_completion:
                # New workload, identical source on each side; never change the
                # workload in just the candidate and call it a comparison.
                fixture = root / 'src' / 'redis_browser_benchmark.rs'
                shutil.copy2(fixture, base_path / 'src' / fixture.name)
                workspace = base_path / 'src' / 'redis_workspace.rs'
                source = workspace.read_text()
                if 'include!("redis_browser_benchmark.rs")' not in source:
                    workspace.write_text(source + '\ninclude!("redis_browser_benchmark.rs");\n')
                if 'fn prepare_analysis_benchmark(' not in source:
                    with workspace.open('a') as output:
                        output.write('\n#[cfg(test)] impl Workspace { fn prepare_analysis_benchmark(&mut self) {} }\n')
                    metadata['analysis_baseline_adapter'] = 'Same 1,000 names and memory values; candidate adds full report metadata and expiry/type tables.'
                metadata['redis_fixture'] = fixture.read_text()
                (logs / 'environment.json').write_text(json.dumps(metadata, indent=2))
            if args.redis or args.redis_editor or args.redis_pages or args.redis_completion:
                # All shared fixture variants compile the page workload, even
                # when another Redis workload is selected for measurement.
                # The current catalog benchmark represents already loaded data,
                # never an attempted request to the disconnected fixture worker.
                # Older catalogs contain names only and need no metadata cache.
                search = base_path / 'src' / 'redis_search.rs'
                source = search.read_text()
                if 'fn prepare_catalog_benchmark(' not in source:
                    source = source.replace('impl View {', 'impl View {\n    #[cfg(test)]\n    pub(crate) fn prepare_catalog_benchmark(&mut self) { self.catalog_loaded = true; }', 1)
                    search.write_text(source)
                    metadata['search_catalog_baseline_adapter'] = 'Mark names-only catalog loaded; candidate adds cached metadata. No network in either measurement.'
                    (logs / 'environment.json').write_text(json.dumps(metadata, indent=2))
            # Both builds finish before any timing. Separate saved executables
            # prevent the second build from replacing the first benchmark binary.
            # Vendored crates can share Cargo package IDs across worktrees.
            # Never let baseline artifacts satisfy candidate dependencies.
            target = root / 'target' / 'ui-performance-build'
            base_binary = build(base_path, target / 'base', logs, 'base')
            head_binary = build(root, target / 'head', logs, 'head')
            for index in range(args.runs):
                order = ('base', 'head') if index % 2 == 0 else ('head', 'base')
                for label in order:
                    print(f'UI benchmark pair {index + 1}/{args.runs}: {label}', flush=True)
                    rows = measure(base_binary if label == 'base' else head_binary,
                                   base_path if label == 'base' else root, logs, label, index)
                    (base_runs if label == 'base' else head_runs).append(rows)
            comparisons = compare(base_runs, head_runs)
            report = dict(environment=metadata, base_runs=base_runs, head_runs=head_runs, comparisons=comparisons)
            (logs / 'results.json').write_text(json.dumps(report, indent=2))
            lines = ['# UI CPU performance comparison', '', metadata['scope'], '',
                     '| Scenario | Base p95 ms | Candidate p95 ms | Change | Result |',
                     '| --- | ---: | ---: | ---: | --- |']
            for row in comparisons:
                lines.append(f"| {row['name']} | {row['base_p95_ms']:.4f} | {row['head_p95_ms']:.4f} | {row['change_percent']:+.1f}% | {row['status']} |")
            summary = '\n'.join(lines) + '\n'
            (logs / 'summary.md').write_text(summary)
            print(summary)
            if os.environ.get('GITHUB_STEP_SUMMARY'):
                with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as output:
                    output.write(summary)
            return 1 if any(r['status'] == 'regression' for r in comparisons) else 2 if any(r['status'] == 'inconclusive' for r in comparisons) else 0
        finally:
            run(['git', 'worktree', 'remove', '--force', str(base_path)], root)
            # Raw measurements and logs stay; bulky binaries are not CI artifacts.
            for label in ('base', 'head'):
                (logs / (label + ('-tests.exe' if os.name == 'nt' else '-tests'))).unlink(missing_ok=True)


if __name__ == '__main__':
    raise SystemExit(main())
