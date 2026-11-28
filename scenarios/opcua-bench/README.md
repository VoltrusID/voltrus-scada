# OPC UA client benchmark

How much CPU, memory and latency Voltrus spends per OPC UA value, for N tags at
a given rate. Results and method: [docs/footprint.md](../../docs/footprint.md#opc-ua-client).

| File | |
|---|---|
| `bench.py` | One run: simulator + server, devices imported through the API, warm-up, measured window, one JSON line |
| `matrix.sh` | The matrix in docs/footprint.md, appending to a results file |
| `perfcount.py` | Counts user-space instructions and CPU time of the server and all its threads, over the window only (`perf stat --delay` without perf) |
| `table.py` | Results file(s) to a Markdown table |

## Build

```bash
VOLTRUS_SKIP_UI_EMBED=1 cargo build --release --features alloc-count --bin voltrus --bin opcua-sim
```

`alloc-count` installs a counting global allocator (`src/alloc_count.rs`), so
allocations per value can be reported. It adds one atomic add per allocation;
leave it off for anything but benchmarks.

## Run

```bash
scenarios/opcua-bench/bench.py --bin target/release --tags 10000 --rate-ms 1000 --change-pct 10 \
    --label after --out results.jsonl
scenarios/opcua-bench/matrix.sh target/release after results.jsonl
scenarios/opcua-bench/table.py results.jsonl
```

Needs Linux, Python 3 and `kernel.perf_event_paranoid` at 2 or lower (the
default on most distributions; no root needed).

## What a run does

1. `opcua-sim --tags N --rate-ms M --change-pct P` serves N Double tags
   (`ns=2;s=Bench.T000000`...) and changes P % of them every M ms (a moving
   window, so each tag changes in turn), plus `Bench.SendTime`, the tick's send
   time in Unix ms, in the same notification batch. In this mode the simulator
   allows any number of monitored items per subscription and any message size;
   `--max-items-per-call` sets the MaxMonitoredItemsPerCall it announces and
   enforces (async-opcua's default is 1,000).
2. The server starts with `VOLTRUS_BENCH_STATS=1` (one log line a second:
   values received, values dropped or superseded, allocations) and
   `VOLTRUS_BENCH_LATENCY_TAG=send_time` (logs simulator-to-cache latency each
   time that tag reaches the cache).
3. The devices are created with one `POST /api/v1/project/import` (N monitored
   items split over `--devices`; `send_time` replaces the last data tag on the
   first device) and started with
   `POST /api/v1/opcua/devices/{id}/start`. `connect_create_s` is the time from
   the start requests to the last "created x/y monitored items" log line.
4. After `--warmup` seconds the window opens and closes right after two
   counter lines of the log, so counts and instructions cover the same time.

## Output fields

| Field | |
|---|---|
| `instructions_per_value` | User-space instructions of the whole server process in the window ÷ values delivered. Hardly moves with machine load |
| instr/change (`table.py`) | The same ÷ changes the simulator made to monitored tags: the number to compare between builds, since a server that coalesces (queue size 1) delivers fewer values for the same changes |
| `allocs_per_value` | Heap allocations in the window ÷ values delivered (needs `alloc-count`) |
| `produced_per_s`, `delivered_per_s`, `dropped`, `superseded` | What the simulator changed (among monitored tags), what the client received, what it threw away on a full queue (builds before the per-tag feed), and what was replaced by a newer value of the same tag before the cache took it (builds with it) |
| `cpu_pct_task_clock`, `cpu_pct_noisy` | CPU time of the server in % of one core (perf task clock; /proc utime+stime). Depends on the machine, its clock speed and what else runs |
| `rss_mib`, `pss_mib` | Memory at the end of the window |
| `latency_p50_ms`, `latency_p99_ms` | Simulator send time to cache merge of `send_time`, one sample per tick |
| `connect_create_s`, `items_created`/`items_requested` | Connect, subscription and monitored item creation |

The whole process is measured, not just the OPC UA code: the historian writing
every tag every `persist_interval_secs`, the flow engine and alarm evaluation
are in the numbers too, as they would be on a real site. No browser is
connected, so the WebSocket live stream does no work.
