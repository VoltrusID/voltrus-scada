"""Line 2, a PET bottle packaging line whose four machines each have a different PLC, served on each PLC's own protocol.

    M1 Filler       Siemens S7-1500 (CPU 1515-2 PN)   S7 (ISO-on-TCP), DB1; also the line master (line OEE, downtime reasons)
    M2 Capper       Omron NJ501                       EtherNet/IP, CIP symbolic tags (Capper_*)
    M3 Labeller     Mitsubishi iQ-R (R04ENCPU)        Modbus TCP, D5000 onwards, 32-bit values low word first
    M4 Case packer  Siemens S7-1200 (CPU 1214C)       S7 (ISO-on-TCP), DB1; DB20 holds the Sinamics G120 conveyor drive

plant-sim.py runs the model and the S7 and EtherNet/IP servers; the Mitsubishi block is served by its Modbus server.
seed.py creates the four devices in Voltrus (two S7, one EtherNet/IP, one Modbus) and the 'Packaging Line' screen.

The machines are linked by accumulation buffers: a stop downstream fills the buffer and blocks the machines
upstream, a stop upstream starves the ones after it. OEE (availability x performance x quality) runs over the
current shift (06:00, 14:00, 22:00 WIB) and starts as if the shift had been running when the simulator started.
"""
import asyncio
import math
import random
import re
import struct
import time

# Stop codes, shown on the screen next to each machine.
STOP_CODES = {0: 'running', 10: 'starved', 20: 'blocked', 31: 'jam', 32: 'material out', 41: 'operator stop'}

# key, label, PLC, protocol line, rated speed (bottles/min), reject rate, A and P the shift starts with,
# fault schedule (period s, duration s, offset s, stop code)
MACHINES = [
    ('m1', 'M1 Filler', 'Siemens S7-1500 · CPU 1515-2 PN', 'S7 ISO-on-TCP · DB1', 300, 0.003, 0.91, 0.94, (660, 35, 300, 31)),
    ('m2', 'M2 Capper', 'Omron NJ501-1300', 'EtherNet/IP · CIP tags', 330, 0.005, 0.93, 0.92, (480, 40, 120, 32)),
    ('m3', 'M3 Labeller', 'Mitsubishi iQ-R · R04ENCPU', 'Modbus TCP · D5000', 330, 0.008, 0.88, 0.93, (360, 55, 200, 32)),
    ('m4', 'M4 Case packer', 'Siemens S7-1200 · CPU 1214C', 'S7 ISO-on-TCP · DB1', 336, 0.002, 0.90, 0.92, (540, 45, 420, 31)),
]
BOTTLES_PER_CASE = 24
BUFFER = [150, 150, 300]   # accumulation between M1-M2, M2-M3, M3-M4 (bottles)
SHIFT_PLAN = 8 * 60 * 300 * 0.85

# key -> (name, unit, min, max, warn, crit, decimals, low_warn, low_crit), as in verticals.META
META: dict[str, tuple] = {}


def _meta(key, name, unit='', lo=0, hi=100, warn=0, crit=0, dec=1, lw=None, lc=None):
    assert re.fullmatch(r'[A-Za-z0-9_ .-]+', name), name
    META[key] = (name, unit, lo, hi, warn, crit, dec, lw, lc)


for k, label, *_ in MACHINES:
    count_unit = 'cases' if k == 'm4' else 'pcs'
    _meta(f'{k}_run', f'{label} Running', '', 0, 1, dec=0)
    _meta(f'{k}_fault', f'{label} Fault', '', 0, 1, 0, 1, 0)
    _meta(f'{k}_code', f'{label} Stop Code', '', 0, 99, dec=0)
    _meta(f'{k}_speed', f'{label} Speed', 'cpm' if k == 'm4' else 'bpm', 0, 400, dec=0)
    _meta(f'{k}_cycle', f'{label} Cycle Time', 's' if k == 'm4' else 'ms', 0, 6 if k == 'm4' else 400, dec=2 if k == 'm4' else 0)
    _meta(f'{k}_good', f'{label} Good Count', count_unit, 0, 200_000, dec=0)
    _meta(f'{k}_rej', f'{label} Reject Count', count_unit, 0, 10_000, dec=0)
    _meta(f'{k}_oee', f'{label} OEE', '%', 0, 100, dec=1, lw=65, lc=50)
    _meta(f'{k}_avail', f'{label} Availability', '%', 0, 100, dec=1)
    _meta(f'{k}_perf', f'{label} Performance', '%', 0, 100, dec=1)
    _meta(f'{k}_qual', f'{label} Quality', '%', 0, 100, dec=1)
    _meta(f'{k}_down', f'{label} Downtime This Shift', 'min', 0, 480, dec=1)
_meta('m1_bowl', 'M1 Filler Bowl Level', '%', 0, 100, dec=1, lw=40, lc=25)
_meta('m1_temp', 'M1 Filler Product Temperature', '°C', 0, 40, 28, 32, 1)
_meta('m2_torque', 'M2 Capper Application Torque', 'Ncm', 0, 300, 230, 250, 0)
_meta('m3_stock', 'M3 Labeller Label Reel Remaining', '%', 0, 100, dec=0, lw=15, lc=5)
_meta('g120_run', 'G120 Conveyor Running', '', 0, 1, dec=0)
_meta('g120_fault', 'G120 Conveyor Fault', '', 0, 1, 0, 1, 0)
_meta('g120_hz', 'G120 Conveyor Output Frequency', 'Hz', 0, 50, dec=1)
_meta('g120_rpm', 'G120 Conveyor Motor Speed', 'rpm', 0, 1500, dec=0)
_meta('g120_amp', 'G120 Conveyor Motor Current', 'A', 0, 10, 7.5, 8.5, 2)
_meta('g120_torque', 'G120 Conveyor Torque', '%', 0, 150, 110, 130, 0)
_meta('g120_kw', 'G120 Conveyor Power', 'kW', 0, 4, dec=2)
_meta('line_oee', 'Line 2 OEE', '%', 0, 100, dec=1, lw=65, lc=50)
_meta('line_output', 'Line 2 Output This Shift', 'bottles', 0, 200_000, dec=0)
_meta('line_plan', 'Line 2 Shift Plan', 'bottles', 0, 200_000, dec=0)
_meta('line_rate', 'Line 2 Output Rate', 'bpm', 0, 400, dec=0)
_meta('dt_jam', 'Line 2 Downtime Jams', 'min', 0, 480, dec=1)
_meta('dt_material', 'Line 2 Downtime Material Out', 'min', 0, 480, dec=1)
_meta('dt_flow', 'Line 2 Downtime Starved or Blocked', 'min', 0, 480, dec=1)
_meta('dt_other', 'Line 2 Downtime Other', 'min', 0, 480, dec=1)

COMMON = ['run', 'fault', 'code', 'speed', 'cycle', 'good', 'rej', 'oee', 'avail', 'perf', 'qual', 'down']
TYPES = {'run': 'bool', 'fault': 'bool', 'code': 'int', 'good': 'dint', 'rej': 'dint'}  # everything else REAL


# ── Device layouts, one per PLC ──

def _s7_layout(keys, start=0):
    """DB byte layout from `start`: each BOOL in its own byte (DBXn.0), then INT/DINT/REAL in order, word-aligned.
    Returns [(key, address, type)]. Voltrus v0.52 names an S7 value read at startup after its area and byte offset
    ('db_12'), so seed.py names the tags that way and both start paths file values under the same metric; byte offsets
    stay unique per device (DB20 starts at 100)."""
    out, off = [], start
    for k in keys:
        if k.endswith(('_run', '_fault')):
            out.append((k, f'DBX{off}.0', 'bool'))
            off += 1
    off += off % 2
    for k in keys:
        if k.endswith(('_run', '_fault')):
            continue
        t = 'real' if k.startswith(('line', 'dt_')) else TYPES.get(k.split('_', 1)[1], 'real')
        if t == 'int':
            out.append((k, f'DBW{off}', 'int'))
            off += 2
        else:
            out.append((k, f'DBD{off}', t))
            off += 4
    return out


def s7_tag_name(address: str) -> str:
    """'DBD12' -> 'db_12', the name Voltrus gives an S7 value it reads by address."""
    return 'db_' + re.match(r'DB[XBWD](\d+)', address).group(1)


M1_KEYS = [f'm1_{s}' for s in COMMON] + ['m1_bowl', 'm1_temp', 'line_oee', 'line_output', 'line_plan', 'line_rate',
                                         'dt_jam', 'dt_material', 'dt_flow', 'dt_other']
M4_KEYS = [f'm4_{s}' for s in COMMON]
G120_KEYS = ['g120_run', 'g120_fault', 'g120_hz', 'g120_rpm', 'g120_amp', 'g120_torque', 'g120_kw']

# Siemens: {db_number: [(key, 'DB-relative address', type)]}
S7_FILLER = {1: _s7_layout(M1_KEYS)}
S7_PACKER = {1: _s7_layout(M4_KEYS), 20: _s7_layout(G120_KEYS, 100)}

# Omron NJ: CIP symbolic tag name -> (key, CIP type)
ENIP_NAMES = {'run': 'Running', 'fault': 'Fault', 'code': 'StopCode', 'speed': 'Speed_bpm', 'cycle': 'CycleTime_ms',
              'good': 'GoodCount', 'rej': 'RejectCount', 'oee': 'OEE', 'avail': 'Availability', 'perf': 'Performance',
              'qual': 'Quality', 'down': 'Downtime_min', 'torque': 'CapTorque_Ncm'}
ENIP_TAGS = {f'Capper_{ENIP_NAMES[k.split("_", 1)[1]]}': (k, {'bool': 'BOOL', 'int': 'INT', 'dint': 'DINT'}.get(
    TYPES.get(k.split('_', 1)[1], ''), 'REAL')) for k in [f'm2_{s}' for s in COMMON] + ['m2_torque']}

# Mitsubishi iQ-R: Modbus holding register (= D register number) -> (key, type, scale); 32-bit low word first.
MELSEC_BASE = 5000
MELSEC = []
_a = MELSEC_BASE
for k, t, sc in [('m3_run', 'int16', 1), ('m3_fault', 'int16', 1), ('m3_code', 'int16', 1), ('m3_speed', 'int16', 1),
                 ('m3_cycle', 'int16', 1), ('m3_good', 'int32', 1), ('m3_rej', 'int32', 1), ('m3_oee', 'int16', 0.1),
                 ('m3_avail', 'int16', 0.1), ('m3_perf', 'int16', 0.1), ('m3_qual', 'int16', 0.1), ('m3_down', 'int16', 0.1),
                 ('m3_stock', 'int16', 1)]:
    MELSEC.append((_a, k, t, sc))
    _a += 2 if t == 'int32' else 1


def melsec_words(values) -> dict[int, int]:
    words = {}
    for addr, k, t, sc in MELSEC:
        raw = int(round(values[k] / sc))
        if t == 'int32':
            raw &= 0xFFFFFFFF
            words[addr], words[addr + 1] = raw & 0xFFFF, raw >> 16   # low word first, as a MELSEC D pair
        else:
            words[addr] = raw & 0xFFFF
    return words


# ── Process model ──

def shift_start_elapsed() -> float:
    """Seconds since the current shift started (06:00, 14:00, 22:00 WIB)."""
    t = time.gmtime(time.time() + 7 * 3600)
    secs = t.tm_hour * 3600 + t.tm_min * 60 + t.tm_sec
    return (secs - 6 * 3600) % (8 * 3600)


class Line:
    def __init__(self):
        self.v = {k: 0.0 for k in META}
        self.t = 0.0
        self.buf = [b * 0.5 for b in BUFFER]
        self.m = {}
        self.code_min = {'dt_jam': 0.0, 'dt_material': 0.0, 'dt_flow': 0.0, 'dt_other': 0.0}
        self._start_shift(shift_start_elapsed())
        self.v.update(m1_bowl=62.0, m1_temp=21.5, m2_torque=185.0, m3_stock=71.0)

    def _start_shift(self, elapsed):
        """Shift accumulators as if the line had been running for `elapsed` seconds of this shift."""
        self.elapsed = elapsed
        for k, _, _, _, rated, rej, a0, p0, _ in MACHINES:
            run = elapsed * a0
            total = run * rated / 60 * p0 / (BOTTLES_PER_CASE if k == 'm4' else 1)
            self.m[k] = {'run_s': run, 'total': total, 'rej': total * rej, 'down_s': elapsed - run}
        down = sum(m['down_s'] for m in self.m.values()) / 60
        self.code_min = {'dt_jam': down * 0.31, 'dt_material': down * 0.38, 'dt_flow': down * 0.24, 'dt_other': down * 0.07}

    def step(self, dt):
        self.t += dt
        v, t = self.v, self.t
        new_elapsed = shift_start_elapsed()
        if new_elapsed < self.elapsed - 60:
            self._start_shift(0.0)  # shift change: counters back to zero
        self.elapsed = new_elapsed
        n = len(MACHINES)
        flows = [0.0] * n
        for i, (k, _, _, _, rated, rej, _, p0, (period, dur, off, code)) in enumerate(MACHINES):
            faulted = (t - off) % period < dur and t > off
            infeed = self.buf[i - 1] if i else math.inf
            outfeed_full = i < n - 1 and self.buf[i] >= BUFFER[i] * 0.98
            starved = infeed < 1
            if faulted:
                state = code
            elif starved:
                state = 10
            elif outfeed_full:
                state = 20
            else:
                state = 0
            # Downstream machines follow the buffer in front of them, so the line settles instead of start-stopping.
            fill = 1.0 if i == 0 else self.buf[i - 1] / BUFFER[i - 1]
            speed = 0.0 if state else rated * p0 * (1.0 if i == 0 else min(1.0, 0.35 + 1.3 * fill)) * (1 + random.gauss(0, 0.004))
            bottles = speed / 60 * dt
            if i:
                bottles = min(bottles, self.buf[i - 1])
                self.buf[i - 1] -= bottles
            if i < n - 1:
                self.buf[i] += bottles
            flows[i] = bottles
            m = self.m[k]
            per = BOTTLES_PER_CASE if k == 'm4' else 1
            if state == 0:
                m['run_s'] += dt
            else:
                m['down_s'] += dt
                bucket = 'dt_jam' if state == 31 else 'dt_material' if state == 32 else 'dt_flow' if state in (10, 20) else 'dt_other'
                self.code_min[bucket] += dt / 60
            m['total'] += bottles / per
            m['rej'] += bottles / per * rej
            planned = max(1.0, self.elapsed)
            a = min(1.0, m['run_s'] / planned)
            p = min(1.0, m['total'] * per * 60 / rated / max(1.0, m['run_s']))
            q = 1 - m['rej'] / max(1.0, m['total'])
            v[f'{k}_run'] = float(state == 0)
            v[f'{k}_fault'] = float(state in (31, 32, 41))
            v[f'{k}_code'] = float(state)
            v[f'{k}_speed'] = speed / per
            v[f'{k}_cycle'] = (60 / (speed / per) if k == 'm4' else 60000 / speed) if speed > 1 else 0.0
            v[f'{k}_good'] = float(int(m['total'] - m['rej']))
            v[f'{k}_rej'] = float(int(m['rej']))
            v[f'{k}_avail'], v[f'{k}_perf'], v[f'{k}_qual'] = a * 100, p * 100, q * 100
            v[f'{k}_oee'] = a * p * q * 100
            v[f'{k}_down'] = m['down_s'] / 60
        # Machine details
        v['m1_bowl'] = max(20.0, min(90.0, v['m1_bowl'] + (62 - v['m1_bowl']) * 0.05 * dt + random.gauss(0, 0.3)
                                     - (4 if v['m1_run'] and (t % 97) < 1 else 0)))
        v['m1_temp'] = 21.5 + 0.8 * math.sin(t / 400) + random.gauss(0, 0.03)
        # Capper torque drifts high as the chuck clutch wears, then drops back after the material stop (chuck changed).
        wear = ((t - 120) % 480) / 480
        v['m2_torque'] = (180 + 55 * wear + random.gauss(0, 3)) if v['m2_run'] else 0.0
        if v['m3_run']:
            v['m3_stock'] -= 0.012 * dt
        if v['m3_code'] == 32:
            v['m3_stock'] = 100.0  # reel change
        v['m3_stock'] = max(0.0, v['m3_stock'])
        # Sinamics G120 on the accumulation conveyor into the case packer
        conv = v['m3_run'] or v['m4_run']
        hz_target = (32 + 14 * self.buf[2] / BUFFER[2]) if conv else 0.0
        v['g120_hz'] += (hz_target - v['g120_hz']) * min(1.0, 0.8 * dt)
        v['g120_run'] = float(v['g120_hz'] > 1)
        v['g120_rpm'] = v['g120_hz'] * 28.8
        load = 0.55 + 0.35 * self.buf[2] / BUFFER[2]
        v['g120_torque'] = load * 100 * v['g120_run'] + random.gauss(0, 0.5) * v['g120_run']
        v['g120_kw'] = 2.2 * load * v['g120_hz'] / 50
        v['g120_amp'] = (2.1 + 4.3 * load) * v['g120_run'] + random.gauss(0, 0.02) * v['g120_run']
        # Line master (in the filler PLC)
        out = v['m4_good'] * BOTTLES_PER_CASE
        v['line_output'] = out
        v['line_plan'] = SHIFT_PLAN
        v['line_rate'] = flows[-1] / dt * 60
        ideal = max(1.0, self.elapsed) * 300 / 60
        v['line_oee'] = min(100.0, out / ideal * 100)
        v.update(self.code_min)


# ── S7comm server (what Voltrus's S7 client asks: COTP connect, setup communication, read var) ──

def s7_area_bytes(layout: dict, values) -> dict[int, bytearray]:
    dbs = {}
    for db, items in layout.items():
        data = bytearray(256)
        for key, addr, typ in items:
            if typ == 'bool':
                byte, bit = map(int, addr[3:].split('.'))
                if values[key] > 0.5:
                    data[byte] |= 1 << bit
            else:
                off = int(addr[3:])
                if typ == 'int':
                    data[off:off + 2] = struct.pack('>h', int(round(values[key])))
                elif typ == 'dint':
                    data[off:off + 4] = struct.pack('>i', int(values[key]))
                else:
                    data[off:off + 4] = struct.pack('>f', values[key])
        dbs[db] = data
    return dbs


async def s7_serve(line: Line, layout: dict, pdu_size: int, reader, writer):
    async def tpkt():
        head = await reader.readexactly(4)
        return await reader.readexactly(struct.unpack('>H', head[2:4])[0] - 4)

    def send(s7: bytes):
        writer.write(struct.pack('>BBH', 3, 0, 7 + len(s7)) + b'\x02\xf0\x80' + s7)

    def ack(ref, params, data=b'', err=0):
        return struct.pack('>BBHHHHH', 0x32, 3, 0, ref, len(params), len(data), err) + params + data

    try:
        cr = await tpkt()
        if cr[1] != 0xE0:
            return
        writer.write(bytes([3, 0, 0, 11, 6, 0xD0, 0, 1, 0, 1, 0]))
        while True:
            pkt = await tpkt()
            s7 = pkt[pkt[0] + 1:]
            if len(s7) < 12 or s7[0] != 0x32 or s7[1] != 1:
                continue
            ref = struct.unpack('>H', s7[4:6])[0]
            fn = s7[10]
            if fn == 0xF0:
                send(ack(ref, bytes([0xF0, 0, 0, 1, 0, 1]) + struct.pack('>H', pdu_size)))
            elif fn == 0x04:
                dbs = s7_area_bytes(layout, line.v)
                count, pos, data = s7[11], 12, bytearray()
                for i in range(count):
                    ts, num, db, area = s7[pos + 3], struct.unpack('>H', s7[pos + 4:pos + 6])[0], \
                        struct.unpack('>H', s7[pos + 6:pos + 8])[0], s7[pos + 8]
                    bits = int.from_bytes(s7[pos + 9:pos + 12], 'big')
                    pos += 12
                    size = {0x01: 1, 0x02: 1, 0x04: 2, 0x05: 2, 0x06: 4, 0x07: 4, 0x08: 4}.get(ts, 1) * num
                    area_bytes = dbs.get(db) if area == 0x84 else None
                    start = bits // 8
                    if area_bytes is None or start + size > len(area_bytes):
                        data += b'\x0a\x00\x00\x00'
                        continue
                    chunk = bytes(area_bytes[start:start + size])
                    if ts == 0x01:
                        chunk = bytes([(chunk[0] >> (bits % 8)) & 1])
                        data += b'\xff\x03' + struct.pack('>H', 1) + chunk
                    elif ts == 0x08:
                        data += b'\xff\x07' + struct.pack('>H', len(chunk)) + chunk
                    else:
                        data += b'\xff\x04' + struct.pack('>H', len(chunk) * 8) + chunk
                    if len(chunk) % 2 and i + 1 < count:
                        data += b'\x00'
                send(ack(ref, bytes([0x04, count]), bytes(data)))
            else:
                send(ack(ref, bytes([fn, 0]), err=0x8104))  # function not supported: the demo PLCs are read-only
            await writer.drain()
    except (asyncio.IncompleteReadError, ConnectionError, IndexError, struct.error):
        pass
    finally:
        writer.close()


# ── EtherNet/IP server (RegisterSession, SendRRData carrying CIP Read Tag 0x4C by symbolic name) ──

CIP_TYPES = {'BOOL': (0xC1, '<B'), 'INT': (0xC3, '<h'), 'DINT': (0xC4, '<i'), 'REAL': (0xCA, '<f')}


async def enip_serve(line: Line, reader, writer):
    session = random.randint(1, 0x7FFFFFFF)
    try:
        while True:
            head = await reader.readexactly(24)
            cmd, length = struct.unpack('<HH', head[:4])
            ctx = head[12:20]
            body = await reader.readexactly(length)

            def reply(b: bytes, c=cmd):
                writer.write(struct.pack('<HHII', c, len(b), session, 0) + ctx + b'\x00\x00\x00\x00' + b)

            if cmd == 0x65:
                reply(struct.pack('<HH', 1, 0))
            elif cmd == 0x66:
                break
            elif cmd == 0x6F:
                items, pos, cip = struct.unpack('<H', body[6:8])[0], 8, b''
                for _ in range(items):
                    typ, ln = struct.unpack('<HH', body[pos:pos + 4])
                    if typ == 0xB2:
                        cip = body[pos + 4:pos + 4 + ln]
                    pos += 4 + ln
                svc = cip[0] if cip else 0
                resp = bytes([svc | 0x80, 0, 0x08, 0])  # service not supported
                if svc == 0x4C and cip[2] == 0x91:
                    name = cip[4:4 + cip[3]].decode(errors='replace')
                    tag = ENIP_TAGS.get(name)
                    if tag:
                        key, typ = tag
                        code, fmt = CIP_TYPES[typ]
                        val = line.v[key]
                        raw = struct.pack(fmt, (1 if val > 0.5 else 0) if typ == 'BOOL' else int(val) if typ in ('INT', 'DINT') else val)
                        resp = bytes([0xCC, 0, 0, 0]) + struct.pack('<H', code) + raw
                    else:
                        resp = bytes([0xCC, 0, 0x05, 0])  # path destination unknown
                payload = struct.pack('<IHH', 0, 0, 2) + struct.pack('<HH', 0, 0) + struct.pack('<HH', 0xB2, len(resp)) + resp
                reply(payload)
            else:
                reply(b'')
            await writer.drain()
    except (asyncio.IncompleteReadError, ConnectionError, IndexError, struct.error):
        pass
    finally:
        writer.close()


async def serve_plcs(line: Line, host: str, s7_filler: int, s7_packer: int, enip: int):
    servers = [
        await asyncio.start_server(lambda r, w: s7_serve(line, S7_FILLER, 480, r, w), host, s7_filler),
        await asyncio.start_server(lambda r, w: s7_serve(line, S7_PACKER, 240, r, w), host, s7_packer),
        await asyncio.start_server(lambda r, w: enip_serve(line, r, w), host, enip),
    ]
    print(f'Line 2 PLCs: S7-1500 filler on {s7_filler}, S7-1200 case packer on {s7_packer}, Omron NJ (EtherNet/IP) on {enip}, '
          f'Mitsubishi iQ-R on the Modbus port at D{MELSEC_BASE}', flush=True)
    await asyncio.gather(*(s.serve_forever() for s in servers))


if __name__ == '__main__':
    ln = Line()
    lo = {k: math.inf for k in META}
    hi = {k: -math.inf for k in META}
    while ln.t < 1800:
        ln.step(0.5)
        for k, x in ln.v.items():
            lo[k], hi[k] = min(lo[k], x), max(hi[k], x)
    for k in META:
        name, unit, mn, mx, warn, crit, dec, lw, lc = META[k]
        flags = []
        if crit and hi[k] >= crit: flags.append('CRIT-HI')
        elif warn and hi[k] >= warn: flags.append('warn-hi')
        if lc is not None and lo[k] <= lc: flags.append('CRIT-LO')
        elif lw is not None and lo[k] <= lw: flags.append('warn-lo')
        print(f'{k:14} {lo[k]:12.2f} {hi[k]:12.2f} {ln.v[k]:12.2f} {unit:7} {" ".join(flags)}')
    print(S7_FILLER, S7_PACKER, list(ENIP_TAGS), MELSEC, sep='\n')
