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
