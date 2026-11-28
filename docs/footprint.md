# Footprint: what a SCADA costs an engineer's laptop

Measured on 2026-11-24 and 2026-11-25 on one lab machine (Arch Linux, 31 GB RAM,
Docker 29 with the containerd image store, cgroup v2). Every platform ran the
same small water-treatment project against the same simulator
([`scenarios/wtp-hmi/plant-sim.py`](../scenarios/wtp-hmi/plant-sim.py), 47 float32
Modbus registers polled every second), idle, with no browser or designer open.

| | Voltrus SCADA 0.52.0 | FUXA 1.3.6 | Rapid SCADA 6.5.1 | Ignition 8.3.10 | WinCC OA 3.21 |
|---|---|---|---|---|---|
| Download | 22.5 MB tarball | 936 MB image | 139 MB zip + .NET 10 runtime image | 2.11 GB compressed image | Siemens Debian packages, image built locally |
| On disk | 63 MB binary (web UI and drivers inside) | 936 MB image | 616 MB image | 4.45 GB image, plus 93 MB Designer Launcher and 1.1 GB Designer cache on the engineer's PC | 8.02 GB image |
| Processes | 1 | 1 (Node.js) | 4 (.NET services) | 1 JVM (+ wrapper) | 13 WinCC OA + 11 PostgreSQL |
| Idle RAM | 14–16 MiB | 81 MiB | 191–211 MiB | 880–890 MiB | 235–372 MiB |
| Idle CPU, % of one core | 0.3–0.4 % | ~0.8 % | 1.7–2.5 % | 44–59 % | 15–21 % |
| Start to usable | 0.1–0.6 s (1.0 s on a new database) | 10 s | ~10 s | 48–75 s | 20 s to the first value read |

**Three Voltrus plants side by side**, each with its own simulator, config,
SQLite database and port, all from one copy of the binary: 3 processes, 44 MiB
RSS (29 MiB PSS) and 1.1 % of one core in total; on disk the 63 MB binary plus
24.5 MB of data after about 70 minutes.

## What the numbers do and do not say

- **One Ignition gateway can host several projects.** Devices, tag providers and
  historians are gateway-wide, so three plants in one gateway cost roughly one
  gateway plus their tags. Separate gateways (different versions, modules or
  customers) are each a JVM of their own. We did not measure several copies of
  FUXA, Rapid SCADA, Ignition or WinCC OA at once.
- **Swapping.** The Ignition and WinCC OA memory figures were taken before the
  machine started swapping. On 2026-11-25 the host had 25 GB of swap in use and
  both showed lower resident memory with a large swapped-out share (Ignition
  326 MiB resident + 1.19 GiB swapped), so the earlier figures are the fair ones.
- **Ignition CPU** was 44–49 % about 15 minutes after start and 59 % about 21
  minutes after start (Designer closed a minute earlier). It may settle lower after
  longer; we did not wait longer.
- **Ignition start**: 48 s from the official image; 75 s on the first start of a
  copy unpacked into a directory (used on 2026-11-25 because the image no longer
  fit on the disk).
- Not measured: RAM with clients connected, and anything on Windows or macOS.
  Battery and fan are not measured either; any claim about them is an inference
  from the CPU figures.
- The Docker image of Voltrus is measured separately in [docker.md](docker.md)
  (103 MB image, ~29 MiB idle with a different, smaller scenario).

## How it was measured

- Containers (FUXA, Rapid SCADA, Ignition, WinCC OA):
  [`cstat.sh`](../scenarios/wtp-hmi/footprint/cstat.sh)` 60 <container>` reads the
  container's cgroup: CPU averaged over 60 s, memory = `memory.current −
  inactive_file` (what `docker stats` shows). FUXA and the 880–890 MiB / 44–49 %
  Ignition figures are from `docker stats` on 2026-11-24 with the official image;
  the rest are from 2026-11-25.
- Voltrus:
  [`three-plants.sh`](../scenarios/wtp-hmi/footprint/three-plants.sh)` start <dir>`
  downloads the v0.52.0 Linux release once (checksum verified), starts three
  copies of `plant-sim.py` (ports 5031–5033) and three servers from the same binary
  (ports 3101–3103), each in a `systemd-run --user --scope`, and seeds the WTP tags
  and screens into each with `seed.py`. Then:

  ```bash
  scenarios/wtp-hmi/footprint/cstat.sh 60 scmp-voltrus-plant{1,2,3}.scope
  scenarios/wtp-hmi/footprint/procstat.sh 60 $(pgrep -f 'voltrus voltrus.yaml')
  scenarios/wtp-hmi/footprint/three-plants.sh stop <dir>
  ```

  [`procstat.sh`](../scenarios/wtp-hmi/footprint/procstat.sh) reads RSS and PSS
  from `/proc` and CPU from `utime + stime` over the window.
- The competitor builds (the same project in each product: Modbus link, tags,
  two alarms, history and trend, a process screen, a command written back, one
  reusable object) are lab notes, not part of this repository. Screenshots of
  the same plant on every platform are on
  [voltrus.id/compare](https://voltrus.id/compare/).

Product names are trademarks of their owners. Each product was run from its
official download or image, in its trial, demo or free edition.

## OPC UA client

How much Voltrus spends per OPC UA value, and how many tags one device can
take. Measured on 2026-11-28 with
[`scenarios/opcua-bench`](../scenarios/opcua-bench/README.md) before and after
the changes on branch `perf/opcua-hot-path` (client handles instead of node id
strings, a coalescing per-tag feed instead of a channel that dropped values,
copy-on-write device snapshots, chunked monitored item creation, up to 5000
values per publish response).

### Method

- One simulator (`opcua-sim --tags N --rate-ms M --change-pct P`, async-opcua
  0.18 server) and one Voltrus server on the same machine. N monitored items
  are split over 1 or 10 devices and imported through
  `POST /api/v1/project/import`, started through the API. 20 s warm-up, then a
  60 s window; security mode None.
- **Instructions per change** (the number to compare): user-space instructions
  of the whole Voltrus process over the window, counted with
  `perf_event_open` (`perfcount.py`), divided by the changes the simulator
  made to monitored tags. It includes everything the server does in that time:
  the historian writing every tag every 5 s, alarms, OEE, the flow engine.
- **Allocations per value**: heap allocations in the window (counting global
  allocator, `--features alloc-count`) per value delivered.
- CPU % of one core (perf task clock) and RSS at the end of the window, for
  orientation only: other builds ran on the laptop at the same time.
- Latency: the simulator writes its send time into a tag with every tick;
  Voltrus logs the delay when it reaches the cache. One sample per tick.
- Builds: `before` is `main` (b16d2dc) plus the counters (974d8d3); `before+H6`
  is that with only the monitored item creation chunked and 1000 values per
  publish response, to see the old value path at sizes where `before` cannot
  connect; `after` is the branch head.

### Results

Laptop, glibc release build, 60 s per row. "Changes/s" is what the simulator
changed among the monitored tags. **instr/change** is the comparable number;
CPU % and RSS are noisy (other builds ran) and RSS is a glibc build, see the
caveats. "old path" is `before+H6` from 1,000 tags up and `before` below.
"no connect": the device never subscribed; "0 values": it subscribed but
nothing reached the cache in the window; "(x % delivered)": share of the
changes that reached the cache.

| Tags | Dev | ms | % | Changes/s | instr/change: before | before+H6 | after | allocs/value: before | before+H6 | after | dropped: old path | after | CPU %: old path | after | RSS MiB after | p50 / p99 ms after | connect s after |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 100 | 1 | 1,000 | 10 | 11 | 192,656 | – | 150,599 | 114.35 | – | 36.22 | 0 | 0 | 0.25 | 0.25 | 86 | 506 / 906 | 0.18 |
| 100 | 1 | 1,000 | 100 | 100 | 26,892 | – | 18,605 | 15.84 | – | 3.93 | 0 | 0 | 0.27 | 0.26 | 86 | 506 / 906 | 0.55 |
| 100 | 1 | 100 | 10 | 109 | 59,455 | – | 23,513 | 72.7 | – | 8.23 | 0 | 0 | 0.72 | 0.58 | 86 | 6 / 7 | 0.24 |
| 100 | 1 | 100 | 100 | 1,000 | 11,509 | – | 4,465 | 11.38 | – | 0.83 | 0 | 0 | 0.88 | 0.62 | 86 | 6 / 7 | 0.24 |
| 1,000 | 1 | 1,000 | 10 | 101 | 0 values | 140,547 | 89,216 | 0 values | 102.01 | 20.23 | 0 | 0 | 0.58 | 0.43 | 73 | 406 / 906 | 0.45 |
| 1,000 | 1 | 1,000 | 100 | 1,000 | 0 values | 22,787 | 11,146 | 0 values | 18.57 | 2.04 | 0 | 0 | 0.96 | 0.51 | 90 | 507 / 908 | 0.2 |
| 1,000 | 1 | 100 | 10 | 1,009 | 0 values | 50,250 | 11,816 | 0 values | 63.17 | 2.5 | 0 | 0 | 2.09 | 0.74 | 69 | 6 / 8 | 0.18 |
| 1,000 | 1 | 100 | 100 | 10,000 | 0 values | 16,084 | 3,205 | 0 values | 19.48 | 0.25 | 0 | 0 | 5.36 | 1.32 | 92 | 8 / 10 | 0.45 |
| 10,000 | 1 | 1,000 | 10 | 1,001 | no connect | 155,767 (96% delivered) | 87,165 | no connect | 127.15 | 18.5 | 4 % | 0 | 4.51 | 2.38 | 142 | 591 / 992 | 0.39 |
| 10,000 | 1 | 1,000 | 100 | 10,000 | no connect | 31,691 (54% delivered) | 10,945 | no connect | 60.8 | 1.86 | 46 % | 0 | 10.3 | 3.15 | 122 | 610 / 917 | 0.17 |
| 10,000 | 1 | 100 | 10 | 10,009 | no connect | 67,028 (87% delivered) | 11,040 | no connect | 97.83 | 1.9 | 13 % | 0 | 21.39 | 3.44 | 120 | 101 / 104 | 0.43 |
| 10,000 | 1 | 100 | 100 | 100,000 | no connect | 22,248 (52% delivered) | 3,116 | no connect | 55.72 | 0.22 | 48 % | 0 | 67.72 | 11.03 | 132 | 25 / 37 | 0.35 |
| 10,000 | 10 | 1,000 | 10 | 1,001 | 46,614 (69% delivered) | 73,571 | 55,235 | 41.86 | 44.53 | 17.1 | 0 | 0 | 2.06 | 1.82 | 137 | 402 / 904 | 0.65 |
| 10,000 | 10 | 1,000 | 100 | 10,000 | 0 values | 21,712 | 10,879 | 0 values | 17.37 | 1.93 | 1 % | 0 | 7.68 | 3.44 | 141 | 419 / 916 | 0.57 |
| 10,000 | 10 | 100 | 10 | 10,009 | 15,753 (60% delivered) | 27,939 | 11,106 | 24.05 | 26.81 | 2.03 | 0 | 0 | 10.72 | 4.14 | 147 | 93 / 96 | 0.76 |
| 10,000 | 10 | 100 | 100 | 100,000 | 0 values | 11,204 (85% delivered) | 3,129 | 0 values | 13.89 | 0.24 | 15 % | 0 | 45.17 | 12.18 | 152 | 18 / 27 | 0.77 |
| 50,000 | 1 | 1,000 | 10 | 5,001 | no connect | – | 89,994 | no connect | – | 20.38 | no connect | 0 | no connect | 12.08 | 416 | 504 / 1003 | 0.61 |
| 50,000 | 1 | 1,000 | 100 | 50,000 | no connect | – | 11,249 | no connect | – | 1.89 | no connect | 0 | no connect | 17.58 | 432 | 564 / 1056 | 0.66 |
| 50,000 | 1 | 100 | 10 | 50,009 | no connect | – | 13,141 | no connect | – | 6.07 | no connect | 0 | no connect | 19.66 | 385 | 100 / 110 | 0.61 |
| 50,000 | 1 | 100 | 100 | 500,000 | no connect | – | 3,244 | no connect | – | 0.35 | no connect | 0 | no connect | 60.87 | 435 | 142 / 174 | 0.89 |
| 50,000 | 10 | 1,000 | 10 | 5,001 | no connect | – | 55,804 | no connect | – | 17.11 | no connect | 0 | no connect | 4.99 | 388 | 595 / 994 | 0.56 |
| 50,000 | 10 | 1,000 | 100 | 50,000 | no connect | – | 10,833 | no connect | – | 1.83 | no connect | 0 | no connect | 13.08 | 411 | 426 / 1026 | 0.66 |
| 50,000 | 10 | 100 | 10 | 50,009 | no connect | – | 10,926 | no connect | – | 1.9 | no connect | 0 | no connect | 14.08 | 374 | 102 / 113 | 0.66 |
| 50,000 | 10 | 100 | 100 | 500,000 | no connect | – | 3,091 | no connect | – | 0.21 | no connect | 0 | no connect | 44.05 | 464 | 80 / 132 | 0.78 |

**What a tag costs when nothing changes** (only the latency tag changes, once
a second): with 10,000 tags, 84.7 M instructions/s and 18,500 allocations/s
after (97.8 M and 57,000 old path), 2.5 % of one laptop core; with 1,000 tags,
8.7 M/s. This is work done for every tag every few seconds whatever the
device sends: the historian writes every tag every `persist_interval_secs`,
alarm and OEE evaluation read every snapshot. In the 10,000 tags / 1 s / 10 %
row it is 97 % of the 87 M instructions/s; the OPC UA values themselves cost
about 2,500 instructions each.

### What it shows

- **Before, a device of more than ~1,000 tags did not work.** All monitored
  items went in one CreateMonitoredItems call: 10,000 items made a request
  over the 320 kB message limit (`BadRequestTooLarge`), and 1,000 or more
  made a response the client refused to decode (async-opcua decodes arrays
  of at most 1,000 elements, `BadDecodingError`). With exactly 1,000 tags the
  session subscribed but dropped on the first publish response carrying more
  than 1,000 values (initial values plus the first changes), reconnected with
  backoff and delivered nothing. Ten devices of 1,000 tags delivered 60-69 %
  at 10 % change and nothing at 100 %. After: items are created in calls of
  at most the server's MaxMonitoredItemsPerCall (capped at 1,000), the server
  is asked for at most 5,000 values per publish response and the client
  decodes arrays that long. 50,000 tags on one device subscribe in 0.6-0.9 s.
  Remote commissioning sites (300 tags) were never affected.
- **The old value path dropped values silently under load** (H4): with the
  creation fixed (`before+H6`), 10,000 tags on one device lost 46 % of the
  changes at 1 s / 100 % and 48 % at 100 ms / 100 %, through the 1,024-slot
  channel's `try_send`. After: a per-tag feed keeps each tag's latest change
  until the cache takes it; nothing is dropped and a newer value of the same
  tag replacing an untaken one is counted
  (`voltrus_opcua_values_superseded_total` in `/metrics`).
- **Per value, 61-86 % fewer instructions and 93-99.6 % fewer allocations**
  where values dominate (100 ms, 100 % change: at 10,000 tags 22,248 → 3,116
  instructions and 56 → 0.22 allocations per value, 67.7 % → 11.0 % of a
  core). At 1 s the gain is smaller because the fixed per-tag cost below
  dominates. This is H1 (a full copy of the device snapshot, three maps with
  String keys, for every batch; plus a JSON message of every metric built for
  the flow engine, which no trigger could match for OPC UA) and H2 (a NodeId
  formatted to a String, a HashMap lookup and two name clones per value).
- **Thin LTO + one codegen unit (H10)**, same source as `after` at commit
  52aa9ab, 12 cells: 2-12 % fewer instructions per change (median -4.8 %),
  binary 76 → 55 MB (glibc, no UI, unstripped), 12 minutes for a clean
  release build on this laptop. Not adopted: a `[profile.release]` applies
  to the whole workspace, including the native console (GPUI), and the
  release workflow was not run. Setting `CARGO_PROFILE_RELEASE_LTO=thin` and
  `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1` in the release job, which builds
  only `voltrus` and `modbus-sim`, is the cheap way to try it.
- **Queue size 1 (H5)** was tried and not kept. At 100 ms against this server
  it delivered a third fewer values (the server coalesced) and 2-11 % fewer
  instructions per change. Its latency looked worse in one series (p50 ~90
  ms against ~15 ms, also with an independent Python asyncua client) and
  better in a repeat (p50 8.5 / p99 10 ms against 12 / 113 ms): at 100 ms the
  phase between simulator and server timers outweighs it. Queue size stays
  10, as before, until it is tried against real servers.
- **Source timestamps (H5)**: the cache now keeps the server's source time
  per tag (`metric_ts_ms`, shown as the tag's time in the live stream, alarms
  and flows; ignored if more than 5 s ahead of our clock). The historian still
  stores one local timestamp per device per cycle; changing that is a
  historian change, not done here.
- Not measured: decoding inside async-opcua (H3) on its own, since no
  profiler was available (no perf, no valgrind); it is inside the ~3,100
  instructions per value at 100 ms. Sign and SignAndEncrypt. Reconnect and
  browse times.

### Caveats

- **A laptop, not a Raspberry Pi.** AMD Ryzen 5 PRO 4650U (6 cores, 12
  threads, up to 4 GHz), Arch Linux. Instruction counts carry over to other
  x86 machines better than CPU %; an ARM core runs different instructions,
  so the Pi numbers have to be measured there.
- **Other builds ran at the same time** on this machine. Instructions and
  allocations hardly move with load; CPU % and latency do.
- **glibc, not the musl release.** These are `cargo build --release` glibc
  builds with the counting allocator. That build idles at 37 MiB RSS without
  devices (the musl release in the table at the top: 14-16 MiB), so compare
  RSS between rows here, not with the table above. RSS grows with tags:
  ~140 MiB at 10,000 and ~400 MiB at 50,000 (cache, client monitored item
  state, SQLite, glibc arenas).
- **One server implementation.** The simulator is async-opcua's server, on
  the same machine. Its limits (1,000 items per call, 20 queued notification
  messages per subscription) and its sampling behaviour shape these numbers;
  Siemens, Kepware or Beckhoff servers have their own.
- **No browser connected**, so the WebSocket live stream did no work. With
  clients connected it reads every device snapshot on every cache update.
- **Latency at 100 ms** depends on the phase between the simulator's tick and
  the server's 100 ms subscription timer, which differs per run: differences
  below one interval mean nothing. At 1 s the p99 is the publishing interval.

### Still to measure on a Raspberry Pi 4/5

The spec's target, 10,000 tags at 1 s, 10 % change, one device, under 15 % of
one core and 64 MiB RSS, with the `aarch64-unknown-linux-musl` release build
of this branch: the same `bench.py` runs there (`perfcount.py` knows
aarch64). On the laptop that row is 2.4 % of a core and 142 MiB RSS (glibc);
whether musl and the Pi meet 64 MiB is the open question, and the fixed
per-tag cost (historian) is what to look at next if CPU is short. Also 1,000
tags at 100 ms, and a musl build with thin LTO.

