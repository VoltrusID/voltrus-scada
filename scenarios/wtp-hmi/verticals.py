"""Four vertical demo sites served by plant-sim.py next to the WTP, for SIs who want to see their own kind of plant.

Each site is its own Modbus device on the same simulator port, in its own address block (float32 big-endian,
two words per tag, tags in the order declared below):

    plts_cikarang   1000  Solar O&M: 1 MWp rooftop PV, eight inverters from five brands, plus a Victron ESS
                          (Cerbo GX on unit IDs 100, 227 and 225, see GX_DEVICES)
    genset_house    2000  Genset house: ATS/AMF panel (DSE7320 MKII) and a two-set synchronising panel (SmartGen HGM9520N)
    mdp_01          3000  Energy monitoring: PM5560 incomer, five PM5100 feeders, two Raritan PX3 rack PDUs
    booster_station 4000  Booster pump station: Danfoss VLT FC 302 and ABB ACS580 drives on constant header pressure

The tag list here is the single source of truth: plant-sim.py serves it, seed.py creates the Modbus devices,
names the tags and sets their alarm limits from it. The sites run on accelerated clocks where it helps the demo:
the sun is always up (an O&M screen at night is all zeros), and fuel burns and mains fails every few minutes.
"""
import math
import random
import re
import time

# key -> (name, unit, min, max, warn, crit, decimals, low_warn, low_crit); warn/crit 0 = no high alarm
META: dict[str, tuple] = {}
# device id -> {'name', 'base', 'unit_id', 'tags': [keys]}
DEVICES: dict[str, dict] = {}
_dev = None


def device(dev_id, name, base, unit_id):
    global _dev
    _dev = dev_id
    DEVICES[dev_id] = {'name': name, 'base': base, 'unit_id': unit_id, 'tags': []}


def tag(key, name, unit='', lo=0, hi=100, warn=0, crit=0, dec=1, lw=None, lc=None):
    assert re.fullmatch(r'[A-Za-z0-9_ .-]+', name), f'tag name {name!r} breaks the server naming convention'
    META[key] = (name, unit, lo, hi, warn, crit, dec, lw, lc)
    DEVICES[_dev]['tags'].append(key)


def dig(key, name):
    tag(key, name, '', 0, 1, 0, 0, 0)


def fault(key, name):
    tag(key, name, '', 0, 1, 0, 1, 0)


# ── Solar O&M ──

INVERTERS = [  # brand/model, rated kW AC, MPP voltage, relative yield (soiling, mismatch)
    ('Sungrow SG110CX', 110, 640, 0.985),
    ('Sungrow SG110CX', 110, 645, 0.98),
    ('Huawei SUN2000-100KTL-M1', 100, 610, 0.975),
    ('Huawei SUN2000-100KTL-M1', 100, 600, 0.90),   # soiled roof section: visibly behind INV-03
    ('SMA Sunny Tripower 60', 60, 700, 0.97),
    ('SMA Sunny Tripower 60', 60, 705, 0.975),
    ('Growatt MAX 100KTL3-X LV', 100, 620, 0.97),   # trips on grid overvoltage every 10 min
    ('Solis S5-GC100K', 100, 630, 0.98),
]
DC_RATIO = 1.25
PV_KWP = sum(r * DC_RATIO for _, r, _, _ in INVERTERS)

device('plts_cikarang', 'PLTS Atap 1 MWp (8 inverters)', 1000, 11)
tag('pv_irr', 'Irradiance POA RT-01', 'W/m²', 0, 1200, dec=0)
tag('pv_tmod', 'Module Temperature TT-02', '°C', 0, 90, 75, 85, 1)
tag('pv_tamb', 'Ambient Temperature TT-03', '°C', 0, 50, dec=1)
tag('pv_site_kw', 'Site AC Power Export Meter', 'kW', 0, 900, dec=0)
tag('pv_site_kwh', 'Site Energy Today', 'kWh', 0, 5000, dec=0)
tag('pv_pr', 'Performance Ratio', '%', 0, 100, dec=1, lw=68, lc=58)
tag('pv_grid_v', 'Grid Voltage 400 V', 'V', 360, 440, 424, 432, 1, 380, 370)
tag('pv_grid_hz', 'Grid Frequency', 'Hz', 49, 51, 50.4, 50.6, 2, 49.6, 49.4)
tag('pv_export_mwh', 'Export Energy Total', 'MWh', 0, 10_000, dec=1)
for i, (model, rated, _, _) in enumerate(INVERTERS, 1):
    p = f'pv_inv{i}_'
    tag(p + 'kw', f'INV-0{i} AC Power', 'kW', 0, rated, dec=1)
    tag(p + 'kwh', f'INV-0{i} Energy Today', 'kWh', 0, rated * 6, dec=0)
    tag(p + 'vdc', f'INV-0{i} DC Voltage', 'V', 0, 1100, dec=0)
    tag(p + 'idc', f'INV-0{i} DC Current', 'A', 0, 250, dec=1)
    tag(p + 'vac', f'INV-0{i} AC Voltage L-L', 'V', 0, 480, dec=0)
    tag(p + 'temp', f'INV-0{i} Internal Temperature', '°C', 0, 90, 68, 78, 1)
    dig(p + 'run', f'INV-0{i} {model} Grid-Connected')
    fault(p + 'fault', f'INV-0{i} {model} Fault')

# ── Genset house ──

device('genset_house', 'Genset House (DSE7320 MKII + 2 × HGM9520N)', 2000, 21)
tag('gs_m_v', 'PLN Mains Voltage L-L', 'V', 0, 480, 440, 456, 0, 370, 340)
tag('gs_m_hz', 'PLN Mains Frequency', 'Hz', 0, 55, dec=2)
dig('gs_m_ok', 'PLN Mains Available')
dig('gs_qm', 'ATS Mains Breaker Q-PLN')
dig('gs_qg1', 'ATS Generator Breaker Q-G1')
tag('gs_load_a_kw', 'Building A Load', 'kW', 0, 500, dec=0)
dig('gs_g1_auto', 'GEN-01 DSE7320 in Auto')
dig('gs_g1_run', 'GEN-01 Engine Running')
tag('gs_g1_v', 'GEN-01 Voltage L-L', 'V', 0, 480, dec=0)
tag('gs_g1_hz', 'GEN-01 Frequency', 'Hz', 0, 55, dec=2)
tag('gs_g1_kw', 'GEN-01 Active Power', 'kW', 0, 400, 360, 390, 0)
tag('gs_g1_rpm', 'GEN-01 Engine Speed', 'RPM', 0, 1800, dec=0)
tag('gs_g1_oil', 'GEN-01 Oil Pressure', 'kPa', 0, 700, dec=0)
tag('gs_g1_ct', 'GEN-01 Coolant Temperature', '°C', 0, 120, 95, 104, 0)
tag('gs_g1_fuel', 'GEN-01 Fuel Level', '%', 0, 100, dec=0, lw=30, lc=15)
tag('gs_g1_batt', 'GEN-01 Battery Voltage', 'V', 0, 32, dec=1, lw=21, lc=18)
tag('gs_g1_hours', 'GEN-01 Engine Hours', 'h', 0, 99_999, dec=1)
tag('gs_bus_v', 'Sync Bus Voltage L-L', 'V', 0, 480, dec=0)
tag('gs_bus_hz', 'Sync Bus Frequency', 'Hz', 0, 55, dec=2)
tag('gs_load_b_kw', 'Building B Load', 'kW', 0, 1600, 1350, 1500, 0)
for n in (2, 3):
    p = f'gs_g{n}_'
    dig(p + 'run', f'GEN-0{n} Engine Running')
    dig(p + 'brk', f'GEN-0{n} Breaker Q-G{n}')
    tag(p + 'v', f'GEN-0{n} Voltage L-L', 'V', 0, 480, dec=0)
    tag(p + 'hz', f'GEN-0{n} Frequency', 'Hz', 0, 55, dec=2)
    tag(p + 'kw', f'GEN-0{n} Active Power', 'kW', 0, 800, 720, 780, 0)
    tag(p + 'share', f'GEN-0{n} Load Share', '%', 0, 100, dec=0)
    tag(p + 'rpm', f'GEN-0{n} Engine Speed', 'RPM', 0, 1800, dec=0)
    tag(p + 'oil', f'GEN-0{n} Oil Pressure', 'kPa', 0, 700, dec=0)
    tag(p + 'ct', f'GEN-0{n} Coolant Temperature', '°C', 0, 120, 95, 104, 0)
    tag(p + 'fuel', f'GEN-0{n} Fuel Level', '%', 0, 100, dec=0, lw=30, lc=15)
    tag(p + 'batt', f'GEN-0{n} Battery Voltage', 'V', 0, 32, dec=1, lw=21, lc=18)
    tag(p + 'hours', f'GEN-0{n} Engine Hours', 'h', 0, 99_999, dec=1)
tag('gs_sync_slip', 'Synchroscope Slip Frequency', 'Hz', -0.5, 0.5, dec=2)
tag('gs_sync_phase', 'Synchroscope Phase Angle', '°', -180, 180, dec=0)
tag('gs_sync_dv', 'Synchroscope Voltage Difference', 'V', 0, 40, dec=1)

# ── Energy monitoring / data centre ──

FEEDERS = [  # key, tag name prefix, label on the screen
    ('f1', 'F1 UPS-A', 'UPS-A (IT)'), ('f2', 'F2 UPS-B', 'UPS-B (IT)'), ('f3', 'F3 Chiller CH-01', 'Chiller CH-01'),
    ('f4', 'F4 CRAC-AHU', 'CRAC / AHU'), ('f5', 'F5 Lighting and Small Power', 'Lighting & small power'),
]
device('mdp_01', 'MDP-01 Energy Meters (PM5560, 5 × PM5100, 2 × PX3)', 3000, 31)
for ph in (1, 2, 3):
    tag(f'em_in_v{ph}', f'Incomer Voltage L{ph}-N', 'V', 200, 260, 245, 253, 1, 213, 207)
for ph in (1, 2, 3):
    tag(f'em_in_i{ph}', f'Incomer Current L{ph}', 'A', 0, 1600, dec=0)
tag('em_in_kw', 'Incomer Active Power', 'kW', 0, 1000, dec=1)
tag('em_in_kvar', 'Incomer Reactive Power', 'kvar', 0, 600, dec=1)
tag('em_in_pf', 'Incomer Power Factor', '', 0.7, 1, dec=3, lw=0.88, lc=0.85)
tag('em_in_hz', 'Incomer Frequency', 'Hz', 49, 51, dec=2)
tag('em_in_thdv', 'Incomer THD Voltage', '%', 0, 10, 5, 8, 1)
tag('em_in_thdi', 'Incomer THD Current', '%', 0, 40, 20, 25, 1)
tag('em_in_kwh', 'Incomer Energy Import', 'kWh', 0, 99_999_999, dec=0)
tag('em_in_dmd', 'Incomer Demand 15 min', 'kW', 0, 700, 560, 600, 0)
for k, name, _ in FEEDERS:
    tag(f'em_{k}_kw', f'{name} Active Power', 'kW', 0, 300, dec=1)
    tag(f'em_{k}_kwh', f'{name} Energy', 'kWh', 0, 99_999_999, dec=0)
    tag(f'em_{k}_pf', f'{name} Power Factor', '', 0.6, 1, dec=3)
    tag(f'em_{k}_i', f'{name} Current Avg', 'A', 0, 500, dec=0)
    tag(f'em_{k}_thdi', f'{name} THD Current', '%', 0, 50, 30, 40, 1)
for r in (1, 2):
    p, rack = f'em_pdu{r}_', f'Rack A0{r} PX3'
    tag(p + 'i', f'{rack} Inlet Current', 'A', 0, 32, 25.6, 28.8, 1)
    tag(p + 'kw', f'{rack} Active Power', 'kW', 0, 7.4, dec=2)
    tag(p + 'kwh', f'{rack} Energy', 'kWh', 0, 9_999_999, dec=1)
    tag(p + 'v', f'{rack} Inlet Voltage', 'V', 200, 260, dec=1)
    tag(p + 'temp', f'Rack A0{r} Inlet Air Temperature', '°C', 15, 35, 27, 32, 1)
    tag(p + 'rh', f'Rack A0{r} Humidity', '%RH', 0, 100, 70, 80, 0)
tag('em_it_kw', 'IT Load UPS Output', 'kW', 0, 400, dec=1)
tag('em_pue', 'PUE', '', 1, 3, 1.9, 2.2, 2)

# ── Booster pump station ──

PUMPS = [('Danfoss VLT FC 302', 37), ('Danfoss VLT FC 302', 37), ('ABB ACS580', 45), ('ABB ACS580', 45)]
device('booster_station', 'Booster Pump Station (2 × FC 302, 2 × ACS580)', 4000, 41)
tag('vsd_lvl', 'Ground Reservoir GR-01 Level', '%', 0, 100, 95, 98, 1, 30, 20)
tag('vsd_ps', 'Suction Pressure PT-01', 'bar', 0, 2, dec=2)
tag('vsd_pd', 'Header Pressure PT-02', 'bar', 0, 8, 5.5, 6.0, 2, 3.9, 3.4)
tag('vsd_sp', 'Header Pressure Setpoint', 'bar', 0, 8, dec=2)
tag('vsd_flow', 'Header Flow FT-02', 'm³/h', 0, 600, dec=0)
tag('vsd_kw', 'Station Power', 'kW', 0, 200, dec=1)
tag('vsd_sec', 'Specific Energy', 'kWh/m³', 0, 0.5, dec=3)
for n, (model, kw) in enumerate(PUMPS, 1):
    p = f'vsd_p{n}_'
    dig(p + 'run', f'P-0{n} Running')
    fault(p + 'fault', f'P-0{n} {model} Trip')
    tag(p + 'hz', f'P-0{n} Output Frequency', 'Hz', 0, 50, dec=1)
    tag(p + 'rpm', f'P-0{n} Motor Speed', 'rpm', 0, 1500, dec=0)
    tag(p + 'amp', f'P-0{n} Motor Current', 'A', 0, 100, 78 if kw == 37 else 92, 85 if kw == 37 else 98, 1)
    tag(p + 'kw', f'P-0{n} Motor Power', 'kW', 0, kw, dec=1)
    tag(p + 'temp', f'P-0{n} Heatsink Temperature', '°C', 0, 100, 75, 85, 0)
    tag(p + 'dc', f'P-0{n} DC-Link Voltage', 'V', 0, 800, dec=0)
    tag(p + 'code', f'P-0{n} Active Fault Code', '', 0, 9999, dec=0)


# ── Victron ESS on the solar site, read the way a Cerbo GX serves it ──
# Three Quattro 48/15000 (one per phase) carry the site's critical loads and buffer PV into a 61 kWh lithium bank
# on a Lynx Smart BMS. A GX device answers each service on its own unit ID with int16/uint16 registers at the
# addresses and scale factors of Victron's register list (scale: engineering value = raw x scale).
GX_DEVICES: dict[str, dict] = {}
_gx = None


def gx_device(dev_id, name, unit_id):
    global _gx
    _gx = dev_id
    GX_DEVICES[dev_id] = {'name': name, 'unit_id': unit_id, 'regs': []}


def gx(addr, key, typ, scale, name, unit='', lo=0, hi=100, warn=0, crit=0, dec=1, lw=None, lc=None):
    assert re.fullmatch(r'[A-Za-z0-9_ .-]+', name), name
    META[key] = (name, unit, lo, hi, warn, crit, dec, lw, lc)
    GX_DEVICES[_gx]['regs'].append((addr, key, typ, scale))


ESS_KWH = 61.4
gx_device('victron_gx_system', 'Victron Cerbo GX system (unit 100)', 100)
for ph in (1, 2, 3):
    gx(816 + ph, f'vic_cons_l{ph}', 'uint16', 1, f'ESS Critical Load L{ph}', 'W', 0, 15000, dec=0)
for ph in (1, 2, 3):
    gx(819 + ph, f'vic_grid_l{ph}', 'int16', 1, f'ESS Grid L{ph}', 'W', -15000, 15000, dec=0)
gx(826, 'vic_source', 'int16', 1, 'ESS Active Input Source', '', 0, 240, dec=0)
gx(840, 'vic_bat_v', 'uint16', 0.1, 'ESS Battery Voltage', 'V', 44, 58, dec=1)
gx(841, 'vic_bat_i', 'int16', 0.1, 'ESS Battery Current', 'A', -600, 600, dec=1)
gx(842, 'vic_bat_w', 'int16', 1, 'ESS Battery Power', 'W', -30000, 30000, dec=0)
gx(843, 'vic_soc', 'uint16', 1, 'ESS Battery State of Charge', '%', 0, 100, dec=0, lw=20, lc=10)
gx(844, 'vic_bat_state', 'uint16', 1, 'ESS Battery State', '', 0, 2, dec=0)
gx_device('victron_gx_vebus', 'Victron 3 x Quattro 48-15000 VE.Bus (unit 227)', 227)
for ph in (1, 2, 3):
    gx(2 + ph, f'vic_in_v{ph}', 'uint16', 0.1, f'Quattro AC-in Voltage L{ph}', 'V', 180, 260, dec=1)
for ph in (1, 2, 3):
    gx(11 + ph, f'vic_in_p{ph}', 'int16', 10, f'Quattro AC-in Power L{ph}', 'W', -15000, 15000, dec=0)
for ph in (1, 2, 3):
    gx(14 + ph, f'vic_out_v{ph}', 'uint16', 0.1, f'Quattro AC-out Voltage L{ph}', 'V', 180, 260, dec=1, lw=207, lc=196)
gx(21, 'vic_out_hz', 'int16', 0.01, 'Quattro AC-out Frequency', 'Hz', 45, 55, dec=2)
for ph in (1, 2, 3):
    gx(22 + ph, f'vic_out_p{ph}', 'int16', 10, f'Quattro AC-out Power L{ph}', 'W', 0, 15000, dec=0)
gx(29, 'vic_active_in', 'uint16', 1, 'Quattro Active Input', '', 0, 240, dec=0)
gx(31, 'vic_vebus_state', 'uint16', 1, 'Quattro VE.Bus State', '', 0, 252, dec=0)
gx(32, 'vic_vebus_err', 'uint16', 1, 'Quattro VE.Bus Error', '', 0, 30, 0, 1, 0)
gx(33, 'vic_switch', 'uint16', 1, 'Quattro Switch Position', '', 0, 4, dec=0)
gx(64, 'vic_grid_lost', 'uint16', 1, 'Quattro Grid Lost Alarm', '', 0, 2, 1, 2, 0)
gx_device('victron_gx_bms', 'Victron Lynx Smart BMS (unit 225)', 225)
gx(259, 'vic_bms_v', 'uint16', 0.01, 'Lynx BMS Battery Voltage', 'V', 44, 58, dec=2)
gx(261, 'vic_bms_i', 'int16', 0.1, 'Lynx BMS Battery Current', 'A', -600, 600, dec=1)
gx(262, 'vic_bms_temp', 'int16', 0.1, 'Lynx BMS Battery Temperature', '°C', 0, 60, 40, 45, 1)
gx(266, 'vic_bms_soc', 'uint16', 0.1, 'Lynx BMS State of Charge', '%', 0, 100, dec=1)
gx(304, 'vic_bms_soh', 'uint16', 0.1, 'Lynx BMS State of Health', '%', 0, 100, dec=1)


def gx_words(unit_id: int, values) -> dict[int, int] | None:
    """Register words a GX device answers on `unit_id`, or None when no Victron service has that unit ID."""
    for d in GX_DEVICES.values():
        if d['unit_id'] == unit_id:
            words = {}
            for addr, key, typ, scale in d['regs']:
                words[addr] = int(round(values[key] / scale)) & 0xFFFF
            return words
    return None


def addresses() -> dict[int, str]:
    """Holding-register start address -> tag key, for every vertical tag."""
    out = {}
    for d in DEVICES.values():
        for i, key in enumerate(d['tags']):
            out[d['base'] + 2 * i] = key
    return out


# ── Process models ──

def clamp(x, lo, hi):
    return max(lo, min(hi, x))


def ramp(x, target, rate, dt):
    return x + clamp(target - x, -rate * dt, rate * dt)


def n(s):
    return random.gauss(0, s)


def wib_hours() -> float:
    t = time.gmtime(time.time() + 7 * 3600)
    return t.tm_hour + t.tm_min / 60 + t.tm_sec / 3600


class Verticals:
    def __init__(self):
        self.v = {k: 0.0 for k in META}
        self.t = 0.0
        v = self.v
        v.update(pv_export_mwh=1842.6, gs_g1_hours=1268.4, gs_g2_hours=12483.7, gs_g3_hours=11906.2,
                 gs_g1_fuel=64.0, gs_g2_fuel=78.0, gs_g3_fuel=71.0, gs_g1_ct=42.0, gs_g2_ct=84.0, gs_g3_ct=40.0,
                 em_in_kwh=4_812_330.0, em_pdu1_kwh=38_214.6, em_pdu2_kwh=51_087.3, vsd_lvl=68.0, vsd_pd=4.5, vsd_sp=4.5,
                 pv_tmod=58.0, pv_tamb=31.5, gs_g2_rpm=1500.0, em_pdu1_temp=23.2, em_pdu2_temp=23.6, em_pdu1_rh=52, em_pdu2_rh=53)
        for i in range(len(INVERTERS)):
            v[f'pv_inv{i + 1}_temp'] = 55.0
        for i in range(len(PUMPS)):
            v[f'vsd_p{i + 1}_temp'] = 40.0
            v[f'vsd_p{i + 1}_dc'] = 562.0
        for i, (k, _, _) in enumerate(FEEDERS):
            v[f'em_{k}_kwh'] = [1_402_118, 1_377_904, 1_120_655, 512_380, 241_906][i]
        self.inv_down = [0.0] * len(INVERTERS)      # seconds until a tripped inverter may reconnect
        self.inv_ramp = [1.0] * len(INVERTERS)      # soft-start factor after reconnect
        self.g = {2: {'phase': 'run', 'tmr': 0.0}, 3: {'phase': 'off', 'tmr': 0.0}}
        self.sync_angle = 140.0
        self.dmd: list[float] = []

    def step(self, dt):
        self.t += dt
        self.solar(dt)
        self.genset(dt)
        self.meters(dt)
        self.booster(dt)
        self.ess(dt)

    # Victron ESS: charges from PV surplus, shaves the afternoon peak, then floats. SOC moves 6x faster than real
    # so a visitor sees it change; the Quattros pass the grid through to the critical loads all the time.
    def ess(self, dt):
        v, t = self.v, self.t
        c = t % 900
        soc = v['vic_bms_soc'] or 58.0
        if c < 420:
            p_bat = min(24000.0, 27 * v['pv_irr']) * (1 if soc < 92 else max(0.12, (100 - soc) / 8 * 0.6))
            state = 3 if soc < 92 else 4
        elif c < 600:
            p_bat = -16000.0 if soc > 25 else 0.0
            state = 9
        else:
            p_bat = 350.0 if soc < 99 else 0.0
            state = 5
        p_bat += random.gauss(0, 60)
        soc = max(5.0, min(100.0, soc + p_bat * dt / 3600 / (ESS_KWH * 1000) * 100 * 6))
        r = 0.004
        ocv = 50.4 + 3.4 * soc / 100
        i_bat = p_bat / ocv
        vbat = ocv + i_bat * r
        v['vic_bms_soc'] = soc
        v['vic_soc'] = round(soc)
        v['vic_bat_w'] = p_bat
        v['vic_bat_i'] = v['vic_bms_i'] = i_bat
        v['vic_bat_v'] = v['vic_bms_v'] = vbat
        v['vic_bat_state'] = 1.0 if p_bat > 200 else 2.0 if p_bat < -200 else 0.0
        v['vic_bms_temp'] = 27.5 + 2.5 * abs(i_bat) / 500 + 0.5 * math.sin(t / 600) + n(0.03)
        v['vic_bms_soh'] = 97.4
        loads = [3800 + 600 * math.sin(t / 130 + ph) + n(25) for ph in (0, 2.1, 4.2)]
        for ph in (1, 2, 3):
            vin = v['pv_grid_v'] / math.sqrt(3) + (0.3, -0.4, 0.2)[ph - 1] + n(0.2)
            v[f'vic_in_v{ph}'] = vin
            v[f'vic_out_v{ph}'] = vin - 0.6
            v[f'vic_out_p{ph}'] = loads[ph - 1]
            v[f'vic_in_p{ph}'] = loads[ph - 1] + p_bat / 3 / 0.95 if p_bat > 0 else loads[ph - 1] + p_bat / 3 * 0.95
            v[f'vic_cons_l{ph}'] = loads[ph - 1]
            v[f'vic_grid_l{ph}'] = v[f'vic_in_p{ph}']
        v['vic_out_hz'] = v['pv_grid_hz']
        v['vic_source'] = 1.0     # grid
        v['vic_active_in'] = 0.0  # AC-in 1
        v['vic_vebus_state'] = float(state)
        v['vic_vebus_err'] = 0.0
        v['vic_switch'] = 3.0     # on
        v['vic_grid_lost'] = 0.0

    # Rooftop PV: clear-sky noon with passing clouds, clipping on the high-DC-ratio inverters, one inverter tripping.
    def solar(self, dt):
        v, t = self.v, self.t
        cloud = 0.5 * (1 + math.sin(t / 140 + 2.2 * math.sin(t / 61)))
        dim = clamp((cloud - 0.72) / 0.28, 0, 1) * 0.5
        v['pv_irr'] = max(0.0, (880 + 90 * math.sin(t / 700)) * (1 - dim) + n(4))
        v['pv_tamb'] = 31.5 + 1.5 * math.sin(t / 900) + n(0.05)
        v['pv_tmod'] += (v['pv_tamb'] + 0.03 * v['pv_irr'] - v['pv_tmod']) * 0.02 * dt
        grid_v = 402 + 5 * math.sin(t / 300) + n(0.4)
        grid_hz = 50 + 0.04 * math.sin(t / 37) + n(0.008)
        # INV-07 sees the bus at its terminal: it trips on overvoltage for ~2 min every 10 min.
        trip7 = 200 < (t % 600) < 215
        day = clamp((wib_hours() - 6) / 12, 0, 1)
        day_frac = (1 - math.cos(math.pi * day)) / 2
        derate = 1 - 0.0037 * (v['pv_tmod'] - 25)
        site_kw = site_kwh = 0.0
        for i, (_, rated, vmp, perf) in enumerate(INVERTERS):
            p = f'pv_inv{i + 1}_'
            if i == 6 and trip7 and self.inv_down[i] <= 0:
                self.inv_down[i] = 120
            if self.inv_down[i] > 0:
                self.inv_down[i] -= dt
                v[p + 'fault'] = 1.0 if self.inv_down[i] > 30 else 0.0  # fault clears, then 30 s reconnect countdown
                v[p + 'run'] = 0.0
                self.inv_ramp[i] = 0.0
            else:
                v[p + 'fault'] = 0.0
                v[p + 'run'] = 1.0
                self.inv_ramp[i] = min(1.0, self.inv_ramp[i] + dt / 25)
            pdc = rated * DC_RATIO * v['pv_irr'] / 1000 * derate * perf
            pac = min(rated * 0.995, pdc * 0.982) * self.inv_ramp[i] * v[p + 'run']
            v[p + 'kw'] = max(0.0, pac + n(0.15) * v[p + 'run'])
            vdc = vmp * (1 - 0.0035 * (v['pv_tmod'] - 25))
            v[p + 'vdc'] = (vdc * 1.21 if not v[p + 'run'] else vdc) + n(1.2)
            v[p + 'idc'] = v[p + 'kw'] / 0.982 * 1000 / v[p + 'vdc'] if v[p + 'run'] else 0.0
            v[p + 'vac'] = grid_v + (1.5 + 0.5 * i) * (v[p + 'kw'] / rated) + n(0.3)
            hot = 10 if i == 4 else 0  # SMA in an unventilated room runs hot
            v[p + 'temp'] += (v['pv_tamb'] + 8 + hot + 22 * v[p + 'kw'] / rated - v[p + 'temp']) * 0.01 * dt
            v[p + 'kwh'] = rated * 4.7 * perf * day_frac * (0.93 if i == 6 else 1.0)
            site_kw += v[p + 'kw']
            site_kwh += v[p + 'kwh']
        v['pv_grid_v'] = grid_v
        v['pv_grid_hz'] = grid_hz
        v['pv_site_kw'] = site_kw * 0.985
        v['pv_site_kwh'] = site_kwh * 0.985
        ref = PV_KWP * v['pv_irr'] / 1000
        v['pv_pr'] = clamp(v['pv_site_kw'] / ref * 100, 0, 100) if ref > 5 else 0.0
        v['pv_export_mwh'] += v['pv_site_kw'] * dt / 3600 / 1000

    # Genset house. A: PLN fails for 2.5 min every 12 min and the DSE7320 runs the AMF sequence.
    # B: two 1000 kVA sets on a sync bus; GEN-03 starts, synchronises and shares load when demand passes 640 kW.
    def genset(self, dt):
        v, t = self.v, self.t
        c = t % 720
        mains = not (480 <= c < 630)
        v['gs_m_ok'] = float(mains)
        v['gs_m_v'] = (400 + 4 * math.sin(t / 90) + n(0.6)) if mains else 0.0
        v['gs_m_hz'] = (50 + 0.03 * math.sin(t / 41) + n(0.01)) if mains else 0.0
        v['gs_g1_auto'] = 1.0
        # AMF timeline inside the outage: 5 s start delay, crank, 10 s warm-up, transfer; mains back -> 30 s return delay, 60 s cool-down.
        since_fail = c - 480
        g1_run = 485 <= c < 720
        cranking = 485 <= c < 489
        on_load = 500 <= c < 660
        v['gs_qg1'] = float(on_load)
        v['gs_qm'] = float(mains and not on_load)
        load_a = 245 + 35 * math.sin(t / 170) + n(2)
        v['gs_load_a_kw'] = load_a if (v['gs_qm'] or v['gs_qg1']) else 0.0
        v['gs_g1_run'] = float(g1_run and not cranking)
        rpm_target = 1500 if g1_run and not cranking else (220 if cranking else 0)
        v['gs_g1_rpm'] = max(0.0, ramp(v['gs_g1_rpm'], rpm_target, 600, dt) + (n(3) if g1_run else 0))
        up = v['gs_g1_rpm'] > 1400
        v['gs_g1_hz'] = v['gs_g1_rpm'] / 30 + (n(0.02) if up else 0) - (0.4 if on_load and since_fail < 22 else 0)
        v['gs_g1_v'] = (400 + n(0.8) - (14 if on_load and since_fail < 22 else 0)) if up else v['gs_g1_rpm'] / 1500 * 380
        v['gs_g1_kw'] = load_a if on_load else 0.0
        v['gs_g1_oil'] = (420 - 0.12 * v['gs_g1_ct'] + n(3)) * min(1, v['gs_g1_rpm'] / 1500) if g1_run else 0.0
        ct_target = 78 + 0.04 * v['gs_g1_kw'] if g1_run else 42
        v['gs_g1_ct'] = ramp(v['gs_g1_ct'], ct_target, 0.6 if g1_run else 0.08, dt)
        v['gs_g1_batt'] = 22.4 + n(0.1) if cranking else (27.6 if g1_run else 26.9) + n(0.03)
        if g1_run:
            v['gs_g1_fuel'] -= (0.012 + 0.00008 * v['gs_g1_kw']) * dt  # accelerated so a day tank visibly empties
            v['gs_g1_hours'] += dt / 3600
        if v['gs_g1_fuel'] < 18:
            v['gs_g1_fuel'] = 92.0  # day tank refilled from the bulk tank

        load_b = 700 + 230 * math.sin(t / 260) + 60 * math.sin(t / 47) + n(6)
        v['gs_load_b_kw'] = load_b
        g2, g3 = self.g[2], self.g[3]
        g3['tmr'] += dt
        ph = g3['phase']
        if ph == 'off' and load_b > 640:
            g3.update(phase='start', tmr=0)
        elif ph == 'start' and g3['tmr'] > 12:
            g3.update(phase='sync', tmr=0)
            self.sync_angle = random.uniform(-170, 170)
        elif ph == 'sync':
            slip = max(0.04, 0.22 - 0.012 * g3['tmr'])
            self.sync_angle = (self.sync_angle + 360 * slip * dt + 180) % 360 - 180
            if g3['tmr'] > 8 and abs(self.sync_angle) < 8:
                g3.update(phase='load', tmr=0)
        elif ph == 'load' and g3['tmr'] > 15:
            g3.update(phase='run', tmr=0)
        elif ph == 'run' and load_b < 520 and g3['tmr'] > 60:
            g3.update(phase='unload', tmr=0)
        elif ph == 'unload' and g3['tmr'] > 12:
            g3.update(phase='cool', tmr=0)
        elif ph == 'cool' and g3['tmr'] > 45:
            g3.update(phase='off', tmr=0)
        ph = g3['phase']
        g3_brk = ph in ('load', 'run', 'unload')
        if ph == 'load':
            share3 = 0.5 * g3['tmr'] / 15
        elif ph == 'run':
            share3 = 0.5
        elif ph == 'unload':
            share3 = 0.5 * (1 - g3['tmr'] / 12)
        else:
            share3 = 0.0
        bus_hz = 50 + 0.05 * math.sin(t / 33) + n(0.01)
        v['gs_bus_hz'] = bus_hz
        v['gs_bus_v'] = 400 + 2 * math.sin(t / 71) + n(0.5)
        for k, brk, share in ((2, True, 1 - share3), (3, g3_brk, share3)):
            p = f'gs_g{k}_'
            running = k == 2 or ph != 'off'
            v[p + 'run'] = float(running)
            v[p + 'brk'] = float(brk)
            target_rpm = 1500 if running else 0
            v[p + 'rpm'] = max(0.0, ramp(v[p + 'rpm'], target_rpm, 300, dt) + (n(2) if running else 0))
            v[p + 'kw'] = max(0.0, load_b * share + n(3)) if brk else 0.0
            v[p + 'share'] = v[p + 'kw'] / 800 * 100
            if brk:
                v[p + 'hz'], v[p + 'v'] = bus_hz, v['gs_bus_v'] + n(0.4)
            else:
                v[p + 'hz'] = v[p + 'rpm'] / 30 + (0.0 if ph != 'sync' else max(0.04, 0.22 - 0.012 * g3['tmr']))
                v[p + 'v'] = min(1.0, v[p + 'rpm'] / 1500) * (403 if ph == 'sync' else 400) + (n(0.5) if running else 0)
            v[p + 'oil'] = (450 - 0.15 * v[p + 'ct'] + n(3)) * min(1, v[p + 'rpm'] / 1500) if running else 0.0
            # GEN-03's radiator is due for cleaning: coolant creeps over 95 °C when it carries half the load.
            ct_target = (78 + (0.02 if k == 2 else 0.042) * v[p + 'kw']) if running else 40
            v[p + 'ct'] = ramp(v[p + 'ct'], ct_target, 0.4 if running else 0.06, dt)
            v[p + 'batt'] = (27.5 if running else 26.8) + n(0.03)
            if running:
                v[p + 'fuel'] -= (0.004 + 0.00002 * v[p + 'kw']) * dt
                v[p + 'hours'] += dt / 3600
            if v[p + 'fuel'] < 22:
                v[p + 'fuel'] = 95.0
        syncing = ph == 'sync'
        v['gs_sync_slip'] = (max(0.04, 0.22 - 0.012 * g3['tmr']) + n(0.004)) if syncing else 0.0
        v['gs_sync_phase'] = self.sync_angle if syncing else 0.0
        v['gs_sync_dv'] = abs(v['gs_g3_v'] - v['gs_bus_v']) if syncing else 0.0

    # Main distribution panel: IT load steady, chiller cycling with a VSD (high THD-I), demand against a 700 kW contract.
    def meters(self, dt):
        v, t = self.v, self.t
        chiller_stage = 0.55 + 0.45 * (0.5 + 0.5 * math.sin(t / 210))
        loads = {
            'f1': (150 + 6 * math.sin(t / 130) + n(0.6), 0.985),
            'f2': (142 + 5 * math.sin(t / 150 + 1) + n(0.6), 0.983),
            'f3': (55 + 85 * chiller_stage + n(1.5), 0.86 + 0.04 * chiller_stage),
            'f4': (40 + 5 * math.sin(t / 95) + n(0.6), 0.89),
            'f5': (21 + 2 * math.sin(t / 400) + n(0.3), 0.92),
        }
        thdi = {'f1': 4.2, 'f2': 4.6, 'f3': 24 + 10 * chiller_stage, 'f4': 11, 'f5': 17}
        v_ln = [230.5 + 1.2 * math.sin(t / 200 + ph) + n(0.15) for ph in (0, 2.1, 4.2)]
        kw_sum = kvar_sum = 0.0
        for k, (kw, pf) in loads.items():
            pf = clamp(pf + n(0.002), 0.5, 1)
            v[f'em_{k}_kw'] = kw
            v[f'em_{k}_pf'] = pf
            v[f'em_{k}_i'] = kw * 1000 / (math.sqrt(3) * 400 * pf)
            v[f'em_{k}_thdi'] = thdi[k] + n(0.2)
            v[f'em_{k}_kwh'] += kw * dt / 3600 * 20  # accelerated so counters visibly move
            kw_sum += kw
            kvar_sum += kw * math.tan(math.acos(pf))
        kw_in = kw_sum * 1.015
        kvar_in = kvar_sum * 1.02
        pf_in = kw_in / math.hypot(kw_in, kvar_in)
        i_avg = kw_in * 1000 / (math.sqrt(3) * 400 * pf_in)
        for ph in (1, 2, 3):
            v[f'em_in_v{ph}'] = v_ln[ph - 1]
            v[f'em_in_i{ph}'] = i_avg * (1 + (0.03, -0.02, -0.01)[ph - 1]) + n(1.5)
        v['em_in_kw'] = kw_in
        v['em_in_kvar'] = kvar_in
        v['em_in_pf'] = pf_in
        v['em_in_hz'] = 50 + 0.03 * math.sin(t / 45) + n(0.01)
        v['em_in_thdv'] = 2.4 + 1.6 * chiller_stage + n(0.05)
        v['em_in_thdi'] = sum(thdi[k] * loads[k][0] for k in loads) / kw_sum + n(0.2)
        v['em_in_kwh'] += kw_in * dt / 3600 * 20
        # 15-minute block demand, compressed to 90 s so it tracks the chiller swing.
        self.dmd.append(kw_in)
        self.dmd = self.dmd[-int(90 / dt):]
        v['em_in_dmd'] = sum(self.dmd) / len(self.dmd)
        # Rack PDUs: A01 steady storage, A02 a GPU node whose batch jobs push the inlet past 80 %.
        job = clamp(math.sin(t / 75) * 1.6 - 0.4, 0, 1)
        for r, base in ((1, 14.2), (2, 17.5)):
            p = f'em_pdu{r}_'
            vin = v_ln[r - 1] + n(0.1)
            amps = base + (9.5 * job if r == 2 else 0.8 * math.sin(t / 60)) + n(0.1)
            v[p + 'v'] = vin
            v[p + 'i'] = amps
            v[p + 'kw'] = vin * amps * 0.97 / 1000
            v[p + 'kwh'] += v[p + 'kw'] * dt / 3600 * 20
            target = 23.2 + (2.6 * job if r == 2 else 0) + 0.3 * math.sin(t / 300)
            v[p + 'temp'] += (target - v[p + 'temp']) * 0.02 * dt
            v[p + 'rh'] = 52 + 3 * math.sin(t / 500 + r) + n(0.2)
        it = (loads['f1'][0] + loads['f2'][0]) * 0.955
        v['em_it_kw'] = it
        v['em_pue'] = kw_in / it

    # Booster station: hold 4.5 bar on the header; duty/assist by demand; P-02 (FC 302) trips on overcurrent every 9 min
    # and the ACS580 standby takes over while the header dips.
    def booster(self, dt):
        v, t = self.v, self.t
        demand = 300 + 120 * math.sin(t / 240) + 30 * math.sin(t / 37) + n(3)
        c = t % 540
        p2_trip = 300 <= c < 420
        want = 1 if demand < 190 else (2 if demand < 360 else 3)
        # Lead pumps in order P-01, P-02, P-03; P-04 stays standby unless one trips.
        order = [0, 1, 2, 3]
        avail = [i for i in order if not (i == 1 and p2_trip)]
        duty = set(avail[:want])
        # Header pressure: dips when a duty pump drops out until the replacement ramps up.
        lag = sum(1 for i in duty if v[f'vsd_p{i + 1}_hz'] < 28)
        target_pd = v['vsd_sp'] - 0.45 * lag + 0.04 * math.sin(t / 23)
        v['vsd_pd'] += (target_pd - v['vsd_pd']) * 0.25 * dt + n(0.006)
        flow = max(0.0, demand * min(1.0, v['vsd_pd'] / v['vsd_sp']))
        v['vsd_flow'] = flow
        q_each = flow / max(1, len(duty))
        total_kw = 0.0
        for i, (model, rated) in enumerate(PUMPS):
            p = f'vsd_p{i + 1}_'
            tripped = i == 1 and p2_trip
            v[p + 'fault'] = float(tripped)
            v[p + 'code'] = (13 if 'Danfoss' in model else 2310) if tripped else 0.0
            hz_target = clamp(33 + 15 * q_each / 190, 30, 50) if i in duty else 0.0
            if tripped:
                v[p + 'hz'] = 0.0  # coast on trip
            else:
                v[p + 'hz'] = max(0.0, ramp(v[p + 'hz'], hz_target, 2.5, dt) + (n(0.05) if hz_target else 0))
            hz = v[p + 'hz']
            v[p + 'run'] = float(hz > 0.5)
            v[p + 'rpm'] = hz * 29.6
            kw = (rated * 0.82 * (hz / 50) ** 3 + 1.2) if hz > 0.5 else 0.0
            v[p + 'kw'] = kw + (n(0.1) if kw else 0)
            v[p + 'amp'] = (kw * 1000 / (math.sqrt(3) * 400 * 0.86 * 0.94) + 9) if kw else 0.0
            v[p + 'temp'] += (36 + 0.9 * kw - v[p + 'temp']) * 0.01 * dt
            v[p + 'dc'] = 561 - 0.15 * kw + n(0.8)
            total_kw += v[p + 'kw']
        v['vsd_kw'] = total_kw
        v['vsd_sec'] = total_kw / flow if flow > 20 else 0.0
        v['vsd_ps'] = 0.35 + 0.006 * v['vsd_lvl'] - 0.0003 * flow + n(0.004)
        inflow = 300 + 40 * math.sin(t / 600)
        v['vsd_lvl'] = clamp(v['vsd_lvl'] + (inflow - flow) / 2500 * dt * 0.3, 5, 99)
        v['vsd_sp'] = 4.5


if __name__ == '__main__':
    import argparse
    ap = argparse.ArgumentParser(description='Print the vertical devices as voltrus.yaml device blocks, or run the model for a while.')
    ap.add_argument('--yaml', action='store_true')
    ap.add_argument('--run', type=float, default=0, help='simulate this many seconds and print a summary')
    a = ap.parse_args()
    if a.yaml:
        for dev_id, d in DEVICES.items():
            print(f'  - id: "{dev_id}"\n    ip: "127.0.0.1"\n    port: 5030\n    unit_id: {d["unit_id"]}\n    interval_ms: 1000\n'
                  f'    endianness: "big"\n    registers:')
            for i, k in enumerate(d['tags']):
                print(f'      - name: "{k}"\n        address: {d["base"] + 2 * i}\n        type: "float32"')
    if a.run:
        m = Verticals()
        lo = {k: math.inf for k in META}
        hi = {k: -math.inf for k in META}
        while m.t < a.run:
            m.step(0.5)
            for k, x in m.v.items():
                lo[k], hi[k] = min(lo[k], x), max(hi[k], x)
        for k in META:
            name, unit, mn, mx, warn, crit, dec, lw, lc = META[k]
            flags = []
            if crit and hi[k] >= crit: flags.append('CRIT-HI')
            elif warn and hi[k] >= warn: flags.append('warn-hi')
            if lc is not None and lo[k] <= lc: flags.append('CRIT-LO')
            elif lw is not None and lo[k] <= lw: flags.append('warn-lo')
            print(f'{k:18} {lo[k]:12.2f} {hi[k]:12.2f}  {unit:6} {" ".join(flags)}')
