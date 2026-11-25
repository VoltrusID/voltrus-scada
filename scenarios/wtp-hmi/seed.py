#!/usr/bin/env python3
"""Seed the WTP showcase: tag names/units/alarm limits, write policies, equipment instances and SCADA screens.

The process equipment is engineered with equipment types (docs/equipment-types.md): the server's starter types (Pump,
Valve, Control valve, Tank, Analog instrument, Dosing pump, Filter) are installed, every pump, valve, tank, filter and
transmitter is one instance line in INSTANCES, and the screens place symbols bound to those instances.

Also creates the four vertical demo sites from verticals.py (solar with its Victron ESS, genset, energy meters,
booster pumps) and the packaging line from line.py (S7, EtherNet/IP and Modbus PLCs): their devices on plant-sim.py,
their tags and one screen each.

    python3 seed.py --url http://localhost:3100 --password <admin password>
    python3 seed.py --log server.log      # read the generated first-run password from the server log

On a fresh database the admin must change password first; seed.py does that and prints the new one.
Re-running is safe: screens are matched by name and their SCADA layouts replaced.
When it adds the packaging line's S7 or EtherNet/IP PLCs or their tags it says so: restart the server then, which
starts their pollers (Voltrus v0.52 decodes S7 REAL tags right only in pollers started with the server).
"""
import argparse
import secrets
import http.cookiejar
import json
import re
from pathlib import Path
import urllib.error
import urllib.parse
import urllib.request

import line
import verticals


# key: (name, unit, min, max, warn, crit, decimals, low_warn, low_crit)
D = ('', 0, 1, 0, 0, 0, None, None)  # digital run / status tag
TAGS = {
    'intake_flow': ('Raw Water Intake Flow FT-101', 'm³/h', 0, 3000, 0, 0, 0, None, None),
    'raw_tank_level': ('Raw Water Tank T-101 Level', '%', 0, 100, 85, 95, 1, 20, 10),
    'p101a_run': ('Intake Pump P-101A',) + D,
    'p101b_run': ('Intake Pump P-101B',) + D,
    'p101b_fault': ('Intake Pump P-101B Trip', '', 0, 1, 0, 1, 0, None, None),
    'xv101_open': ('Intake Valve XV-101',) + D,
    'raw_turbidity': ('Raw Water Turbidity AT-102', 'NTU', 0, 40, 25, 32, 1, None, None),
    'raw_ph': ('Raw Water pH AT-101', 'pH', 6, 9, 8.5, 8.8, 2, None, None),
    'coag_dose': ('Coagulant Dose', 'mg/L', 0, 60, 45, 55, 1, None, None),
    'coag_pump_run': ('Coagulant Dosing Pump P-201',) + D,
    'coag_tank_level': ('Coagulant Tank T-201 Level', '%', 0, 100, 0, 0, 1, 20, 10),
    'flash_mixer_run': ('Flash Mixer M-202',) + D,
    'floc_speed': ('Flocculator Speed', 'rpm', 0, 10, 0, 0, 1, None, None),
    'clarifier_turbidity': ('Settled Water Turbidity AT-203', 'NTU', 0, 10, 4, 6, 2, None, None),
    'sludge_pump_run': ('Sludge Pump P-204',) + D,
    'filter1_headloss': ('Filter F-301 Headloss', 'm', 0, 3, 2.2, 2.6, 2, None, None),
    'filter2_headloss': ('Filter F-302 Headloss', 'm', 0, 3, 2.2, 2.6, 2, None, None),
    'filter3_headloss': ('Filter F-303 Headloss', 'm', 0, 3, 2.2, 2.6, 2, None, None),
    'filter1_turbidity': ('Filter F-301 Effluent Turbidity', 'NTU', 0, 1, 0.3, 0.5, 2, None, None),
    'filter2_turbidity': ('Filter F-302 Effluent Turbidity', 'NTU', 0, 1, 0.3, 0.5, 2, None, None),
    'filter3_turbidity': ('Filter F-303 Effluent Turbidity', 'NTU', 0, 1, 0.3, 0.5, 2, None, None),
    'filter1_backwash': ('Filter F-301 Backwash',) + D,
    'filter2_backwash': ('Filter F-302 Backwash',) + D,
    'filter3_backwash': ('Filter F-303 Backwash',) + D,
    'backwash_pump_run': ('Backwash Pump P-310',) + D,
    'air_blower_run': ('Air Scour Blower B-311',) + D,
    'filtered_flow': ('Filtered Water Flow FT-320', 'm³/h', 0, 3000, 0, 0, 0, None, None),
    'fcv201_pos': ('Filter Effluent Valve FCV-320', '%', 0, 100, 0, 0, 0, None, None),
    'cl2_dose': ('Chlorine Dose', 'mg/L', 0, 5, 0, 0, 2, None, None),
    'cl2_pump_run': ('Hypochlorite Dosing Pump P-401',) + D,
    'clearwell_level': ('Clearwell T-401 Level', '%', 0, 100, 90, 97, 1, 25, 15),
    'chlorine_residual': ('Free Chlorine Residual AT-402', 'mg/L', 0, 4, 2.5, 3.2, 2, 0.5, 0.2),
    'treated_ph': ('Treated Water pH AT-403', 'pH', 6, 9, 8.3, 8.6, 2, None, None),
    'treated_turbidity': ('Treated Water Turbidity AT-404', 'NTU', 0, 1, 0.3, 0.5, 3, None, None),
    'p301a_run': ('High-Lift Pump P-501A',) + D,
    'p301b_run': ('High-Lift Pump P-501B',) + D,
    'p301c_run': ('High-Lift Pump P-501C',) + D,
    'p301a_speed': ('P-501A VFD Speed', 'Hz', 0, 50, 0, 0, 1, None, None),
    'distribution_pressure': ('Distribution Pressure PT-501', 'bar', 0, 8, 6.0, 6.8, 2, 3.0, 2.0),
    'output_flow': ('Treated Water Output FT-501', 'm³/h', 0, 3000, 0, 0, 0, None, None),
    'power_kw': ('Plant Power Demand', 'kW', 0, 800, 650, 750, 0, None, None),
    'energy_kwh': ('Energy Import', 'kWh', 0, 10_000_000, 0, 0, 0, None, None),
    'bus_voltage': ('MCC Bus Voltage', 'V', 360, 440, 0, 0, 1, None, None),
    'power_factor': ('Power Factor', '', 0.7, 1, 0, 0, 3, None, None),
    'flash_mix_level': ('Flash Mix Tank T-202 Level', '%', 0, 100, 95, 98, 1, None, None),
    'clarifier_rake_run': ('Clarifier Rake Drive',) + D,
    'cl2_dose_sp': ('Chlorine Dose Setpoint', 'mg/L', 0, 5, 0, 0, 2, None, None),
}
WRITABLE = ['p101a_run', 'p101b_run', 'xv101_open', 'coag_pump_run', 'flash_mixer_run', 'sludge_pump_run', 'cl2_pump_run',
            'p301a_run', 'p301b_run', 'p301c_run', 'backwash_pump_run', 'air_blower_run', 'clarifier_rake_run']
# Setpoints: (write_min, write_max). A write outside the range is refused; the plant clamps to 0-5 mg/L itself.
SETPOINTS = {'cl2_dose_sp': (1.0, 3.0)}
TAGS.update(verticals.META)
TAGS.update(line.META)

# Equipment instances on the WTP PLC: (name, type, tag prefix). No base address: the plant's register map is not laid
# out per equipment, so every instance binds the tags its type's key patterns name ({prefix}_run, {prefix}_fault, ...).
INSTANCES = [
    ('XV-101', 'valve', 'xv101'),
    ('P-101A', 'pump', 'p101a'),
    ('P-101B', 'pump', 'p101b'),
    ('FT-101', 'analog', 'intake_flow'),
    ('AT-101', 'analog', 'raw_ph'),
    ('AT-102', 'analog', 'raw_turbidity'),
    ('T-101', 'tank', 'raw_tank'),
    ('T-201', 'tank', 'coag_tank'),
    ('P-201', 'dosing_pump', 'coag'),
    ('T-202', 'tank', 'flash_mix'),
    ('AT-203', 'analog', 'clarifier_turbidity'),
    ('P-204', 'pump', 'sludge_pump'),
    ('F-301', 'filter', 'filter1'),
    ('F-302', 'filter', 'filter2'),
    ('F-303', 'filter', 'filter3'),
    ('P-310', 'pump', 'backwash_pump'),
    ('FCV-320', 'control_valve', 'fcv201'),
    ('FT-320', 'analog', 'filtered_flow'),
    ('P-401', 'dosing_pump', 'cl2'),
    ('T-401', 'tank', 'clearwell'),
    ('AT-402', 'analog', 'chlorine_residual'),
    ('P-501A', 'pump', 'p301a'),
    ('P-501B', 'pump', 'p301b'),
    ('P-501C', 'pump', 'p301c'),
    ('PT-501', 'analog', 'distribution_pressure'),
    ('FT-501', 'analog', 'output_flow'),
]
# name -> the instance as GET /api/v1/equipment-instances returns it (with `resolved`), filled by ensure_equipment().
EQUIP = {}
SYMBOLS = Path(__file__).resolve().parents[2] / 'symbols'


class Api:
    def __init__(self, base):
        self.base = base.rstrip('/')
        self.jar = http.cookiejar.CookieJar()
        self.op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))

    def call(self, method, path, body=None):
        req = urllib.request.Request(self.base + path, method=method, data=None if body is None else json.dumps(body).encode(),
                                     headers={'Content-Type': 'application/json'})
        with self.op.open(req) as r:
            raw = r.read()
        # Servers behind TLS (cookie_secure: true) mark the session cookie Secure; seeding talks plain HTTP on localhost.
        for c in self.jar:
            c.secure = False
        return json.loads(raw) if raw else None


# ── Layout helpers ──

class Screen:
    def __init__(self, ids):
        self.ids = ids
        self.items, self.conns, self.n = [], [], 0

    def _add(self, wtype, x, y, w, h, tag=None, tags=None, name='', **config):
        self.n += 1
        eid = f'e{self.n:03d}'
        bindings = [self.ids[t] for t in (tags or ([tag] if tag else []))]
        for k in ('runSensorId', 'faultSensorId'):
            if k in config:
                config[k] = self.ids[config[k]]
        config['z'] = self.n
        self.items.append({
            'equipment_id': eid, 'equipment_type': wtype, 'equipment_name': name or config.get('label') or config.get('title') or wtype,
            'x': x, 'y': y, 'width': w, 'height': h, 'rotation': 0,
            'sensor_bindings': json.dumps(bindings), 'config': json.dumps(config),
        })
        return eid

    def equip(self, name, x, y, w, h, **config):
        """A symbol bound to an equipment instance, as the Designer places it: the server's binding (symbol, primary
        tag, faceplate and its role map, fault tag, the type's symbol settings) plus this symbol's own settings."""
        rw = EQUIP[name]['resolved']['widget']
        cfg = {k: v for k, v in rw['config'].items() if k != 'label'}
        cfg.update(config)
        cfg.setdefault('label', name)
        cfg['equipment'] = name
        self.n += 1
        eid = f'e{self.n:03d}'
        cfg['z'] = self.n
        self.items.append({
            'equipment_id': eid, 'equipment_type': 'symbol', 'equipment_name': cfg['label'],
            'x': x, 'y': y, 'width': w, 'height': h, 'rotation': 0,
            'sensor_bindings': json.dumps([] if rw['sensor_id'] is None else [rw['sensor_id']]), 'config': json.dumps(cfg),
        })
        return eid

    def sym(self, key, x, y, w, h, tag=None, **config):
        return self._add('symbol', x, y, w, h, tag, symbol=key, **config)

    def widget(self, wtype, x, y, w, h, tag=None, **config):
        return self._add(wtype, x, y, w, h, tag, **config)

    def area(self, x, y, w, h, title, subtitle=''):
        return self._add('area', x, y, w, h, title=title, subtitle=subtitle)

    def label(self, x, y, w, h, text, size=14, **config):
        return self._add('label', x, y, w, h, text=text, fontSize=size, **config)

    def pipe(self, a, b, kind='flow', waypoints=None):
        self.conns.append({'connection_id': f'c{len(self.conns) + 1:03d}', 'from_equipment': a, 'to_equipment': b,
                           'waypoints': json.dumps(waypoints or []), 'connection_type': kind})


def header(s, title, subtitle):
    s.label(24, 14, 900, 40, title, 26, align='left', bold=True)
    s.label(24, 52, 900, 26, subtitle, 14, align='left', muted=True)


def kpis(s, tags, x=1000):
    for i, t in enumerate(tags):
        s.widget('big_number', x + i * 228, 12, 216, 78, t, fontSize=30, showName=True, decimals=TAGS[t][6])


def overview(ids):
    s = Screen(ids)
    header(s, 'WTP-01 · Process Overview', 'Municipal treatment, 50,000 m³/d · Modbus TCP PLC · live')
    kpis(s, ['intake_flow', 'output_flow', 'chlorine_residual', 'power_kw'])

    s.area(16, 104, 556, 552, 'Intake', 'Raw water')
    s.area(584, 104, 520, 552, 'Coagulation & Clarification')
    s.area(1116, 104, 372, 552, 'Filtration', '3 × rapid gravity')
    s.area(1500, 104, 404, 552, 'Disinfection & Distribution')

    # Intake. Process equipment is placed by instance (ISA-101 symbols from the equipment types); the clarifier and
    # the flash mixer's agitator have no type and are plain symbols bound to their tags.
    xv = s.equip('XV-101', 36, 300, 90, 112, actuator='motor')
    pa = s.equip('P-101A', 176, 170, 90, 116)
    pb = s.equip('P-101B', 176, 400, 90, 116)
    at = s.equip('AT-102', 320, 140, 80, 118, tagFunction='AT', loopNumber='102')
    ft = s.equip('FT-101', 320, 320, 80, 118, tagFunction='FT', loopNumber='101')
    t101 = s.equip('T-101', 448, 250, 110, 220, label='T-101 Raw')
    s.pipe(xv, pa); s.pipe(xv, pb); s.pipe(pa, ft); s.pipe(pb, ft); s.pipe(ft, t101)
    s.pipe(at, ft, 'data')

    # Coagulation & clarification
    t201 = s.equip('T-201', 606, 380, 84, 172, label='T-201 Coag')
    p201 = s.equip('P-201', 606, 220, 84, 108)
    t202 = s.equip('T-202', 740, 170, 110, 200, label='T-202 Flash mix')
    s.sym('isa_agitator', 771, 150, 48, 96, 'flash_mixer_run', showLabel=False)
    cl = s.sym('isa_clarifier', 864, 410, 224, 130, 'clarifier_turbidity', label='CL-203 Clarifier', showValue=True,
               runSensorId='clarifier_rake_run')
    p204 = s.equip('P-204', 940, 540, 84, 112, label='P-204 Sludge')
    s.pipe(t101, t202); s.pipe(t201, p201); s.pipe(p201, t202); s.pipe(t202, cl); s.pipe(cl, p204)

    # Filtration
    filters = [s.equip(f'F-30{i + 1}', 1150, 124 + i * 176, 110, 168) for i in range(3)]
    fcv = s.equip('FCV-320', 1360, 300, 90, 120, actuator='diaphragm')
    for f in filters:
        s.pipe(cl, f); s.pipe(f, fcv)

    # Disinfection & distribution
    p401 = s.equip('P-401', 1520, 150, 84, 108, label='P-401 NaOCl')
    t401 = s.equip('T-401', 1528, 330, 110, 220, label='T-401 Clearwell')
    hl = [s.equip(f'P-501{c}', 1700, 140 + i * 150, 90, 116) for i, c in enumerate('ABC')]
    pt = s.equip('PT-501', 1816, 290, 76, 118, label='To network', tagFunction='PT', loopNumber='501')
    s.pipe(fcv, t401); s.pipe(p401, t401)
    for p in hl:
        s.pipe(t401, p); s.pipe(p, pt)

    # Trends + quality
    s.area(16, 668, 1176, 396, 'Process trends', 'live, 1 s samples')
    for i, t in enumerate(['raw_turbidity', 'treated_turbidity', 'distribution_pressure', 'raw_tank_level', 'clearwell_level', 'power_kw']):
        s.widget('trend', 32 + (i % 3) * 386, 708 + (i // 3) * 176, 374, 164, t, lineColor='var(--cyan)')
    s.area(1204, 668, 700, 396, 'Water quality')
    for i, t in enumerate(['treated_ph', 'chlorine_residual', 'treated_turbidity']):
        s.widget('semicircle_gauge', 1222 + i * 226, 706, 214, 140, t)
    s.widget('annunciator', 1222, 862, 666, 186, None, maxItems=12)
    s.items[-1]['sensor_bindings'] = json.dumps([ids[t] for t in [
        'raw_tank_level', 'p101b_fault', 'raw_turbidity', 'clarifier_turbidity', 'filter1_headloss', 'filter2_headloss',
        'filter3_headloss', 'treated_turbidity', 'chlorine_residual', 'clearwell_level', 'distribution_pressure', 'power_kw']])
    return s


FOOTER = {'pad': 2, 'gap': 2, 'label': 15, 'value': 19, 'status': 12}


def nozzle(x, y, w, h, key, side, rows=('label', 'value')):
    """Stage point of a symbol's port, mirroring the client's port math (resolve.ts portPoint) for waypoint routing.
    `key`, `side`: a data symbol's port, viewBox and ports read from its file (symbols/<key>.json, shared with the
    browser and the console). The shaded code symbols pass their viewBox and port point instead."""
    if isinstance(key, str):
        sym = json.loads((SYMBOLS / f'{key}.json').read_text())
        vb, port = sym['viewBox'], sym['ports'][side]
    else:
        vb, port = key, side
    foot = FOOTER['pad'] + sum(FOOTER[r] for r in rows) + FOOTER['gap'] * (len(rows) - 1) if rows else 0
    avail = h - foot
    sc = min(w / vb[0], avail / vb[1])
    ox, oy = (w - vb[0] * sc) / 2, (avail - vb[1] * sc) / 2
    return round(x + ox + port[0] * sc), round(y + oy + port[1] * sc)


def filtration(ids):
    s = Screen(ids)
    header(s, 'WTP-01 · Filtration Gallery', 'Rapid gravity filters F-301…F-303 · backwash on headloss ≥ 2.45 m')
    kpis(s, ['filtered_flow', 'treated_turbidity', 'fcv201_pos', 'clarifier_turbidity'])
    s.area(16, 104, 1888, 560, 'Filters', 'settled water → filter beds → clearwell')

    cl_box = (40, 330, 236, 140)
    cl = s.sym('isa_clarifier', *cl_box, 'clarifier_turbidity', label='CL-203 settled water', showValue=True, runSensorId='clarifier_rake_run')
    cl_out = nozzle(*cl_box, 'isa_clarifier', 'right')
    fcv_box = (1460, 300, 90, 120)
    fcv = s.equip('FCV-320', *fcv_box, actuator='diaphragm')
    fcv_in = nozzle(*fcv_box, 'isa_valve_control', 'left')
    ft_box = (1600, fcv_in[1] - nozzle(0, 0, 72, 110, 'isa_instrument', 'left')[1], 72, 110)
    ft = s.equip('FT-320', *ft_box, tagFunction='FT', loopNumber='320')
    t401 = s.equip('T-401', 1770, 240, 110, 220, label='T-401 Clearwell')

    inlet_header_y, bw_header_y = 186, 600
    bw_box = (330, 450, 90, 116)
    bw = s.equip('P-310', *bw_box, label='P-310 Backwash')
    bw_out = nozzle(*bw_box, 'isa_pump_centrifugal', 'right', ('label', 'status'))
    bl_box = (180, 450, 90, 116)
    bl = s.sym('isa_blower', *bl_box, 'air_blower_run', label='B-311 Air scour', showStatus=True, writable=True)
    bl_out = nozzle(*bl_box, 'isa_blower', 'right', ('label', 'status'))
    for i in range(3):
        box = (470 + i * 330, 200, 150, 220)
        f = s.equip(f'F-30{i + 1}', *box, label=f'F-30{i + 1} headloss')
        f_in = nozzle(*box, 'isa_filter', 'left')
        f_out = nozzle(*box, 'isa_filter', 'right')
        f_bot = nozzle(*box, 'isa_filter', 'bottom')
        # Settled water header across the top, one drop per filter.
        s.pipe(cl, f, waypoints=[[cl_out[0] + 30, cl_out[1]], [cl_out[0] + 30, inlet_header_y], [f_in[0] - 24, inlet_header_y], [f_in[0] - 24, f_in[1]]])
        # Filtered water collector at the nozzle height of FCV-320.
        s.pipe(f, fcv, waypoints=[[f_out[0] + 18, f_out[1]], [f_out[0] + 18, fcv_in[1]]])
        # Backwash header under the gallery, rising into each underdrain.
        s.pipe(bw, f, waypoints=[[bw_out[0] + 20, bw_out[1]], [bw_out[0] + 20, bw_header_y], [f_bot[0], bw_header_y]])
        # Air scour shares the underdrain; its header runs just below the backwash water header.
        s.pipe(bl, f, waypoints=[[bl_out[0] + 10, bl_out[1]], [bl_out[0] + 10, bw_header_y + 30], [f_bot[0], bw_header_y + 30]])
        at = s.sym('isa_instrument', box[0] + 178, 236, 76, 118, f'filter{i + 1}_turbidity', label=f'AT-30{i + 1}', tagFunction='AT',
                   loopNumber=f'30{i + 1}', showValue=True)
        s.pipe(at, f, 'data')
    s.pipe(fcv, ft); s.pipe(ft, t401)

    s.area(16, 676, 1888, 388, 'Filter trends')
    for i in range(3):
        s.widget('trend', 32 + i * 624, 716, 608, 160, f'filter{i + 1}_headloss')
        s.widget('trend', 32 + i * 624, 888, 608, 160, f'filter{i + 1}_turbidity')
    return s


def power(ids):
    s = Screen(ids)
    header(s, 'WTP-01 · High-Lift & Power', 'Clearwell → distribution · 400 V MCC · energy')
    kpis(s, ['power_kw', 'bus_voltage', 'power_factor', 'energy_kwh'])
    s.area(16, 104, 1100, 560, 'High-lift pumping', 'duty / assist / standby')
    t401 = s.equip('T-401', 60, 220, 130, 260, label='T-401 Clearwell')
    pt = s.equip('PT-501', 900, 200, 84, 120, tagFunction='PT', loopNumber='501')
    ft = s.equip('FT-501', 900, 410, 84, 120, tagFunction='FT', loopNumber='501')
    for i, c in enumerate('ABC'):
        p = s.equip(f'P-501{c}', 380, 150 + i * 160, 100, 128)
        # Discharge valves follow their pump's run tag; they are not instances of their own.
        v = s.sym('isa_valve_gate', 600, 168 + i * 160, 90, 112, f'p301{c.lower()}_run', label=f'XV-50{i + 1}', showStatus=True)
        s.pipe(t401, p); s.pipe(p, v); s.pipe(v, pt)
    s.pipe(pt, ft)
    s.widget('semicircle_gauge', 860, 530, 200, 126, 'p301a_speed')

    s.area(1128, 104, 776, 560, 'Electrical', '11 kV / 0.4 kV MCC')
    # Equipment fed from below carries its tag beside it, so the cable doesn't cut through the label.
    tx = s.sym('transformer', 1460, 132, 120, 108, 'bus_voltage', onThreshold=300)
    s.label(1300, 170, 160, 30, 'TX-1 · 11/0.4 kV', 12, align='right', muted=True)
    mtr = s.sym('meter', 1660, 140, 110, 116, 'power_kw', label='PM-01', showValue=False)
    bus = s.sym('busbar', 1160, 300, 720, 14, 'bus_voltage', onThreshold=300, showValue=False)
    feeders = [('p101a_run', 'P-101A'), ('p101b_run', 'P-101B'), ('p301a_run', 'P-501A'), ('p301b_run', 'P-501B'), ('p301c_run', 'P-501C'), ('backwash_pump_run', 'P-310')]
    for i, (t, name) in enumerate(feeders):
        x = 1160 + i * 120
        br = s.sym('breaker', x + 10, 350, 100, 100, t)
        s.label(x + 86, 388, 44, 24, f'Q{i + 1}', 11, align='left', muted=True)
        m = s.sym('motor', x + 10, 520, 100, 96, t, label=name)
        s.pipe(bus, br, 'power'); s.pipe(br, m, 'power')
    s.pipe(tx, bus, 'power'); s.pipe(mtr, tx, 'data')

    s.area(16, 676, 1888, 388, 'Trends')
    for i, t in enumerate(['distribution_pressure', 'output_flow', 'power_kw', 'clearwell_level']):
        s.widget('trend', 32 + (i % 2) * 940, 716 + (i // 2) * 172, 924, 160, t)
    return s

# ── Vertical demo screens (tags from verticals.py) ──

def row(s, x, y, w, text, tag, size=15, h=30):
    """Caption, live value and unit on one line, for faceplate-style lists."""
    s.label(x, y, w * 0.46, h, text, 11, align='left', muted=True)
    s.widget('big_number', x + w * 0.40, y, w * 0.44, h, tag, fontSize=size, showName=False, showUnit=False, decimals=TAGS[tag][6])
    if TAGS[tag][1]:
        s.label(x + w * 0.85, y, 44, h, TAGS[tag][1], 10, align='left', muted=True)


def cell(s, x, y, w, text, tag, size=17):
    """A small caption (with the unit) over a live value, for rows of drive readings."""
    s.label(x, y, w, 16, text, 10, muted=True)
    s.widget('big_number', x, y + 14, w, 34, tag, fontSize=size, showName=False, showUnit=False, decimals=TAGS[tag][6])


def gen_feed(s, gen, x, y, brk):
    """Cable from a generator's side terminal down into its breaker, clear of the name and status under the symbol."""
    gx, gy = nozzle(x, y, 100, 132, (100, 100), (88, 50), ('label', 'status'))
    s.pipe(gen, brk, 'power', waypoints=[[gx + 16, gy], [gx + 16, 284], [x + 50, 284]])


def annunciator(s, ids, x, y, w, h, tags, n=12):
    s.widget('annunciator', x, y, w, h, None, maxItems=n)
    s.items[-1]['sensor_bindings'] = json.dumps([ids[t] for t in tags])


def solar(ids):
    s = Screen(ids)
    header(s, 'PLTS Atap Cikarang · 925 kWp + 61 kWh ESS', 'Rooftop PV: 8 string inverters (Sungrow, Huawei, SMA, Growatt, Solis) and a Victron ESS · Modbus TCP · live')
    kpis(s, ['pv_site_kw', 'pv_site_kwh', 'pv_irr', 'pv_pr'])

    s.area(16, 104, 1376, 600, 'PV arrays & inverters', 'string inverters, AC side to ACP-PV')
    for i, (model, rated, _, _) in enumerate(verticals.INVERTERS):
        k, cx = i + 1, 16 + 86 + i * 170
        p = f'pv_inv{k}_'
        s.label(cx - 80, 130, 160, 18, f'PV-0{k} · {rated * verticals.DC_RATIO:.0f} kWp', 11, muted=True)
        arr = s.sym('solar_panel', cx - 50, 150, 100, 86, p + 'run')
        inv = s.sym('inverter', cx - 45, 262, 90, 136, p + 'kw', label=f'INV-0{k}', showValue=True, showStatus=True,
                    onThreshold=0.5, faultSensorId=p + 'fault')
        s.pipe(arr, inv, 'power')
        s.label(cx - 84, 402, 168, 18, model, 10, muted=True)
        for j, (text, t) in enumerate([('Today', 'kwh'), ('DC V', 'vdc'), ('DC A', 'idc'), ('AC V', 'vac'), ('Temp', 'temp')]):
            row(s, cx - 78, 430 + j * 36, 156, text, p + t, size=14)
    s.label(32, 668, 1340, 24, 'O&M notes: INV-04 roof section B soiled, cleaning scheduled · INV-07 trips on grid overvoltage, '
            'reconnects 30 s after the fault clears', 11, align='left', muted=True)

    s.area(1404, 104, 500, 600, 'Grid connection', '0.4 / 20 kV · export to PLN')
    s.label(1430, 140, 300, 20, 'ACP-PV 400 V AC combiner', 11, align='left', muted=True)
    bus = s.sym('busbar', 1430, 164, 240, 14, 'pv_site_kw', onThreshold=1, showValue=False)
    q = s.sym('breaker', 1520, 196, 60, 96, 'pv_site_kw', onThreshold=1)
    s.label(1584, 232, 60, 22, 'Q-PV', 11, align='left', muted=True)
    m = s.sym('meter', 1502, 316, 96, 96, 'pv_site_kw', showValue=False)
    s.label(1602, 352, 90, 22, 'PM-EXP', 11, align='left', muted=True)
    tx = s.sym('transformer', 1514, 448, 72, 108, 'pv_grid_v', onThreshold=300)
    s.label(1590, 486, 120, 24, 'TX-PV 1 MVA', 11, align='left', muted=True)
    s.label(1430, 566, 240, 24, '▼ PLN 20 kV', 13, bold=True)
    s.pipe(bus, q, 'power'); s.pipe(q, m, 'power'); s.pipe(m, tx, 'power')
    s.label(1700, 140, 190, 20, 'Weather station', 11, align='left', muted=True)
    s.sym('instrument', 1706, 166, 84, 128, 'pv_irr', label='Pyranometer', tagFunction='RT', loopNumber='01', showValue=True)
    s.sym('instrument', 1800, 166, 84, 128, 'pv_tmod', label='Module', tagFunction='TT', loopNumber='02', showValue=True)
    for j, (text, t) in enumerate([('Ambient', 'pv_tamb'), ('Grid V', 'pv_grid_v'), ('Grid Hz', 'pv_grid_hz'), ('Export', 'pv_export_mwh')]):
        row(s, 1706, 330 + j * 36, 186, text, t)

    s.area(16, 716, 560, 348, 'Trends', 'live, 1 s samples')
    s.widget('trend', 32, 756, 528, 145, 'pv_irr', lineColor='var(--warn)')
    s.widget('trend', 32, 911, 528, 145, 'pv_site_kw')

    s.area(588, 716, 804, 348, 'ESS · Victron', '3 × Quattro 48/15000 · Lynx Smart BMS · Cerbo GX, Modbus TCP units 100 / 227 / 225')
    s.sym('battery', 604, 768, 160, 96, 'vic_soc', label='Lithium 51.2 V · 61 kWh')
    s.sym('inverter', 636, 896, 96, 128, 'vic_out_p1', label='3 × Quattro', showStatus=True, onThreshold=100,
          faultSensorId='vic_vebus_err')
    for j, (text, t) in enumerate([('SOC', 'vic_soc'), ('Battery power', 'vic_bat_w'), ('Battery voltage', 'vic_bat_v'), ('Battery current', 'vic_bat_i'),
                                   ('BMS temp', 'vic_bms_temp'), ('SoH', 'vic_bms_soh')]):
        row(s, 784, 752 + j * 32, 286, text, t)
    for j, (text, t) in enumerate([('AC-in L1', 'vic_in_p1'), ('AC-in L2', 'vic_in_p2'), ('AC-in L3', 'vic_in_p3'),
                                   ('AC-out L1', 'vic_out_p1'), ('AC-out L2', 'vic_out_p2'), ('AC-out L3', 'vic_out_p3')]):
        row(s, 1090, 752 + j * 32, 290, text, t)
    for j, (text, t) in enumerate([('VE.Bus state', 'vic_vebus_state'), ('Battery state', 'vic_bat_state'), ('Active input', 'vic_active_in')]):
        row(s, 784 + j * 202, 948, 196, text, t, size=14)
    for j, text in enumerate(['VE.Bus: 3 bulk · 4 absorption · 5 float · 9 inverting · Battery: 0 idle · 1 charging · 2 discharging',
                              'Active input: 0 = AC-in 1 (PLN), 240 = disconnected (critical loads on battery)']):
        s.label(784, 986 + j * 22, 600, 20, text, 10, align='left', muted=True)
    s.label(784, 1032, 600, 20, 'ESS charges from PV surplus, shaves the afternoon peak, then floats; critical loads on AC-out.',
            10, align='left', muted=True)

    s.area(1404, 716, 500, 348, 'Alarms')
    annunciator(s, ids, 1420, 754, 468, 298, [f'pv_inv{k}_fault' for k in range(1, 9)] + ['pv_inv5_temp', 'pv_pr', 'pv_grid_v',
                                                'vic_soc', 'vic_vebus_err', 'vic_grid_lost', 'vic_bms_temp'], n=16)
    return s


def genset(ids):
    s = Screen(ids)
    header(s, 'Genset House · RS Gedung A & B', 'ATS/AMF: Deep Sea DSE7320 MKII · Synchronising: 2 × SmartGen HGM9520N · Modbus TCP · live')
    kpis(s, ['gs_m_v', 'gs_load_a_kw', 'gs_load_b_kw', 'gs_bus_hz'])

    # A: ATS / AMF
    s.area(16, 104, 820, 600, 'ATS / AMF panel · Gedung A', 'DSE7320 MKII · 500 kVA standby')
    s.label(40, 136, 200, 20, 'PLN 20 kV', 12, align='left', bold=True)
    tx = s.sym('transformer', 70, 160, 72, 108, 'gs_m_v', onThreshold=300)
    s.label(146, 200, 120, 22, 'TX-A 630 kVA', 11, align='left', muted=True)
    qm = s.sym('breaker', 76, 300, 60, 96, 'gs_qm')
    s.label(140, 336, 80, 22, 'Q-PLN', 11, align='left', muted=True)
    g1 = s.sym('generator', 300, 140, 100, 132, 'gs_g1_run', label='GEN-01', showStatus=True)
    qg = s.sym('breaker', 320, 300, 60, 96, 'gs_qg1')
    s.label(384, 336, 80, 22, 'Q-G1', 11, align='left', muted=True)
    bus = s.sym('busbar', 50, 430, 400, 14, 'gs_load_a_kw', onThreshold=5, showValue=False)
    s.label(50, 452, 400, 20, 'LVMDP Gedung A · essential load', 11, muted=True)
    s.widget('big_number', 130, 480, 240, 70, 'gs_load_a_kw', fontSize=28, showName=True, decimals=0)
    s.pipe(tx, qm, 'power'); s.pipe(qm, bus, 'power'); gen_feed(s, g1, 300, 140, qg); s.pipe(qg, bus, 'power')
    s.sym('tank_horizontal', 60, 576, 200, 118, 'gs_g1_fuel', label='GEN-01 day tank')
    for j, text in enumerate(['AMF: PLN fails → 5 s start delay → crank → 10 s warm-up → transfer.',
                              'PLN back → 30 s return delay → retransfer → 60 s cool-down → stop.']):
        s.label(290, 600 + j * 24, 530, 22, text, 11, align='left', muted=True)
    s.label(480, 136, 340, 20, 'DSE7320 MKII · GEN-01', 12, align='left', bold=True)
    s.label(480, 166, 150, 30, 'Control mode', 11, align='left', muted=True)
    s.widget('status_light', 660, 168, 26, 26, 'gs_g1_auto')
    s.label(692, 166, 100, 30, 'AUTO', 13, align='left', bold=True)
    for j, (text, t) in enumerate([(None, None), ('Engine speed', 'gs_g1_rpm'), ('Frequency', 'gs_g1_hz'),
                                   ('Voltage L-L', 'gs_g1_v'), ('Active power', 'gs_g1_kw'), ('Oil pressure', 'gs_g1_oil'),
                                   ('Coolant', 'gs_g1_ct'), ('Fuel', 'gs_g1_fuel'), ('Battery', 'gs_g1_batt'),
                                   ('Engine hours', 'gs_g1_hours'), ('PLN frequency', 'gs_m_hz')]):
        if t:
            row(s, 480, 166 + j * 36, 330, text, t)

    # B: synchronising panel
    s.area(848, 104, 1056, 600, 'Synchronising panel · Gedung B', '2 × HGM9520N · 2 × 1000 kVA prime · load-dependent start/stop')
    gens = {}
    for idx, k in enumerate((2, 3)):
        x = 900 + idx * 250
        g = s.sym('generator', x, 140, 100, 132, f'gs_g{k}_run', label=f'GEN-0{k}', showStatus=True)
        q = s.sym('breaker', x + 20, 300, 60, 96, f'gs_g{k}_brk')
        s.label(x + 84, 336, 80, 22, f'Q-G{k}', 11, align='left', muted=True)
        gen_feed(s, g, x, 140, q)
        gens[k] = q
    sbus = s.sym('busbar', 880, 430, 420, 14, 'gs_bus_v', onThreshold=300, showValue=False)
    s.label(880, 452, 420, 20, 'Sync bus 400 V · LVMDP Gedung B', 11, muted=True)
    for q in gens.values():
        s.pipe(q, sbus, 'power')
    s.widget('big_number', 970, 480, 260, 70, 'gs_load_b_kw', fontSize=28, showName=True, decimals=0)
    s.label(870, 566, 440, 20, 'Synchroscope (GEN-03 to bus)', 11, align='left', muted=True)
    s.widget('semicircle_gauge', 870, 590, 170, 104, 'gs_sync_phase')
    row(s, 1050, 600, 250, 'Slip', 'gs_sync_slip')
    row(s, 1050, 636, 250, 'ΔV', 'gs_sync_dv')
    for idx, k in enumerate((2, 3)):
        x = 1330 + idx * 284
        s.label(x, 136, 270, 20, f'HGM9520N · GEN-0{k}', 12, align='left', bold=True)
        for j, (text, t) in enumerate([('Active power', 'kw'), ('Load share', 'share'), ('Frequency', 'hz'), ('Voltage L-L', 'v'),
                                       ('Engine speed', 'rpm'), ('Oil pressure', 'oil'), ('Coolant', 'ct'), ('Fuel', 'fuel'),
                                       ('Battery', 'batt'), ('Engine hours', 'hours')]):
            row(s, x, 166 + j * 36, 266, text, f'gs_g{k}_{t}')
    for j, text in enumerate(['Load-dependent start: GEN-03 starts above 640 kW, synchronises',
                              '(slip < 0.1 Hz, angle < 8°), closes Q-G3 and ramps to a 50/50 share;',
                              'below 520 kW it unloads, opens Q-G3 and cools down.']):
        s.label(1330, 560 + j * 24, 560, 22, text, 11, align='left', muted=True)

    s.area(16, 716, 1176, 348, 'Trends', 'live, 1 s samples')
    s.widget('multi_trend', 32, 756, 568, 300, None, maxTags=3, pointsCount=240)
    s.items[-1]['sensor_bindings'] = json.dumps([ids[t] for t in ['gs_m_v', 'gs_g1_v']])
    s.widget('multi_trend', 612, 756, 568, 300, None, maxTags=3, pointsCount=240)
    s.items[-1]['sensor_bindings'] = json.dumps([ids[t] for t in ['gs_load_b_kw', 'gs_g2_kw', 'gs_g3_kw']])
    s.area(1204, 716, 700, 348, 'Alarms')
    annunciator(s, ids, 1220, 754, 668, 298, ['gs_m_v', 'gs_g1_fuel', 'gs_g1_ct', 'gs_g1_batt', 'gs_g1_kw', 'gs_g2_kw',
                                               'gs_g2_ct', 'gs_g3_ct', 'gs_g2_fuel', 'gs_g3_fuel', 'gs_load_b_kw', 'gs_g3_batt'])
    return s


def meters(ids):
    s = Screen(ids)
    header(s, 'MDP-01 · Energy Monitoring', 'Schneider PM5560 incomer · 5 × PM5100 feeders · 2 × Raritan PX3 rack PDUs · Modbus TCP via Link150 · live')
    kpis(s, ['em_in_kw', 'em_in_pf', 'em_in_dmd', 'em_pue'])

    s.area(16, 104, 440, 600, 'Incomer · PM5560', 'TX-01 1600 kVA · ACB 2500 A')
    tx = s.sym('transformer', 60, 140, 72, 108, 'em_in_v1', onThreshold=150)
    s.label(136, 180, 120, 22, 'TX-01 20/0.4 kV', 11, align='left', muted=True)
    acb = s.sym('breaker', 66, 270, 60, 96, 'em_in_kw', onThreshold=1)
    s.label(130, 306, 80, 22, 'ACB-1', 11, align='left', muted=True)
    pm = s.sym('meter', 48, 390, 96, 116, 'em_in_kw', label='PM5560', showValue=False)
    s.pipe(tx, acb, 'power'); s.pipe(acb, pm, 'power')
    for j, (text, t) in enumerate([('V L1-N', 'em_in_v1'), ('V L2-N', 'em_in_v2'), ('V L3-N', 'em_in_v3'), ('I L1', 'em_in_i1'),
                                   ('I L2', 'em_in_i2'), ('I L3', 'em_in_i3'), ('kvar', 'em_in_kvar'), ('Frequency', 'em_in_hz'),
                                   ('THD-V', 'em_in_thdv'), ('THD-I', 'em_in_thdi'), ('Import', 'em_in_kwh'), ('PF', 'em_in_pf')]):
        row(s, 230, 140 + j * 36, 210, text, t)
    s.label(32, 600, 420, 22, 'PLN contract 690 kVA · kVArh charged when PF < 0.85', 11, align='left', muted=True)

    s.area(468, 104, 1000, 600, 'Feeders · PM5100', '400 V main busbar')
    bus = s.sym('busbar', 500, 150, 940, 14, 'em_in_kw', onThreshold=1, showValue=False)
    for i, (k, _, label) in enumerate(verticals.FEEDERS):
        x = 490 + i * 194
        q = s.sym('breaker', x + 66, 180, 60, 90, f'em_{k}_kw', onThreshold=1)
        m = s.sym('meter', x + 48, 290, 96, 116, f'em_{k}_kw', label=f'PM5100 · {k.upper()}', showValue=False)
        s.pipe(bus, q, 'power'); s.pipe(q, m, 'power')
        s.label(x, 410, 190, 20, label, 12, bold=True)
        for j, (text, t) in enumerate([('kW', 'kw'), ('kWh', 'kwh'), ('PF', 'pf'), ('I avg', 'i'), ('THD-I', 'thdi')]):
            row(s, x + 6, 436 + j * 32, 178, text, f'em_{k}_{t}', size=14)

    s.area(1480, 104, 424, 600, 'Server room · Raritan PX3', 'rack PDUs, 32 A single-phase inlets')
    for r in (1, 2):
        x = 1496 + (r - 1) * 204
        s.label(x, 136, 196, 20, f'Rack A0{r}', 12, bold=True)
        s.widget('semicircle_gauge', x, 160, 196, 124, f'em_pdu{r}_i')
        for j, (text, t) in enumerate([('kW', 'kw'), ('kWh', 'kwh'), ('Inlet V', 'v'), ('Inlet air', 'temp'), ('RH', 'rh')]):
            row(s, x + 4, 296 + j * 32, 188, text, f'em_pdu{r}_{t}', size=14)
    s.widget('big_number', 1496, 470, 196, 86, 'em_it_kw', fontSize=30, showName=True, decimals=1)
    s.widget('big_number', 1700, 470, 196, 86, 'em_pue', fontSize=30, showName=True, decimals=2)
    s.label(1496, 566, 400, 22, 'PUE = MDP-01 incomer / UPS output (IT load)', 11, align='left', muted=True)

    s.area(16, 716, 1176, 348, 'Trends', 'live, 1 s samples')
    s.widget('multi_trend', 32, 756, 568, 300, None, maxTags=5, pointsCount=240)
    s.items[-1]['sensor_bindings'] = json.dumps([ids[f'em_{k}_kw'] for k, _, _ in verticals.FEEDERS])
    s.widget('trend', 612, 756, 568, 145, 'em_in_dmd')
    s.widget('trend', 612, 911, 568, 145, 'em_f3_thdi', lineColor='var(--warn)')
    s.area(1204, 716, 700, 348, 'Alarms')
    annunciator(s, ids, 1220, 754, 668, 298, ['em_in_pf', 'em_in_dmd', 'em_in_thdv', 'em_in_thdi', 'em_f3_thdi', 'em_pdu1_i', 'em_pdu2_i',
                                               'em_pdu1_temp', 'em_pdu2_temp', 'em_in_v1', 'em_in_v2', 'em_in_v3'])
    return s


def pumps(ids):
    s = Screen(ids)
    header(s, 'BPS-01 · Booster Pump Station', 'Constant header pressure 4.5 bar · Danfoss VLT FC 302 (P-01, P-02) · ABB ACS580 (P-03, P-04) · Modbus TCP · live')
    kpis(s, ['vsd_pd', 'vsd_flow', 'vsd_kw', 'vsd_sec'])

    s.area(16, 104, 1376, 600, 'Pumping', 'duty / assist / standby, speed shared')
    tank = s.sym('tank', 36, 200, 120, 240, 'vsd_lvl', label='GR-01 Reservoir')
    tank_out = nozzle(36, 200, 120, 240, (100, 170), (88, 130))
    pt1 = s.sym('instrument', 36, 470, 84, 128, 'vsd_ps', label='Suction', tagFunction='PT', loopNumber='01', showValue=True)
    ft = s.sym('flow_meter', 1040, 128, 110, 104, 'vsd_flow', label='FT-02', showValue=True)
    ft_in = nozzle(1040, 128, 110, 104, (100, 70), (0, 46))
    pt2 = s.sym('instrument', 1190, 116, 84, 128, 'vsd_pd', label='Header', tagFunction='PT', loopNumber='02', showValue=True)
    s.label(1280, 160, 100, 40, '▶ to network', 12, align='left', bold=True)
    for i, (model, kw) in enumerate(verticals.PUMPS):
        k, y = i + 1, 252 + i * 112
        p = f'vsd_p{k}_'
        pump = s.sym('pump', 200, y, 90, 108, p + 'run', label=f'P-0{k}', showStatus=True, faultSensorId=p + 'fault')
        p_in = nozzle(200, y, 90, 108, (100, 100), (2, 52.5), ('label', 'status'))
        p_out = nozzle(200, y, 90, 108, (100, 100), (97, 29.5), ('label', 'status'))
        # Suction header down the reservoir side, discharge riser up to the header line, both clear of the drive readouts.
        s.pipe(tank, pump, waypoints=[[176, tank_out[1]], [176, p_in[1]]])
        s.pipe(pump, ft, waypoints=[[316, p_out[1]], [316, ft_in[1]]])
        s.label(336, y + 8, 140, 20, f'P-0{k} · {model}', 11, align='left', bold=True)
        s.label(336, y + 28, 140, 20, f'{kw} kW · 400 V', 11, align='left', muted=True)
        for j, (text, t) in enumerate([('Output Hz', 'hz'), ('Speed rpm', 'rpm'), ('Current A', 'amp'), ('Power kW', 'kw'),
                                       ('Heatsink °C', 'temp'), ('DC-link V', 'dc'), ('Fault code', 'code')]):
            cell(s, 480 + j * 128, y + 6, 118, text, p + t)
    s.pipe(pt1, tank, 'data')
    s.pipe(ft, pt2)

    s.area(1404, 104, 500, 600, 'Station')
    s.label(1424, 136, 220, 20, 'Header pressure PT-02', 11, muted=True)
    s.label(1664, 136, 220, 20, 'Reservoir GR-01 level', 11, muted=True)
    s.widget('semicircle_gauge', 1424, 158, 220, 140, 'vsd_pd')
    s.widget('semicircle_gauge', 1664, 158, 220, 140, 'vsd_lvl')
    for j, (text, t) in enumerate([('Pressure setpoint', 'vsd_sp'), ('Suction pressure', 'vsd_ps'), ('Header flow', 'vsd_flow'),
                                   ('Station power', 'vsd_kw'), ('Specific energy', 'vsd_sec')]):
        row(s, 1430, 316 + j * 38, 450, text, t, size=17, h=32)
    for j, text in enumerate(['Duty P-01 · assist P-02, P-03 by demand · standby P-04.',
                              'When P-02 trips (FC 302 alarm 13, over current) the next',
                              'pump starts and the header dips below 3.9 bar until it is',
                              'up to speed.']):
        s.label(1430, 530 + j * 24, 460, 22, text, 11, align='left', muted=True)

    s.area(16, 716, 1176, 348, 'Trends', 'live, 1 s samples')
    s.widget('trend', 32, 756, 568, 145, 'vsd_pd')
    s.widget('trend', 32, 911, 568, 145, 'vsd_flow')
    s.widget('multi_trend', 612, 756, 568, 300, None, maxTags=4, pointsCount=240)
    s.items[-1]['sensor_bindings'] = json.dumps([ids[f'vsd_p{k}_hz'] for k in range(1, 5)])
    s.area(1204, 716, 700, 348, 'Alarms')
    annunciator(s, ids, 1220, 754, 668, 298, [f'vsd_p{k}_fault' for k in range(1, 5)] + ['vsd_pd', 'vsd_lvl']
                + [f'vsd_p{k}_amp' for k in range(1, 5)] + ['vsd_p1_temp', 'vsd_p3_temp'])
    return s


def packaging(ids):
    s = Screen(ids)
    header(s, 'Line 2 · PET Bottle Packaging 600 ml', 'Four machines, four PLCs: Siemens S7-1500 and S7-1200 over S7, Omron NJ over EtherNet/IP, '
           'Mitsubishi iQ-R over Modbus TCP · live')
    kpis(s, ['line_oee', 'line_output', 'line_rate', 'line_plan'])

    s.area(16, 104, 1888, 200, 'Line flow', 'each machine read from its own PLC, in its own protocol')
    syms = []
    for i, (k, label, plc, proto, *_rest) in enumerate(line.MACHINES):
        cx = 16 + 236 + i * 472
        s.label(cx - 220, 128, 440, 22, label, 15, bold=True)
        s.label(cx - 220, 150, 440, 20, plc, 12)
        s.label(cx - 220, 170, 440, 18, proto, 11, muted=True)
        syms.append(s.sym('motor', cx - 60, 192, 120, 104, f'{k}_run', label=k.upper(), showStatus=True, faultSensorId=f'{k}_fault'))
    for i, (tag, text) in enumerate([('m2_run', 'CV-01 accumulation'), ('m3_run', 'CV-02 accumulation'), ('g120_run', 'CV-03 · Sinamics G120')]):
        cx = 16 + 236 + i * 472
        s.sym('conveyor', cx + 90, 214, 292, 76, tag, label=text, **({'faultSensorId': 'g120_fault'} if tag == 'g120_run' else {}))

    extra = {'m1': [('Bowl level', 'm1_bowl'), ('Product temp', 'm1_temp')], 'm2': [('Cap torque', 'm2_torque')],
             'm3': [('Label reel', 'm3_stock')], 'm4': [('G120 speed', 'g120_rpm'), ('G120 current', 'g120_amp'), ('G120 torque', 'g120_torque')]}
    for i, (k, label, *_rest) in enumerate(line.MACHINES):
        x = 16 + i * 472
        s.area(x, 316, 460, 388, f'{label} · this shift')
        s.widget('semicircle_gauge', x + 12, 352, 200, 130, f'{k}_oee')
        s.label(x + 12, 482, 200, 18, 'OEE', 11, muted=True)
        for j, (text, t) in enumerate([('Availability', 'avail'), ('Performance', 'perf'), ('Quality', 'qual'), ('Downtime', 'down')]):
            row(s, x + 222, 352 + j * 34, 230, text, f'{k}_{t}')
        for j, (text, t) in enumerate([('Speed', 'speed'), ('Cycle time', 'cycle'), ('Good', 'good'), ('Reject', 'rej'), ('Stop code', 'code')]):
            row(s, x + 12, 516 + j * 34, 214, text, f'{k}_{t}')
        for j, (text, t) in enumerate(extra[k]):
            row(s, x + 236, 516 + j * 34, 214, text, t)

    s.area(16, 716, 560, 348, 'Downtime this shift', 'machine-minutes by reason, from the S7-1500 line master')
    for j, (text, t) in enumerate([('Jams', 'dt_jam'), ('Material out', 'dt_material'), ('Starved / blocked', 'dt_flow'), ('Other', 'dt_other')]):
        row(s, 36, 756 + j * 44, 520, text, t, size=22, h=38)
    for j, text in enumerate(['Stop codes: 0 running · 10 starved · 20 blocked · 31 jam · 32 material out · 41 operator stop',
                              'OEE = availability × performance × quality, per shift (06:00 / 14:00 / 22:00 WIB)']):
        s.label(36, 950 + j * 24, 520, 22, text, 10, align='left', muted=True)

    s.area(588, 716, 604, 348, 'Trends', 'live, 1 s samples')
    s.widget('trend', 604, 756, 572, 145, 'line_rate')
    s.widget('multi_trend', 604, 911, 572, 145, None, maxTags=4, pointsCount=240)
    s.items[-1]['sensor_bindings'] = json.dumps([ids[f'{k}_speed'] for k in ('m1', 'm2', 'm3')])
    s.area(1204, 716, 700, 348, 'Alarms')
    annunciator(s, ids, 1220, 754, 668, 298, [f'{k}_fault' for k, *_ in line.MACHINES] + ['g120_fault', 'm2_torque', 'm3_stock',
                'm1_bowl', 'g120_amp', 'line_oee'] + [f'{k}_oee' for k, *_ in line.MACHINES], n=16)
    return s


VERTICAL_SCREENS = [('Solar PV', solar), ('Genset', genset), ('Energy Meters', meters), ('Pump VSD', pumps),
                    ('Packaging Line', packaging)]


def ensure_devices(api, plc_port, plc_hosts):
    """Create the vertical sites' and the packaging line's devices (or refresh their registers) and their tags."""
    have = {d['id'] for d in api.call('GET', '/api/v1/modbus-devices')}
    for dev_id, d in verticals.DEVICES.items():
        regs = [{'name': k, 'offset': d['base'] + 2 * i, 'register_type': 'float32'} for i, k in enumerate(d['tags'])]
        if dev_id in have:
            api.call('PUT', f'/api/v1/modbus-devices/{dev_id}/registers', {'registers': regs})
        else:
            api.call('POST', '/api/v1/modbus-devices', {'id': dev_id, 'name': d['name'], 'ip': '127.0.0.1', 'port': plc_port,
                                                        'unit_id': d['unit_id'], 'interval_ms': 1000, 'endianness': 'big',
                                                        'registers': regs})
            print(f'created Modbus device {dev_id} ({len(regs)} registers)')
    # Victron GX services and the Mitsubishi labeller: Modbus devices with typed registers.
    typed = {dev_id: (d['name'], d['unit_id'], 'big', [{'name': k, 'offset': addr, 'register_type': typ, 'scale': sc}
                                                       for addr, k, typ, sc in d['regs']])
             for dev_id, d in verticals.GX_DEVICES.items()}
    typed[MELSEC_DEVICE] = ('Line 2 M3 Labeller Mitsubishi iQ-R', 1, 'little',
                            [{'name': k, 'offset': addr, 'register_type': typ, 'scale': sc} for addr, k, typ, sc in line.MELSEC])
    for dev_id, (name, unit_id, order, regs) in typed.items():
        if dev_id in have:
            api.call('PUT', f'/api/v1/modbus-devices/{dev_id}/registers', {'registers': regs})
        else:
            api.call('POST', '/api/v1/modbus-devices', {'id': dev_id, 'name': name, 'ip': '127.0.0.1', 'port': plc_port,
                                                        'unit_id': unit_id, 'interval_ms': 1000, 'endianness': order,
                                                        'registers': regs})
            print(f'created Modbus device {dev_id} ({len(regs)} registers)')
    # Data sources are only made from config.yaml on a fresh database; for devices added later we create them.
    keys = {s['key'] for s in api.call('GET', '/api/v1/data-sources')}
    made = 0
    modbus_tags = [(dev_id, d['tags']) for dev_id, d in verticals.DEVICES.items()]
    modbus_tags += [(dev_id, [r['name'] for r in regs]) for dev_id, (_, _, _, regs) in typed.items()]
    for dev_id, tags in modbus_tags:
        for key in tags:
            if key not in keys:
                api.call('POST', '/api/v1/data-sources', {'key': key, 'name': TAGS[key][0], 'device_id': dev_id,
                                                          'source_type': 'modbus', 'chart_type': 'sparkline'})
                made += 1
    if made:
        print(f'created {made} vertical tags')
    ensure_line_plcs(api, plc_hosts)


MELSEC_DEVICE = 'l2_m3_labeller_iqr'
S7_PLCS = [  # device name, port option, rack/slot, layout
    ('L2-M1-Filler-S7-1500', 's7_filler', 0, 1, line.S7_FILLER),
    ('L2-M4-CasePacker-S7-1200', 's7_packer', 0, 1, line.S7_PACKER),
]
ENIP_PLC = 'L2-M2-Capper-Omron-NJ501'


def ensure_line_plcs(api, hosts):
    """The packaging line's S7 and EtherNet/IP PLCs: create the devices and tags that are missing (enabled, so the
    server starts their pollers when it starts), then link their values to data sources (link_line_tags)."""
    have = {d['name']: d for d in api.call('GET', '/api/v1/s7/devices')}
    new_pollers = False
    for name, port_opt, rack, slot, layout in S7_PLCS:
        dev, changed = have.get(name), False
        if dev is None:
            changed = True
            dev = {'id': api.call('POST', '/api/v1/s7/devices', {'name': name, 'url': f"127.0.0.1:{hosts[port_opt]}", 'rack': rack,
                                                                  'slot': slot, 'interval_ms': 1000, 'enabled': True})['id']}
            print(f'created S7 device {name}')
        tags = {t['name'] for t in api.call('GET', f"/api/v1/s7/devices/{dev['id']}/tags")}
        for db, items in layout.items():
            for key, addr, typ in items:
                if line.s7_tag_name(addr) not in tags:
                    changed = True
                    api.call('POST', f"/api/v1/s7/devices/{dev['id']}/tags",
                             {'tag_address': f'DB{db}.{addr}', 'name': line.s7_tag_name(addr), 'display_name': TAGS[key][0],
                              'data_type': typ})
        new_pollers |= changed
    have = {d['name']: d for d in api.call('GET', '/api/v1/enip/devices')}
    dev, changed = have.get(ENIP_PLC), False
    if dev is None:
        changed = True
        dev = {'id': api.call('POST', '/api/v1/enip/devices', {'name': ENIP_PLC, 'url': f"127.0.0.1:{hosts['enip']}", 'slot': 0,
                                                                'interval_ms': 1000, 'enabled': True})['id']}
        print(f'created EtherNet/IP device {ENIP_PLC}')
    tags = {t['name'] for t in api.call('GET', f"/api/v1/enip/devices/{dev['id']}/tags")}
    for cip_name, (key, typ) in line.ENIP_TAGS.items():
        if key not in tags:
            changed = True
            api.call('POST', f"/api/v1/enip/devices/{dev['id']}/tags",
                     {'tag_name': cip_name, 'name': key, 'display_name': TAGS[key][0], 'data_type': typ})
    link_line_tags(api)
    if new_pollers or changed:
        # Not started through the API: Voltrus v0.52 decodes S7 REAL tags right only in pollers started with the server.
        print('packaging line PLCs added or changed: restart the Voltrus server to start their pollers')


def plc_cache_keys(api):
    """{data source key: our tag key} for the S7 and EtherNet/IP tags, plus the cache device of each.

    The S7 and EtherNet/IP pollers file a value under the cache device 's7_<id>' / 'enip_<id>' with the tag name as
    the metric. A data source keyed '<cache device>.<tag>' on that device reads it on every Voltrus version;
    the data source a tag gets when it is created (keyed by device name, device_id the bare id) is only resolved
    by versions after v0.52.0."""
    out = {}
    s7 = {d['name']: d['id'] for d in api.call('GET', '/api/v1/s7/devices')}
    for name, _, _, _, layout in S7_PLCS:
        for items in layout.values():
            for key, addr, _ in items:
                out[f"s7_{s7[name]}.{line.s7_tag_name(addr)}"] = (key, f"s7_{s7[name]}", line.s7_tag_name(addr))
    enip = {d['name']: d['id'] for d in api.call('GET', '/api/v1/enip/devices')}[ENIP_PLC]
    for key, _ in line.ENIP_TAGS.values():
        out[f'enip_{enip}.{key}'] = (key, f'enip_{enip}', key)
    return out


def link_line_tags(api):
    sources = api.call('GET', '/api/v1/data-sources')
    have = {s['key'] for s in sources}
    made = 0
    for ds_key, (key, device, tag) in plc_cache_keys(api).items():
        if ds_key not in have:
            api.call('POST', '/api/v1/data-sources', {'key': ds_key, 'name': TAGS[key][0], 'device_id': device,
                                                      'source_type': 'modbus', 'chart_type': 'sparkline',
                                                      'source_config': json.dumps({'tag': tag})})
            made += 1
    # The per-tag data sources the S7/EtherNet/IP tag API made would duplicate these in the tag list.
    prefixes = tuple(f's7_{name}:' for name, *_ in S7_PLCS) + (f'{ENIP_PLC}:',)
    dropped = 0
    for s in sources:
        if s.get('source_type') in ('s7', 'enip') and s['key'].startswith(prefixes):
            api.call('DELETE', f"/api/v1/data-sources/{s['id']}")
            dropped += 1
    if made or dropped:
        print(f'line PLC tags: {made} data sources linked to the poller cache, {dropped} duplicates removed')


def ensure_equipment(api):
    """Install the starter equipment types and create (or bring up to date) the WTP instances; fills EQUIP."""
    r = api.call('POST', '/api/v1/equipment-types/library/install', {})
    if r['installed']:
        print(f"equipment types installed: {', '.join(r['installed'])}")
    have = {i['name']: i for i in api.call('GET', '/api/v1/equipment-instances')}
    made = changed = 0
    for name, typ, prefix in INSTANCES:
        body = {'name': name, 'type': typ, 'device': 'wtp', 'prefix': prefix}
        cur = have.get(name)
        if cur is None:
            api.call('POST', '/api/v1/equipment-instances', body)
            made += 1
        elif any(cur.get(k) != v for k, v in body.items()):
            keep = {k: cur[k] for k in ('base', 'description', 'symbol', 'overrides') if k in cur}
            api.call('PUT', f'/api/v1/equipment-instances/{urllib.parse.quote(name)}', {**keep, **body})
            changed += 1
    EQUIP.clear()
    EQUIP.update({i['name']: i for i in api.call('GET', '/api/v1/equipment-instances')})
    missing = sorted(f"{n}.{r['role']}" for n, *_ in INSTANCES for r in EQUIP[n]['resolved']['roles'] if r['status'] == 'missing')
    print(f'{len(INSTANCES)} equipment instances ({made} created, {changed} updated)'
          + (f'; roles without a tag: {", ".join(missing)}' if missing else ''))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--url', default='http://localhost:3100')
    ap.add_argument('--password')
    ap.add_argument('--log', help='server log holding the generated first-run admin password')
    ap.add_argument('--plc-port', type=int, default=5030, help='plant-sim.py port the vertical devices poll (default 5030)')
    ap.add_argument('--s7-filler', type=int, default=5102, help='plant-sim.py S7 port of the S7-1500 filler')
    ap.add_argument('--s7-packer', type=int, default=5103, help='plant-sim.py S7 port of the S7-1200 case packer')
    ap.add_argument('--enip', type=int, default=5144, help='plant-sim.py EtherNet/IP port of the Omron NJ capper')
    ap.add_argument('--new-password', default=None, help='password set on first login (default: a random one, printed once)')
    a = ap.parse_args()
    if not a.new_password:
        a.new_password = secrets.token_urlsafe(12)
    api = Api(a.url)

    pw = a.password
    if not pw and a.log:
        m = re.findall(r'GENERATED ADMIN PASSWORD: (\S+)', open(a.log, errors='ignore').read())
        pw = m[-1] if m else None
    for candidate in [pw, a.new_password]:
        if not candidate:
            continue
        try:
            r = api.call('POST', '/api/v1/auth/login', {'username': 'admin', 'password': candidate})
            pw = candidate
            break
        except urllib.error.HTTPError:
            continue
    else:
        raise SystemExit('login failed: pass --password or --log')
    if r.get('must_change_password'):
        api.call('POST', '/api/v1/auth/change-password', {'old_password': pw, 'new_password': a.new_password})
        print(f'admin password changed to {a.new_password}')

    ensure_devices(api, a.plc_port, {'s7_filler': a.s7_filler, 's7_packer': a.s7_packer, 'enip': a.enip})
    sources = api.call('GET', '/api/v1/data-sources')
    # S7 and EtherNet/IP tags are keyed '<cache device>.<tag>'; map them back to our tag keys.
    plc_keys = {ds_key: key for ds_key, (key, _, _) in plc_cache_keys(api).items()}
    by_key = {plc_keys.get(s['key'], s['key']): s for s in sources}
    ids = {k: s['id'] for k, s in by_key.items() if k in TAGS}
    missing = set(TAGS) - set(ids)
    if missing:
        raise SystemExit(f'tags not found (is the server polling plant-sim?): {sorted(missing)}')

    # Instances first: instantiating applies the types' attributes (units, limits, alarms, write policies) to the tags
    # it binds. The showcase's own tag metadata and write policies below then win, on the first run and every rerun.
    ensure_equipment(api)

    for key, (name, unit, lo, hi, warn, crit, dec, lw, lc) in TAGS.items():
        # Keep what the source config already holds (the S7/EtherNet/IP tag binding), set the low alarm limits.
        try:
            cfg = json.loads(by_key[key].get('source_config') or '{}')
        except ValueError:
            cfg = {}
        cfg = {k: v for k, v in cfg.items() if k not in ('low_warn', 'low_crit')}
        cfg.update({k: v for k, v in (('low_warn', lw), ('low_crit', lc)) if v is not None})
        api.call('PUT', f'/api/v1/data-sources/{ids[key]}', {
            'name': name, 'unit': unit, 'min_val': lo, 'max_val': hi, 'warn_val': warn, 'crit_val': crit, 'decimals': dec,
            'base_val': lo, 'source_config': json.dumps(cfg) if cfg else '{}',
        })
    for key in WRITABLE:
        api.call('PATCH', f'/api/v1/data-sources/{ids[key]}/write-policy', {'writable': True, 'write_min': 0, 'write_max': 1})
    for key, (lo, hi) in SETPOINTS.items():
        api.call('PATCH', f'/api/v1/data-sources/{ids[key]}/write-policy',
                 {'writable': True, 'write_min': lo, 'write_max': hi, 'write_max_rate': 10})
    print(f'{len(TAGS)} tags configured, {len(WRITABLE) + len(SETPOINTS)} writable')

    screens = {sc['name']: sc['id'] for sc in api.call('GET', '/api/v1/screens')}
    plan = [('Plant Overview', overview), ('Filtration', filtration), ('High-Lift & Power', power)] + VERTICAL_SCREENS
    for name, build in plan:
        if name not in screens:
            if name == 'Plant Overview' and 'WTP' in screens:
                api.call('PUT', f"/api/v1/screens/{screens['WTP']}", {'name': name})
                screens[name] = screens.pop('WTP')
            else:
                screens[name] = api.call('POST', '/api/v1/screens', {'name': name})['id']
        sid = screens[name]
        s = build(ids)
        bound = sorted({b for it in s.items for b in json.loads(it['sensor_bindings'])}
                       | {v for it in s.items for k, v in json.loads(it['config']).items()
                          if k in ('runSensorId', 'faultSensorId', 'manualSensorId', 'interlockSensorId') and isinstance(v, int)}
                       # Faceplate roles (commands, speeds, setpoints) stream too, so a faceplate opens with live values.
                       | {v for it in s.items for v in (json.loads(it['config']).get('faceplateMap') or {}).values() if isinstance(v, int)})
        for b in bound:
            try:
                api.call('POST', f'/api/v1/screens/{sid}/data-sources', {'data_source_id': b})
            except urllib.error.HTTPError:
                pass  # already on the screen
        api.call('PUT', f'/api/v1/screens/{sid}/scada-equipment', {'items': s.items})
        api.call('PUT', f'/api/v1/screens/{sid}/scada-connections', {'items': s.conns})
        print(f'screen {name!r} ({sid}): {len(s.items)} items, {len(s.conns)} connections')


if __name__ == '__main__':
    main()
