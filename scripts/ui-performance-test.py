#!/usr/bin/env python3
"""Offline tests for performance-result parsing and regression classification."""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('ui_performance', Path(__file__).with_name('ui-performance.py'))
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def runs(values):
    return [{name: {'p95': value} for name in bench.EXPECTED} for value in values]


class ComparisonTests(unittest.TestCase):
    def test_repeatable_slowdown_fails(self):
        rows = bench.compare(runs([1] * 5), runs([1.20] * 5))
        self.assertEqual({row['status'] for row in rows}, {'regression'})

    def test_speedup_and_small_variation_pass(self):
        for candidate in ([0.8] * 5, [1.01, 0.99, 1.02, 1.03, 1.01]):
            self.assertEqual({r['status'] for r in bench.compare(runs([1] * 5), runs(candidate))}, {'pass'})

    def test_noisy_measurements_do_not_silently_pass(self):
        self.assertEqual({r['status'] for r in bench.compare(runs([1] * 5), runs([0.5, 0.8, 1, 1.3, 1.6]))}, {'inconclusive'})

    def test_inconsistent_slowdown_requires_investigation(self):
        self.assertEqual({r['status'] for r in bench.compare(runs([1] * 5), runs([1, 1, 1.15, 1.15, 1.15]))}, {'inconclusive'})

    def test_missing_cases_and_too_few_runs_fail(self):
        with self.assertRaises(ValueError):
            bench.compare(runs([1] * 4), runs([1] * 4))
        incomplete = runs([1] * 5)
        incomplete[0].pop(next(iter(bench.EXPECTED)))
        with self.assertRaises(ValueError):
            bench.compare(incomplete, runs([1] * 5))

    def test_legacy_output_is_supported_without_invented_p99(self):
        parsed = bench.parse_output('''wide grid vertical precise CPU frame ms: p50=0.200 p95=0.300
  wheel-event layout p95=0.310 ms; tessellation p95=0.050 ms
workspace selection 120 columns inspector=true: CPU layout+tessellation p50=0.4 p95=0.5 ms
sidebar 2000 objects p50=0.05 p95=0.06 ms''')
        self.assertEqual(parsed['grid/vertical precise/wheel_layout']['p95'], 0.31)
        self.assertEqual(parsed['workspace/120/true']['p95'], 0.5)
        self.assertNotIn('p99', parsed['sidebar/2000'])

    def test_structured_output_retains_tails_and_overrides_rounded_logs(self):
        row = dict(name='sidebar/20', p95=0.0456, p99=0.07, max=0.09, samples=100)
        output = 'sidebar 20 objects p50=0.04 p95=0.046 ms\ntest app::tests::sidebar ... RUSTABLE_UI_BENCH ' + json.dumps(row)
        self.assertEqual(bench.parse_output(output)['sidebar/20'], row)
        with self.assertRaises(ValueError):
            bench.parse_output(output + '\nRUSTABLE_UI_BENCH ' + json.dumps(row))

    def test_invalid_measurements_rejected(self):
        for value in [0, -1, float('nan'), float('inf')]:
            with self.assertRaises(ValueError):
                bench.parse_output('RUSTABLE_UI_BENCH ' + json.dumps(dict(name='sidebar/20', p95=value)))


if __name__ == '__main__':
    unittest.main()
