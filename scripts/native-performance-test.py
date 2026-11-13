#!/usr/bin/env python3
"""Validate the native sampler against the OS process CPU clock, not wall time."""
import ctypes
import importlib.util
from pathlib import Path
import os
import platform
import time
import unittest

spec = importlib.util.spec_from_file_location('native_performance', Path(__file__).with_name('native-performance.py'))
profile = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profile)

@unittest.skipUnless(platform.system() == 'Darwin', 'macOS libproc sampler')
class NativeSamplerTests(unittest.TestCase):
    def test_libproc_cpu_ticks_match_process_cpu_clock_after_conversion(self):
        library = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
        library.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        library.proc_pid_rusage.restype = ctypes.c_int
        def sample():
            value = profile.Usage()
            self.assertEqual(library.proc_pid_rusage(os.getpid(), 4, ctypes.byref(value)), 0)
            self.assertGreater(value.resident_size, 0)
            return {'user_time': value.user_time, 'system_time': value.system_time}
        scale = profile.mach_timebase()
        before = sample()
        start = time.process_time_ns()
        while time.process_time_ns() - start < 50_000_000:
            pass
        expected = time.process_time_ns() - start
        actual = profile.cpu_nanoseconds(before, sample(), scale)
        self.assertLess(abs(actual / expected - 1), 0.05)
        print(f'libproc CPU {actual:.0f} ns; process clock {expected} ns; timebase {scale}')

if __name__ == '__main__':
    unittest.main()
