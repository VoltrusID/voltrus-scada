#!/usr/bin/env python3
"""Results of bench.py (JSON lines) to a Markdown table, one row per run, in matrix order.

    table.py results.jsonl [more.jsonl...]
"""
import json
import sys

COLS = [('label', ''), ('tags', 'N'), ('devices', 'dev'), ('rate_ms', 'ms'), ('change_pct', '%'),
        ('produced_per_s', 'produced/s'), ('delivered_per_s', 'delivered/s'), ('dropped', 'dropped'),
        ('instructions_per_value', 'instr/value'), ('allocs_per_value', 'allocs/value'),
        ('cpu_pct_task_clock', 'CPU %'), ('rss_mib', 'RSS MiB'), ('latency_p50_ms', 'p50 ms'),
        ('latency_p99_ms', 'p99 ms'), ('connect_create_s', 'connect s'), ('error', 'error')]


def fmt(v):
    if v is None:
        return ''
    if isinstance(v, float) and v.is_integer():
        v = int(v)
    if isinstance(v, int) and abs(v) >= 10000:
        return f'{v:,}'
    return str(v)


rows = [json.loads(line) for path in sys.argv[1:] for line in open(path) if line.strip()]
rows.sort(key=lambda r: (r.get('tags', 0), r.get('devices', 1), -r.get('rate_ms', 0), r.get('change_pct', 0),
                         r.get('label', '')))
print('| ' + ' | '.join(h for _, h in COLS) + ' |')
print('|' + '---|' * len(COLS))
for r in rows:
    print('| ' + ' | '.join(fmt(r.get(k)) for k, _ in COLS) + ' |')
