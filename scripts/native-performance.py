#!/usr/bin/env python3
"""Profile an isolated QA Rustable process on macOS, without saved connections.

Uses libproc RUSAGE_INFO_V4 for CPU time, wakeups, RSS and physical footprint.
Native app actions are sent over stdin; sampling does not request repaints.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import queue
import subprocess
import threading
import time

# Public Darwin libproc layout (libc rusage_info_v4), all counters are u64.
FIELDS = '''user_time system_time pkg_idle_wkups interrupt_wkups pageins wired_size
resident_size phys_footprint proc_start_abstime proc_exit_abstime child_user_time
child_system_time child_pkg_idle_wkups child_interrupt_wkups child_pageins
child_elapsed_abstime diskio_bytesread diskio_byteswritten cpu_time_qos_default
cpu_time_qos_maintenance cpu_time_qos_background cpu_time_qos_utility cpu_time_qos_legacy
cpu_time_qos_user_initiated cpu_time_qos_user_interactive billed_system_time
serviced_system_time logical_writes lifetime_max_phys_footprint instructions cycles
billed_energy serviced_energy interval_max_phys_footprint runnable_time'''.split()

class Usage(ctypes.Structure):
    _fields_ = [('uuid', ctypes.c_ubyte * 16)] + [(name, ctypes.c_uint64) for name in FIELDS]


def mach_timebase():
    class Timebase(ctypes.Structure):
        _fields_ = [('numer', ctypes.c_uint32), ('denom', ctypes.c_uint32)]
    system = ctypes.CDLL('/usr/lib/libSystem.B.dylib')
    system.mach_timebase_info.argtypes = [ctypes.POINTER(Timebase)]
    value = Timebase()
    if system.mach_timebase_info(ctypes.byref(value)) != 0 or not value.denom:
        raise RuntimeError('Cannot obtain Mach CPU time conversion')
    return {'numer': value.numer, 'denom': value.denom}


def cpu_nanoseconds(first, last, timebase):
    ticks = last['user_time'] + last['system_time'] - first['user_time'] - first['system_time']
    return ticks * timebase['numer'] / timebase['denom']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--seconds', type=int, default=8, help='Settled sampling duration per phase')
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        parser.error('This sampler uses macOS libproc; no portable RSS/CPU claims are made')
    if args.seconds < 5:
        parser.error('Use at least five seconds per settled phase')
    binary = args.binary.resolve()
    output = args.output.resolve()
    if output.exists() and any(output.iterdir()):
        parser.error('Output must be empty; preserve earlier evidence')
    output.mkdir(parents=True, exist_ok=True)
    library = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
    library.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
    library.proc_pid_rusage.restype = ctypes.c_int
    timebase = mach_timebase()
    events = queue.Queue()
    env = dict(os.environ, RUSTABLE_NATIVE_PROFILE=str(output))
    records = {'environment': {
        'platform': platform.platform(), 'binary': str(binary),
        'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'hardware': subprocess.check_output(['sysctl', '-n', 'machdep.cpu.brand_string'], text=True).strip(),
        'sampling_seconds': args.seconds, 'settle_seconds': 2,
        'scope': 'Native QA process; synthetic results, real session/tab lifecycle; no user databases',
        'cpu_unit': 'percent of one CPU core; libproc user+system Mach ticks converted to nanoseconds',
        'mach_timebase': timebase,
        'wakeups': 'libproc interrupt and package-idle wakeup counters, not UI frames',
    }, 'phases': []}
    stderr = (output / 'stderr.log').open('w')
    started = time.monotonic()
    process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=stderr, text=True, bufsize=1, env=env)

    def read_events():
        with (output / 'events.log').open('w') as log:
            for line in process.stdout:
                log.write(line)
                log.flush()
                if line.startswith('RUSTABLE_NATIVE_PROFILE '):
                    events.put(json.loads(line.split(' ', 1)[1]))
        events.put({'event': 'process-exit'})

    threading.Thread(target=read_events, daemon=True).start()

    def receive(name, timeout=45):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            try:
                event = events.get(timeout=min(1, max(0.01, end - time.monotonic())))
            except queue.Empty:
                if process.poll() is not None:
                    raise RuntimeError(f'Native fixture exited with {process.returncode}')
                continue
            if event.get('event') == 'process-exit':
                raise RuntimeError('Native fixture exited before expected event')
            if event.get('event') == name:
                return event
        raise TimeoutError(f'No {name} from native fixture; see events/stderr logs')

    def command(name):
        process.stdin.write(name + '\n')
        process.stdin.flush()

    def sample():
        usage = Usage()
        if library.proc_pid_rusage(process.pid, 4, ctypes.byref(usage)) != 0:
            raise OSError(ctypes.get_errno(), 'proc_pid_rusage failed')
        return dict(elapsed_s=time.monotonic() - started,
                    **{name: getattr(usage, name) for name in FIELDS})

    try:
        ready = receive('ready')
        records['startup'] = dict(ready=ready, launch_to_first_update_s=time.monotonic() - started,
                                  usage=sample())
        for phase in ['launcher-idle', 'one-tab', 'tabs20', 'close-tabs', 'three-windows', 'close-windows', 'close-last', 'close-all-windows']:
            action_started = time.monotonic()
            command(phase)
            event = receive('phase')
            action_seconds = time.monotonic() - action_started
            if event['phase'] != phase:
                raise RuntimeError(f'Unexpected phase: {event}')
            time.sleep(2)
            command('snapshot')
            before = receive('snapshot')
            # Telemetry reads stay on the fixture's background command thread.
            samples = [sample()]
            for _ in range(args.seconds):
                time.sleep(1)
                samples.append(sample())
            command('snapshot')
            after = receive('snapshot')
            first, last = samples[0], samples[-1]
            duration = last['elapsed_s'] - first['elapsed_s']
            cpu_ns = cpu_nanoseconds(first, last, timebase)
            result = {
                'phase': phase, 'event': event, 'before': before, 'after': after,
                'action_seconds': action_seconds,
                'duration_s': duration, 'cpu_percent': cpu_ns / 1e9 / duration * 100,
                'interrupt_wakeups_per_s': (last['interrupt_wkups'] - first['interrupt_wkups']) / duration,
                'package_idle_wakeups_per_s': (last['pkg_idle_wkups'] - first['pkg_idle_wkups']) / duration,
                'wrapper_updates': max(0, after['updates'] - before['updates']),
                'rss_min': min(s['resident_size'] for s in samples),
                'rss_max': max(s['resident_size'] for s in samples),
                'footprint_min': min(s['phys_footprint'] for s in samples),
                'footprint_max': max(s['phys_footprint'] for s in samples),
                'samples': samples,
            }
            records['phases'].append(result)
            (output / 'results.json').write_text(json.dumps(records, indent=2))
            print(f"{phase}: CPU {result['cpu_percent']:.3f}%, RSS {result['rss_max']/2**20:.1f} MiB, "
                  f"footprint {result['footprint_max']/2**20:.1f} MiB, updates {result['wrapper_updates']}", flush=True)
        command('quit')
        process.wait(timeout=15)
        if process.returncode:
            raise RuntimeError(f'Native fixture exited with {process.returncode}')
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        stderr.close()
    (output / 'results.json').write_text(json.dumps(records, indent=2))

if __name__ == '__main__':
    main()
