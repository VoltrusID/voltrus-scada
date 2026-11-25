# Docker image

The repository's `Dockerfile` builds a container with the Core server: the static
Linux binary (web client and every protocol driver inside) on Alpine 3.21, with CA
certificates and time zones, running as an unprivileged user (uid 10001). No image
is published to a registry yet; build it yourself.

```bash
docker build -t voltrus-scada .
docker run -d --name voltrus -p 3000:3000 -v voltrus-data:/data voltrus-scada
# open http://localhost:3000 and sign in as described in the README
```

## Targets

| Target | What it does |
|---|---|
| `runtime` (default) | Builds the web client (Node 22), then the server as a static musl binary (`rust:1-alpine`, `cargo build --release --locked --bin voltrus`), and copies only the binary into the runtime image. Needs several GB of build space; the cargo registry and `target/` are BuildKit cache mounts. |
| `prebuilt` | Builds no code: copies `dist/voltrus` from the build context. Put the `voltrus` file of a Linux musl release archive (`voltrus-core-<version>-linux-x86_64.tar.gz` or `-aarch64`) there and run `docker build --target prebuilt -t voltrus-scada .`. |

The CI workflow (`.github/workflows/ci.yml`, job `docker`) builds the default
target, starts it and checks `/api/v1/health` on every push and pull request. It
does not push the image anywhere.

## Data and configuration

* `/data` is a volume: the SQLite database (`/data/voltrus.db`), the history
  archive and `config.yaml`.
* The server runs as uid 10001, so `/data` must be writable by that user. A named
  volume (`-v voltrus-data:/data`) is set up for it automatically. A bind mount of
  a host directory is not: a directory created by root (or by `docker run -v`
  itself) makes the first start fail because nothing can be written there.
  Give it to the container's user first, `sudo chown -R 10001:10001 /srv/voltrus`,
  then `-v /srv/voltrus:/data`.
* On first start the entrypoint copies the image's `/etc/voltrus/config.yaml`
  (port 3000, database in `/data`) to `/data/config.yaml` and appends a random
  `auth_secret`, so sign-in sessions survive restarts. Edit `/data/config.yaml` to
  change settings (`docker run --rm -it -v voltrus-data:/data alpine vi /data/config.yaml`),
  or mount your own file there before the first start.
* The first admin password is generated on first start and written to
  `/data/initial-admin-password` (read it with
  `docker exec voltrus cat /data/initial-admin-password`); changing it at the
  first sign-in deletes the file.
* Arguments replace the default command, for example
  `docker run --rm -v voltrus-data:/data -v $PWD/site:/site voltrus-scada voltrus project validate /site`.
* A health check calls `GET /api/v1/health` every 30 s.

## Reaching devices

Devices on the plant network are reached through Docker's default bridge
(outbound TCP works as from the host: Modbus 502, S7 102, EtherNet/IP 44818,
DNP3 20000, OPC UA 4840). Two cases need more:

* **BACnet/IP** uses UDP broadcasts (who-is) on the local subnet, which do not
  cross the bridge: run with `--network host`.
* **Devices or simulators on the Docker host itself** (127.0.0.1 in a config)
  need `--network host`, or the host's address instead of 127.0.0.1.

The embedded MQTT broker listens on the ports in `config.yaml`; publish them with
`-p` like port 3000 if clients outside the container use it.

## Measured footprint

Measured on 2026-11-25 on an AMD Ryzen 5 PRO 4650U (6 cores, 12 threads), Docker
29, Linux 7.2. The image was built with `--target prebuilt` from the v0.52.0
Linux x86_64 release binary (the published release when this was measured; the
default target builds the same binary from source).

| | |
|---|---|
| Image size (`docker image ls`, unpacked) | 103 MB (63.4 MB of it is the binary) |
| Image content size (compressed layers, what a pull transfers) | 26.9 MB |

Idle with the factory-floor scenario: the container ran
`scenarios/factory-floor/voltrus.yaml` as `/data/config.yaml` with
`--network host`, polling the PM5560 power meter of that scenario (7 float32
registers every 1 s) from `modbus-sim --scenario factory-floor --role grid-meter`
on the host, with the embedded MQTT broker, history every 5 s and no browser
connected. After 100 s of warm-up, 12 samples of `docker stats` 10 s apart:

| | |
|---|---|
| Memory (`docker stats`) | 29.3 to 29.5 MiB |
| Resident set of the server process (`VmRSS`), peak (`VmHWM`) | 28.3 MiB, 33.4 MiB |
| CPU (`docker stats`, 100 % = one core) | 0.11 % to 0.52 %, mean 0.22 % |

For comparison, the same lab measured FUXA 1.3.6 at 936 MB image and 81 MiB idle
and Rapid SCADA 6.5.1 at 616 MB and 211 MiB (both with a 47-tag Modbus project;
`~/scada-compare/COMPARISON.md`). Those projects are larger than the scenario
above, so the memory figures are not a like-for-like comparison of the same
plant; the image sizes are.

Limits: one architecture per build (no multi-arch manifest yet); no published
image, so `docker pull` is not available; the Enterprise add-on is not part of
this image.
