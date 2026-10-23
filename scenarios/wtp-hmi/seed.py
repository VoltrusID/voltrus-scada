#!/usr/bin/env python3
"""Seed the WTP showcase: tag names/units/alarm limits, write policies, and SCADA screens.

    python3 seed.py --url http://localhost:3100 --password <admin password>
    python3 seed.py --log server.log      # read the generated first-run password from the server log

On a fresh database the admin must change password first; seed.py does that and prints the new one.
Re-running is safe: screens are matched by name and their SCADA layouts replaced.
"""
import argparse
import secrets
import http.cookiejar
import json
import re
import urllib.error
import urllib.request


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
}
WRITABLE = ['p101a_run', 'p101b_run', 'xv101_open', 'coag_pump_run', 'flash_mixer_run', 'sludge_pump_run', 'cl2_pump_run',
            'p301a_run', 'p301b_run', 'p301c_run', 'backwash_pump_run', 'air_blower_run', 'clarifier_rake_run']


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

    # Intake
    xv = s.sym('valve', 36, 300, 90, 106, 'xv101_open', label='XV-101', showStatus=True, writable=True)
    pa = s.sym('pump', 176, 170, 90, 124, 'p101a_run', label='P-101A', showStatus=True, writable=True)
    pb = s.sym('pump', 176, 400, 90, 124, 'p101b_run', label='P-101B', showStatus=True, writable=True, faultSensorId='p101b_fault')
    at = s.sym('instrument', 316, 140, 84, 128, 'raw_turbidity', label='Raw turbidity', tagFunction='AT', loopNumber='102', showValue=True)
    ft = s.sym('flow_meter', 310, 320, 96, 104, 'intake_flow', label='FT-101', showValue=True)
    t101 = s.sym('tank', 448, 250, 110, 220, 'raw_tank_level', label='T-101 Raw')
    s.pipe(xv, pa); s.pipe(xv, pb); s.pipe(pa, ft); s.pipe(pb, ft); s.pipe(ft, t101)
    s.pipe(at, ft, 'data')

    # Coagulation & clarification
    t201 = s.sym('tank', 606, 380, 84, 172, 'coag_tank_level', label='T-201 Coag')
    p201 = s.sym('dosing_pump', 606, 214, 84, 106, 'coag_pump_run', label='P-201', showStatus=True, writable=True)
    t202 = s.sym('mixer', 740, 150, 110, 220, 'flash_mix_level', label='T-202 Flash mix', runSensorId='flash_mixer_run')
    cl = s.sym('clarifier', 858, 400, 236, 144, 'clarifier_turbidity', label='CL-203 Clarifier', showValue=True, runSensorId='clarifier_rake_run')
    p204 = s.sym('pump', 900, 540, 90, 106, 'sludge_pump_run', label='P-204 Sludge', writable=True)
    s.pipe(t101, t202); s.pipe(t201, p201); s.pipe(p201, t202); s.pipe(t202, cl); s.pipe(cl, p204)

    # Filtration
    filters = []
    for i in range(3):
        filters.append(s.sym('gravity_filter', 1140, 130 + i * 170, 140, 160, f'filter{i + 1}_headloss',
                             label=f'F-30{i + 1} headloss', showValue=True, runSensorId=f'filter{i + 1}_backwash'))
    fcv = s.sym('control_valve', 1360, 300, 90, 128, 'fcv201_pos', label='FCV-320', showValue=True)
    for f in filters:
        s.pipe(cl, f); s.pipe(f, fcv)

    # Disinfection & distribution
    p401 = s.sym('dosing_pump', 1520, 150, 84, 106, 'cl2_pump_run', label='P-401 NaOCl', showStatus=True, writable=True)
    t401 = s.sym('tank', 1528, 330, 110, 220, 'clearwell_level', label='T-401 Clearwell')
    hl = [s.sym('pump', 1700, 140 + i * 150, 90, 124, f'p301{c}_run', label=f'P-501{c.upper()}', showStatus=True, writable=True)
          for i, c in enumerate('abc')]
    pt = s.sym('instrument', 1816, 290, 76, 128, 'distribution_pressure', label='To network', tagFunction='PT', loopNumber='501', showValue=True)
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


def nozzle(x, y, w, h, vb, port, rows=('label', 'value')):
    """Stage point of a symbol nozzle, mirroring the client's port math (resolve.ts portPoint) for waypoint routing."""
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

    cl = s.sym('clarifier', 40, 330, 236, 144, 'clarifier_turbidity', label='CL-203 settled water', showValue=True, runSensorId='clarifier_rake_run')
    cl_out = nozzle(40, 330, 236, 144, (220, 100), (220, 22))
    fcv_x, fcv_y = 1460, 300
    fcv = s.sym('control_valve', fcv_x, fcv_y, 90, 128, 'fcv201_pos', label='FCV-320', showValue=True)
    fcv_in = nozzle(fcv_x, fcv_y, 90, 128, (100, 100), (0, 72))
    ft = s.sym('flow_meter', 1600, fcv_in[1] - nozzle(0, 0, 96, 104, (100, 70), (0, 46))[1], 96, 104, 'filtered_flow', label='FT-320', showValue=True)
    t401 = s.sym('tank', 1770, 240, 110, 220, 'clearwell_level', label='T-401 Clearwell')

    inlet_header_y, bw_header_y = 186, 600
    bw_x, bw_y = 330, 450
    bw = s.sym('pump', bw_x, bw_y, 90, 124, 'backwash_pump_run', label='P-310 Backwash', showStatus=True, writable=True)
    bw_out = nozzle(bw_x, bw_y, 90, 124, (100, 100), (97, 29.5), ('label', 'status'))
    bl = s.sym('fan', 180, 450, 90, 124, 'air_blower_run', label='B-311 Air scour', showStatus=True, writable=True)
    bl_out = nozzle(180, 450, 90, 124, (100, 100), (94, 50), ('label', 'status'))
    for i in range(3):
        x, y, w, h = 440 + i * 330, 210, 200, 200
        f = s.sym('gravity_filter', x, y, w, h, f'filter{i + 1}_headloss', label=f'F-30{i + 1} headloss', showValue=True,
                  runSensorId=f'filter{i + 1}_backwash')
        f_in = nozzle(x, y, w, h, (140, 120), (0, 26))
        f_out = nozzle(x, y, w, h, (140, 120), (140, 106))
        f_bot = nozzle(x, y, w, h, (140, 120), (70, 120))
        # Settled water header across the top, one drop per filter.
        s.pipe(cl, f, waypoints=[[cl_out[0] + 30, cl_out[1]], [cl_out[0] + 30, inlet_header_y], [x - 24, inlet_header_y], [x - 24, f_in[1]]])
        # Filtered water collector at the nozzle height of FCV-320.
        s.pipe(f, fcv, waypoints=[[f_out[0] + 18, f_out[1]], [f_out[0] + 18, fcv_in[1]]])
        # Backwash header under the gallery, rising into each underdrain.
        s.pipe(bw, f, waypoints=[[bw_out[0] + 20, bw_out[1]], [bw_out[0] + 20, bw_header_y], [f_bot[0], bw_header_y]])
        # Air scour shares the underdrain; its header runs just below the backwash water header.
        s.pipe(bl, f, waypoints=[[bl_out[0] + 10, bl_out[1]], [bl_out[0] + 10, bw_header_y + 30], [f_bot[0], bw_header_y + 30]])
        at = s.sym('instrument', x + 214, 236, 76, 128, f'filter{i + 1}_turbidity', label=f'AT-30{i + 1}', tagFunction='AT',
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
    t401 = s.sym('tank', 60, 220, 130, 260, 'clearwell_level', label='T-401 Clearwell')
    pt = s.sym('instrument', 900, 200, 84, 128, 'distribution_pressure', label='PT-501', tagFunction='PT', loopNumber='501', showValue=True)
    ft = s.sym('flow_meter', 880, 410, 110, 110, 'output_flow', label='FT-501', showValue=True)
    for i, c in enumerate('abc'):
        p = s.sym('pump', 380, 150 + i * 160, 100, 136, f'p301{c}_run', label=f'P-501{c.upper()}', showStatus=True, writable=True)
        v = s.sym('valve', 600, 168 + i * 160, 90, 106, f'p301{c}_run', label=f'XV-50{i + 1}', showStatus=True)
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--url', default='http://localhost:3100')
    ap.add_argument('--password')
    ap.add_argument('--log', help='server log holding the generated first-run admin password')
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

    sources = api.call('GET', '/api/v1/data-sources')
    ids = {s['key']: s['id'] for s in sources if s['key'] in TAGS}
    missing = set(TAGS) - set(ids)
    if missing:
        raise SystemExit(f'tags not found (is the server polling plant-sim?): {sorted(missing)}')

    for key, (name, unit, lo, hi, warn, crit, dec, lw, lc) in TAGS.items():
        cfg = {k: v for k, v in (('low_warn', lw), ('low_crit', lc)) if v is not None}
        api.call('PUT', f'/api/v1/data-sources/{ids[key]}', {
            'name': name, 'unit': unit, 'min_val': lo, 'max_val': hi, 'warn_val': warn, 'crit_val': crit, 'decimals': dec,
            'base_val': lo, 'source_config': json.dumps(cfg) if cfg else '{}',
        })
    for key in WRITABLE:
        api.call('PATCH', f'/api/v1/data-sources/{ids[key]}/write-policy', {'writable': True, 'write_min': 0, 'write_max': 1})
    print(f'{len(TAGS)} tags configured, {len(WRITABLE)} writable')

    screens = {sc['name']: sc['id'] for sc in api.call('GET', '/api/v1/screens')}
    plan = [('Plant Overview', overview), ('Filtration', filtration), ('High-Lift & Power', power)]
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
                       | {v for it in s.items for k, v in json.loads(it['config']).items() if k in ('runSensorId', 'faultSensorId')})
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
