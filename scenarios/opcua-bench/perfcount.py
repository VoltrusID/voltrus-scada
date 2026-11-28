#!/usr/bin/env python3
"""Count user-space instructions and CPU time of a command and every thread it starts, over a window chosen at run time.

    perfcount.py <out.json> -- <command> [args...]

The counters start disabled. SIGUSR1 enables them, SIGUSR2 disables them, SIGTERM stops the command (SIGTERM, then
SIGKILL after 10 s), waits for it and writes {"instructions": ..., "task_clock_ns": ...} to out.json. Threads that are
still running only hand their counts to the parent counter when they exit, so the totals are read after the command
has ended. Same idea as `perf stat --delay`, without perf: perf_event_open(2) on our own child is allowed at
kernel.perf_event_paranoid <= 2 (user space only, so the kernel's share is not counted).
"""
import ctypes
import json
import os
import signal
import struct
import sys
import time

libc = ctypes.CDLL(None, use_errno=True)
SYS_perf_event_open = {'x86_64': 298, 'aarch64': 241}[os.uname().machine]
PERF_EVENT_IOC_ENABLE, PERF_EVENT_IOC_DISABLE = 0x2400, 0x2401
# flags bits: disabled(0) inherit(1) exclude_kernel(5) exclude_hv(6)
FLAGS = 1 | 2 | 32 | 64
EVENTS = {'instructions': (0, 1), 'task_clock_ns': (1, 1)}  # (PERF_TYPE_HARDWARE, INSTRUCTIONS), (PERF_TYPE_SOFTWARE, TASK_CLOCK)


def open_counter(pid, typ, config):
    # perf_event_attr, version 0 layout (64 bytes): type, size, config, sample_period, sample_type, read_format, flags, ...
    attr = struct.pack('IIQQQQQIIQ', typ, 64, config, 0, 0, 0, FLAGS, 0, 0, 0)
    buf = ctypes.create_string_buffer(attr, 64)
    fd = libc.syscall(SYS_perf_event_open, buf, pid, -1, -1, 0)
    if fd < 0:
        raise OSError(ctypes.get_errno(), f'perf_event_open: {os.strerror(ctypes.get_errno())}')
    return fd


def main():
    out, cmd = sys.argv[1], sys.argv[sys.argv.index('--') + 1:]
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.close(w)
        os.read(r, 1)  # wait until the counters are attached
        os.execvp(cmd[0], cmd)
    os.close(r)
    fds = {name: open_counter(pid, *ev) for name, ev in EVENTS.items()}
    os.write(w, b'x')
    os.close(w)
    state = {'stop': False}

    def ctl(req):
        for fd in fds.values():
            libc.ioctl(fd, req, 0)

    signal.signal(signal.SIGUSR1, lambda *_: ctl(PERF_EVENT_IOC_ENABLE))
    signal.signal(signal.SIGUSR2, lambda *_: ctl(PERF_EVENT_IOC_DISABLE))
    signal.signal(signal.SIGTERM, lambda *_: state.update(stop=True))
    signal.signal(signal.SIGINT, lambda *_: state.update(stop=True))
    while not state['stop']:
        done, _ = os.waitpid(pid, os.WNOHANG)
        if done:
            break
        time.sleep(0.1)
    else:
        ctl(PERF_EVENT_IOC_DISABLE)
        os.kill(pid, signal.SIGTERM)
        for _ in range(100):
            if os.waitpid(pid, os.WNOHANG)[0]:
                break
            time.sleep(0.1)
        else:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
    res = {name: struct.unpack('Q', os.read(fd, 8))[0] for name, fd in fds.items()}
    with open(out, 'w') as f:
        json.dump(res, f)
    print(json.dumps(res))


if __name__ == '__main__':
    main()
