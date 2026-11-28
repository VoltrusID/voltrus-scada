#!/usr/bin/env python3
"""OPC UA client benchmark: one simulator, one Voltrus server, N tags over D devices.

    bench.py --bin <dir with voltrus and opcua-sim> --tags 1000 --rate-ms 1000 --change-pct 10 [--devices 1]
             [--warmup 20] [--window 60] [--label before] [--out results.jsonl] [--max-items-per-call N]

One run:
  1. starts `opcua-sim --tags N --rate-ms M --change-pct P` (Double tags `Bench.T000000`... and `Bench.SendTime`);
  2. starts `voltrus` under perfcount.py (user-space instructions and CPU time over the window only), with
     VOLTRUS_BENCH_STATS=1 (counters logged once a second) and VOLTRUS_BENCH_LATENCY_TAG=send_time;
  3. signs in, imports the devices through POST /api/v1/project/import (N tags split over D devices, send_time on
     the first), and starts them with POST /api/v1/opcua/devices/{id}/start: the time until every device logs
     "created x/y monitored items" is the connect + create-items time;
  4. waits --warmup seconds, then measures --window seconds between two counter lines of the log: values delivered
     to the cache and dropped, allocations (with --features alloc-count), instructions, CPU, RSS/PSS at the end,
     and the latency of send_time (simulator send time to cache merge) for p50/p99.
Prints one JSON line and appends it to --out. Needs Linux, kernel.perf_event_paranoid <= 2, python3.
"""
import argparse
import http.cookiejar
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
# Before the coalescing feed the client dropped values on a full queue; after it, it counts superseded ones.
STATS = re.compile(r'opcua_bench delivered=(\d+) (?:dropped|superseded)=(\d+) allocs=(\d+)')
LATENCY = re.compile(r'opcua_bench latency_ms=(-?[\d.]+)')
CREATED = re.compile(r'OPC-UA device (\S+) created (\d+)/(\d+) monitored items|OPC-UA device (\S+) error: (.*)')


class StopRun(Exception):
    """The run cannot be measured (the error is in the result)."""


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


class Api:
    def __init__(self, base):
        self.base = base
        self.op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    def call(self, method, path, body=None, raw=None, ctype='application/json'):
        data = raw.encode() if raw is not None else (None if body is None else json.dumps(body).encode())
        req = urllib.request.Request(self.base + path, method=method, data=data, headers={'Content-Type': ctype})
        with self.op.open(req, timeout=600) as r:
            out = r.read()
        return json.loads(out) if out else None


def proc_sample(pid):
    with open(f'/proc/{pid}/stat') as f:
        fields = f.read().rsplit(')', 1)[1].split()
    ticks = int(fields[11]) + int(fields[12])  # utime + stime
    rss = pss = 0
    with open(f'/proc/{pid}/status') as f:
        for line in f:
            if line.startswith('VmRSS:'):
                rss = int(line.split()[1])
    with open(f'/proc/{pid}/smaps_rollup') as f:
        for line in f:
            if line.startswith('Pss:'):
                pss = int(line.split()[1])
    return ticks, rss / 1024, pss / 1024


def child_pid(ppid):
    for _ in range(100):
        try:
            with open(f'/proc/{ppid}/task/{ppid}/children') as f:
                kids = f.read().split()
            if kids:
                return int(kids[0])
        except FileNotFoundError:
            pass
        time.sleep(0.05)
    raise SystemExit('voltrus did not start')


class Log:
    """Follows a log file; lines are handed out once, in order."""

    def __init__(self, path):
        self.path, self.pos, self.buf, self.queue = path, 0, '', []

    def lines(self):
        """Every line not handed out yet."""
        with open(self.path, errors='ignore') as f:
            f.seek(self.pos)
            chunk = f.read()
            self.pos = f.tell()
        self.buf += chunk
        *done, self.buf = self.buf.split('\n')
        out, self.queue = self.queue + done, []
        return out

    def wait_for(self, regex, timeout, on_line=None):
        """The next line matching `regex`; the lines after it stay for the next call."""
        end = time.time() + timeout
        while time.time() < end:
            lines = self.lines()
            for i, line in enumerate(lines):
                if on_line:
                    on_line(line)
                m = regex.search(line)
                if m:
                    self.queue = lines[i + 1:]
                    return m
            time.sleep(0.02)
        raise SystemExit(f'timed out waiting for {regex.pattern!r} in {self.path}')


def project_yaml(url, tags, devices, rate_ms):
    """N monitored items in all, split over the devices; send_time takes the place of the last data tag on the
    first device, so a device of 1,000 tags is 1,000 monitored items."""
    per = [tags // devices + (1 if d < tags % devices else 0) for d in range(devices)]
    out, k = ['format: voltrus-project/1', 'opcua_devices:'], 0
    for d, n in enumerate(per):
        out += [f'- name: bench{d}', f'  url: {url}', f'  interval_ms: {rate_ms}', '  enabled: true', '  nodes:']
        if d == 0:
            out.append("  - {node_id: 'ns=NS;s=Bench.SendTime', name: send_time}")
            n -= 1
        for _ in range(n):
            out.append(f"  - {{node_id: 'ns=NS;s=Bench.T{k:06d}', name: t{k:06d}}}")
            k += 1
    return '\n'.join(out) + '\n'


def pct(xs, p):
    if not xs:
        return None
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(round(p / 100 * (len(xs) - 1))))], 1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--bin', required=True)
    ap.add_argument('--tags', type=int, required=True)
    ap.add_argument('--rate-ms', type=int, default=1000)
    ap.add_argument('--change-pct', type=float, default=10)
    ap.add_argument('--devices', type=int, default=1)
    ap.add_argument('--warmup', type=int, default=20)
    ap.add_argument('--window', type=int, default=60)
    ap.add_argument('--max-items-per-call', type=int)
    ap.add_argument('--label', default='')
    ap.add_argument('--out')
    ap.add_argument('--keep', action='store_true', help='keep the work directory')
    a = ap.parse_args()

    work = tempfile.mkdtemp(prefix='opcua-bench-')
    sim_port, http_port = free_port(), free_port()
    procs = []
    result = {'label': a.label, 'tags': a.tags, 'rate_ms': a.rate_ms, 'change_pct': a.change_pct,
              'devices': a.devices, 'window_s': a.window}
    try:
        sim_cmd = [os.path.join(a.bin, 'opcua-sim'), '--port', str(sim_port), '--tags', str(a.tags),
                   '--rate-ms', str(a.rate_ms), '--change-pct', str(a.change_pct)]
        if a.max_items_per_call:
            sim_cmd += ['--max-items-per-call', str(a.max_items_per_call)]
        sim_log = os.path.join(work, 'sim.log')
        sim = subprocess.Popen(sim_cmd, cwd=work, stdout=open(sim_log, 'w'), stderr=subprocess.STDOUT,
                               env={**os.environ, 'NO_COLOR': '1'}, start_new_session=True)
        procs.append(sim)
        ns = Log(sim_log).wait_for(re.compile(r'has index (\d+)'), 120).group(1)

        with open(os.path.join(work, 'voltrus.yaml'), 'w') as f:
            f.write(f'server_port: {http_port}\nserver_host: "127.0.0.1"\ndb_path: "bench.db"\n'
                    'cookie_secure: false\ndevices: []\n')
        srv_log = os.path.join(work, 'server.log')
        env = {**os.environ, 'NO_COLOR': '1', 'VOLTRUS_BENCH_STATS': '1', 'VOLTRUS_BENCH_LATENCY_TAG': 'send_time'}
        counter_out = os.path.join(work, 'perf.json')
        pc = subprocess.Popen([sys.executable, '-I', os.path.join(HERE, 'perfcount.py'), counter_out, '--',
                               os.path.join(a.bin, 'voltrus'), 'voltrus.yaml'],
                              cwd=work, stdout=open(srv_log, 'w'), stderr=subprocess.STDOUT, env=env,
                              start_new_session=True)
        procs.append(pc)
        vpid = child_pid(pc.pid)
        api = Api(f'http://127.0.0.1:{http_port}')
        for _ in range(600):
            try:
                urllib.request.urlopen(api.base + '/', timeout=1)
                break
            except (urllib.error.URLError, ConnectionError):
                time.sleep(0.05)
        pw = open(os.path.join(work, 'initial-admin-password')).read().strip()
        r = api.call('POST', '/api/v1/auth/login', {'username': 'admin', 'password': pw})
        if r.get('must_change_password'):
            api.call('POST', '/api/v1/auth/change-password', {'old_password': pw, 'new_password': 'bench-Pa55word!'})
            api.call('POST', '/api/v1/auth/login', {'username': 'admin', 'password': 'bench-Pa55word!'})

        yaml = project_yaml(f'opc.tcp://127.0.0.1:{sim_port}', a.tags, a.devices, a.rate_ms).replace('NS', ns)
        t = time.time()
        api.call('POST', '/api/v1/project/import', raw=yaml, ctype='application/yaml')
        result['import_s'] = round(time.time() - t, 2)
        devices = [d for d in api.call('GET', '/api/v1/opcua/devices') if d['name'].startswith('bench')]

        log = Log(srv_log)
        log.lines()
        t0 = time.time()
        for d in devices:
            api.call('POST', f"/api/v1/opcua/devices/{d['id']}/start")
        created, failed = {}, 0
        while len(created) < len(devices):
            m = log.wait_for(CREATED, 300)
            if m.group(4):
                result['error'] = m.group(5).strip()[:300]
                raise StopRun
            created[m.group(1)] = (int(m.group(2)), int(m.group(3)))
        result['connect_create_s'] = round(time.time() - t0, 2)
        result['items_created'] = sum(c for c, _ in created.values())
        result['items_requested'] = sum(n for _, n in created.values())

        time.sleep(a.warmup)
        # Start and end the window right after a counter line, so its counts match the window.
        lat = []
        log.lines()  # skip what the warm-up logged
        stats0 = log.wait_for(STATS, 10)
        os.kill(pc.pid, signal.SIGUSR1)
        t_start = time.time()
        p0 = proc_sample(vpid)
        sp0 = proc_sample(sim.pid)
        def latency(line):
            m = LATENCY.search(line)
            if m:
                lat.append(float(m.group(1)))

        while time.time() < t_start + a.window - 1.5:
            for line in log.lines():
                latency(line)
            time.sleep(0.2)
        for line in log.lines():  # the next counter line closes the window
            latency(line)
        stats1 = log.wait_for(STATS, 10, on_line=latency)
        os.kill(pc.pid, signal.SIGUSR2)
        elapsed = time.time() - t_start
        p1 = proc_sample(vpid)
        sp1 = proc_sample(sim.pid)
        hz = os.sysconf('SC_CLK_TCK')

        d0, d1 = [int(x) for x in stats0.groups()], [int(x) for x in stats1.groups()]
        delivered, lost, allocs = (d1[i] - d0[i] for i in range(3))
        dropped, superseded = (lost, 0) if 'dropped=' in stats1.group(0) else (0, lost)
        # Changes per tick among the monitored tags: all tags but the last one, plus send_time.
        per_tick = min(a.tags, -(-a.tags * a.change_pct // 100)) * (a.tags - 1) / a.tags + 1
        result.update({
            'measured_s': round(elapsed, 2),
            'produced_per_s': round(per_tick * 1000 / a.rate_ms, 1),
            'delivered_per_s': round(delivered / elapsed, 1),
            'dropped': dropped,
            'superseded': superseded,
            'allocs_per_value': round(allocs / delivered, 2) if delivered and allocs else None,
            'cpu_pct_noisy': round((p1[0] - p0[0]) / hz / elapsed * 100, 2),
            'rss_mib': round(p1[1], 1),
            'pss_mib': round(p1[2], 1),
            'sim_cpu_pct': round((sp1[0] - sp0[0]) / hz / elapsed * 100, 2),
            'latency_p50_ms': pct(lat, 50),
            'latency_p99_ms': pct(lat, 99),
            'latency_samples': len(lat),
        })
        pc.send_signal(signal.SIGTERM)
        pc.wait(timeout=60)
        counts = json.load(open(counter_out))
        result['instructions_per_value'] = round(counts['instructions'] / delivered) if delivered else None
        result['instructions_per_s'] = round(counts['instructions'] / elapsed)
        result['cpu_pct_task_clock'] = round(counts['task_clock_ns'] / 1e9 / elapsed * 100, 2)
    except StopRun:
        pass
    finally:
        for p in procs:
            if p.poll() is None:
                try:
                    os.killpg(p.pid, signal.SIGTERM)
                    p.wait(timeout=20)
                except (ProcessLookupError, subprocess.TimeoutExpired):
                    os.killpg(p.pid, signal.SIGKILL)
        if a.keep:
            print('work directory:', work, file=sys.stderr)
        else:
            shutil.rmtree(work, ignore_errors=True)
    line = json.dumps(result)
    print(line)
    if a.out:
        with open(a.out, 'a') as f:
            f.write(line + '\n')


if __name__ == '__main__':
    main()
