#!/usr/bin/env python3
"""Water treatment plant process simulator — Modbus TCP, stdlib only.

Unlike modbus-sim (independent drifting registers), every value here comes
from one process model, so an HMI shows a plant that behaves like one: pumps
fill and drain tanks, filters load up and backwash, a duty pump trips and the
standby takes over, flows follow demand.

    python3 plant-sim.py [--port 5030]

Map: holding registers, float32 big-endian, two words per tag, addresses in
TAGS order (0, 2, 4, ...). Run/command tags are writable (FC06 or FC16); a
write puts that tag in manual until it is written again.
"""
import argparse
import asyncio
import math
import random
import struct
import time

TAGS = [
    # Intake
    'intake_flow', 'raw_tank_level', 'p101a_run', 'p101b_run', 'p101b_fault', 'xv101_open',
    'raw_turbidity', 'raw_ph',
    # Coagulation / flocculation / clarification
    'coag_dose', 'coag_pump_run', 'coag_tank_level', 'flash_mixer_run', 'floc_speed',
    'clarifier_turbidity', 'sludge_pump_run',
    # Filtration
    'filter1_headloss', 'filter2_headloss', 'filter3_headloss',
    'filter1_turbidity', 'filter2_turbidity', 'filter3_turbidity',
    'filter1_backwash', 'filter2_backwash', 'filter3_backwash',
    'backwash_pump_run', 'air_blower_run', 'filtered_flow', 'fcv201_pos',
    # Disinfection / clearwell / distribution
    'cl2_dose', 'cl2_pump_run', 'clearwell_level', 'chlorine_residual', 'treated_ph',
    'treated_turbidity', 'p301a_run', 'p301b_run', 'p301c_run', 'p301a_speed',
    'distribution_pressure', 'output_flow',
    # Electrical
    'power_kw', 'energy_kwh', 'bus_voltage', 'power_factor',
    # Added later; kept at the end so earlier addresses stay put
    'flash_mix_level', 'clarifier_rake_run',
]
ADDR = {name: i * 2 for i, name in enumerate(TAGS)}
WRITABLE = {'clarifier_rake_run', 'p101a_run', 'p101b_run', 'xv101_open', 'coag_pump_run', 'flash_mixer_run', 'sludge_pump_run',
            'cl2_pump_run', 'p301a_run', 'p301b_run', 'p301c_run', 'air_blower_run', 'backwash_pump_run', 'fcv201_pos'}

DT = 0.5           # model step, seconds
FAULT_PERIOD = 300  # P-101B trips for FAULT_LEN s every FAULT_PERIOD s
FAULT_LEN = 110


class Plant:
    def __init__(self):
        self.v = {n: 0.0 for n in TAGS}
        self.manual: dict[str, float] = {}
        self.t0 = time.monotonic()
        v = self.v
        v.update(raw_tank_level=58.0, coag_tank_level=71.0, clearwell_level=64.0, xv101_open=1,
                 filter1_headloss=0.9, filter2_headloss=2.25, filter3_headloss=1.5,
                 energy_kwh=184_220.0, fcv201_pos=46.0, p301a_speed=48.5, chlorine_residual=1.15)
        self.backwash_left = [0.0, 0.0, 0.0]

    def cmd(self, name: str, auto: float) -> float:
        return self.manual.get(name, auto)

    def write(self, name: str, value: float):
        if name in WRITABLE:
            self.manual[name] = value

    def step(self):
        v, t = self.v, time.monotonic() - self.t0
        n = lambda s: random.gauss(0, s)

        # Raw water quality follows a slow diurnal-ish swing.
        v['raw_turbidity'] = 14 + 7 * math.sin(t / 260) + n(0.3)
        v['raw_ph'] = 7.35 + 0.15 * math.sin(t / 400) + n(0.01)

        # Intake: P-101A duty, P-101B assists below 45 % and trips periodically.
        phase = (t - 25) % FAULT_PERIOD
        fault = 1.0 if t > 25 and phase < FAULT_LEN else 0.0
        v['p101b_fault'] = fault
        valve = self.cmd('xv101_open', 1.0)
        v['xv101_open'] = valve
        v['p101a_run'] = self.cmd('p101a_run', 1.0)
        lead_needs_help = v['raw_tank_level'] < 45 or (v['p101b_run'] and v['raw_tank_level'] < 62)
        v['p101b_run'] = 0.0 if fault else self.cmd('p101b_run', 1.0 if lead_needs_help else 0.0)
        pumps = v['p101a_run'] + v['p101b_run']
        v['intake_flow'] = max(0.0, (1180 * pumps + n(8)) * valve)

        # Treatment draw holds the raw tank near 60 %.
        treat = max(0.0, 1500 + 45 * (v['raw_tank_level'] - 55))
        v['raw_tank_level'] = clamp(v['raw_tank_level'] + (v['intake_flow'] - treat) / 2400 * DT * 0.25, 3, 99)

        # Chemistry
        v['coag_pump_run'] = self.cmd('coag_pump_run', 1.0 if treat > 50 else 0.0)
        v['coag_dose'] = (16 + 0.65 * v['raw_turbidity'] + n(0.2)) * v['coag_pump_run']
        v['coag_tank_level'] = clamp(v['coag_tank_level'] - 0.004 * v['coag_pump_run'] * DT, 8, 100)
        if v['coag_tank_level'] < 12:
            v['coag_tank_level'] = 92.0  # delivery
        v['flash_mixer_run'] = self.cmd('flash_mixer_run', 1.0)
        v['flash_mix_level'] = 78 + 0.004 * (treat - 1600) + 1.5 * math.sin(t / 40) + n(0.2)
        v['clarifier_rake_run'] = self.cmd('clarifier_rake_run', 1.0)
        v['floc_speed'] = 4.2 + 0.4 * math.sin(t / 90) + n(0.03)
        dosed = v['coag_pump_run'] and v['flash_mixer_run']
        v['clarifier_turbidity'] = max(0.2, v['raw_turbidity'] * (0.11 if dosed else 0.55) + n(0.08))
        v['sludge_pump_run'] = self.cmd('sludge_pump_run', 1.0 if (t % 180) < 50 else 0.0)

        # Filtration: headloss climbs; at 2.45 m a filter backwashes for 75 s.
        washing = False
        for i in range(3):
            hl = f'filter{i + 1}_headloss'
            if self.backwash_left[i] > 0:
                self.backwash_left[i] -= DT
                v[hl] = max(0.35, v[hl] - 0.03 * DT)
                v[f'filter{i + 1}_backwash'] = 1.0
                washing = True
            else:
                v[f'filter{i + 1}_backwash'] = 0.0
                v[hl] += 0.0042 * DT * (1 + 0.25 * i)
                if v[hl] > 2.45 and not washing and all(b <= 0 for b in self.backwash_left):
                    self.backwash_left[i] = 75
            turb = 0.06 + 0.07 * v[hl] + v['clarifier_turbidity'] * 0.02
            v[f'filter{i + 1}_turbidity'] = 0.0 if v[f'filter{i + 1}_backwash'] else max(0.02, turb + n(0.005))
        v['backwash_pump_run'] = self.cmd('backwash_pump_run', 1.0 if washing else 0.0)
        v['air_blower_run'] = self.cmd('air_blower_run', 1.0 if washing and any(15 < b < 75 for b in self.backwash_left) else 0.0)
        online = 3 - sum(1 for b in self.backwash_left if b > 0)
        v['filtered_flow'] = max(0.0, treat * online / 3 * 0.985 + n(6))
        v['fcv201_pos'] = clamp(self.cmd('fcv201_pos', 30 + 0.012 * v['filtered_flow']), 0, 100)

        # Distribution demand drives how many high-lift pumps run.
        # Network demand swings around plant output; the clearwell buffers the difference.
        demand = (1230 + 260 * math.sin(t / 220) + n(10)) * (0.6 if self.v['clearwell_level'] < 25 else 1.0)
        v['p301a_run'] = self.cmd('p301a_run', 1.0)
        v['p301b_run'] = self.cmd('p301b_run', 1.0 if demand > 1150 or v['clearwell_level'] > 80 else 0.0)
        v['p301c_run'] = self.cmd('p301c_run', 0.0)
        hl_pumps = v['p301a_run'] + v['p301b_run'] + v['p301c_run']
        v['output_flow'] = min(demand, 1050 * hl_pumps) + n(5) if hl_pumps else 0.0
        v['clearwell_level'] = clamp(v['clearwell_level'] + (v['filtered_flow'] - v['output_flow']) / 3000 * DT * 0.2, 2, 99.5)
        v['p301a_speed'] = (44 + 4 * math.sin(t / 220) + n(0.1)) * v['p301a_run']
        v['distribution_pressure'] = max(0.0, 1.1 + 1.55 * hl_pumps - 0.00025 * v['output_flow'] + n(0.03)) if hl_pumps else 0.4

        v['cl2_pump_run'] = self.cmd('cl2_pump_run', 1.0)
        v['cl2_dose'] = (1.7 + 0.1 * math.sin(t / 150) + n(0.02)) * v['cl2_pump_run']
        target = 1.15 * v['cl2_dose'] / 1.7
        v['chlorine_residual'] += (target - v['chlorine_residual']) * 0.01 * DT
        v['treated_ph'] = 7.6 + 0.08 * math.sin(t / 300) + n(0.01)
        v['treated_turbidity'] = sum(v[f'filter{i}_turbidity'] for i in (1, 2, 3)) / max(1, online) * 0.8

        # Electrical
        kw = (75 * pumps + 110 * hl_pumps * (0.8 + 0.2 * v['p301a_speed'] / 50) + 22 * v['backwash_pump_run']
              + 30 * v['air_blower_run'] + 7.5 * v['flash_mixer_run'] + 4 * v['sludge_pump_run'] + 38)
        v['power_kw'] = kw + n(2)
        v['energy_kwh'] += v['power_kw'] * DT / 3600 * 60  # accelerated so the counter visibly moves
        v['bus_voltage'] = 400 + 3 * math.sin(t / 50) + n(0.4)
        v['power_factor'] = 0.93 + 0.02 * math.sin(t / 120) + n(0.002)

    def registers(self) -> list[int]:
        words = []
        for name in TAGS:
            words.extend(struct.unpack('>HH', struct.pack('>f', self.v[name])))
        return words


def clamp(x, lo, hi):
    return max(lo, min(hi, x))


def handle_pdu(plant: Plant, pdu: bytes) -> bytes:
    fc = pdu[0]
    regs = plant.registers()
    if fc in (3, 4):
        addr, qty = struct.unpack('>HH', pdu[1:5])
        if qty < 1 or addr + qty > len(regs):
            return bytes([fc | 0x80, 2])
        data = regs[addr:addr + qty]
        return bytes([fc, qty * 2]) + struct.pack(f'>{qty}H', *data)
    if fc == 6:
        addr, word = struct.unpack('>HH', pdu[1:5])
        name = TAGS[addr // 2] if addr // 2 < len(TAGS) else None
        if name is None:
            return bytes([fc | 0x80, 2])
        plant.write(name, float(word))
        return pdu[:5]
    if fc == 16:
        addr, qty, _ = struct.unpack('>HHB', pdu[1:6])
        words = struct.unpack(f'>{qty}H', pdu[6:6 + qty * 2])
        if addr % 2 == 0 and qty >= 2 and addr // 2 < len(TAGS):
            for i in range(0, qty - 1, 2):
                val = struct.unpack('>f', struct.pack('>HH', words[i], words[i + 1]))[0]
                plant.write(TAGS[addr // 2 + i // 2], val)
        else:
            return bytes([fc | 0x80, 2])
        return pdu[:5]
    return bytes([fc | 0x80, 1])


async def serve(plant: Plant, reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
    try:
        while True:
            header = await reader.readexactly(7)
            tid, pid, length, uid = struct.unpack('>HHHB', header)
            pdu = await reader.readexactly(length - 1)
            resp = handle_pdu(plant, pdu)
            writer.write(struct.pack('>HHHB', tid, pid, len(resp) + 1, uid) + resp)
            await writer.drain()
    except (asyncio.IncompleteReadError, ConnectionError):
        pass
    finally:
        writer.close()


async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--port', type=int, default=5030)
    args = ap.parse_args()
    plant = Plant()

    async def ticker():
        while True:
            plant.step()
            await asyncio.sleep(DT)

    server = await asyncio.start_server(lambda r, w: serve(plant, r, w), '127.0.0.1', args.port)
    print(f'WTP plant sim on 127.0.0.1:{args.port} — {len(TAGS)} float32 tags, holding regs 0..{len(TAGS) * 2 - 1}', flush=True)
    async with server:
        await asyncio.gather(server.serve_forever(), ticker())


if __name__ == '__main__':
    asyncio.run(main())
