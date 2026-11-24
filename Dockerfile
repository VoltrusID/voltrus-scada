# syntax=docker/dockerfile:1.7
#
# Voltrus SCADA Core as a container: one static binary (web client and every
# protocol driver inside) on Alpine. See docs/docker.md.
#
#   docker build -t voltrus-scada .
#   docker run -d -p 3000:3000 -v voltrus-data:/data voltrus-scada
#
# The default target builds everything from source. `--target prebuilt`
# packages a Linux musl release binary placed at dist/voltrus instead (for
# example from voltrus-core-<version>-linux-x86_64.tar.gz) and builds no code.

# ── Web client ──
FROM node:22-alpine AS client
WORKDIR /src/client
COPY client/package.json client/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY client/ ./
# Writes the bundle to /src/static, which the release build embeds.
RUN npm run build

# ── Server: a static musl binary ──
FROM rust:1-alpine AS server
# A C toolchain for the bundled SQLite, ring and QuickJS; clang as the release
# workflow's musl builds use it.
RUN apk add --no-cache build-base clang clang-dev linux-headers
WORKDIR /src
COPY . .
COPY --from=client /src/static ./static
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin voltrus \
    && mkdir -p /out && cp target/release/voltrus /out/voltrus

# ── Runtime base: Alpine, CA certificates and time zones, an unprivileged user ──
FROM alpine:3.21 AS base
RUN apk add --no-cache ca-certificates tzdata \
    && adduser -D -H -u 10001 voltrus \
    && mkdir -p /data /etc/voltrus \
    && chown voltrus:voltrus /data
COPY deploy/docker/config.yaml /etc/voltrus/config.yaml
COPY deploy/docker/entrypoint.sh /usr/local/bin/voltrus-entrypoint
USER voltrus
WORKDIR /data
VOLUME ["/data"]
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s \
    CMD wget -q -O /dev/null http://127.0.0.1:3000/api/v1/health || exit 1
ENTRYPOINT ["/usr/local/bin/voltrus-entrypoint"]

# ── From a release binary (dist/voltrus) ──
FROM base AS prebuilt
COPY --chmod=755 dist/voltrus /usr/local/bin/voltrus

# ── Default: from source ──
FROM base AS runtime
COPY --from=server /out/voltrus /usr/local/bin/voltrus
