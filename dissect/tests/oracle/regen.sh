#!/usr/bin/env bash
# Regenerate the tshark golden files for voltrus-dissect's oracle tests.
#
#   dissect/tests/oracle/regen.sh            # goldens for every fixture capture
#   dissect/tests/oracle/regen.sh --capture  # first record gen-modbus.pcap again
#   dissect/tests/oracle/regen.sh --dir DIR  # goldens for the captures in DIR
#                                            # (e.g. downloaded samples, not committed)
#   TSHARK_ARGS="-d tcp.port==4446,mbtcp" regen.sh --dir DIR
#                                            # extra tshark options ("Decode As")
#
# tshark runs in a container built from ./Dockerfile (pinned tshark version),
# with no network, reading the captures read-only. It is a test oracle only:
# nothing from Wireshark is linked into or shipped with Voltrus.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
FIXTURES="$HERE/../fixtures/modbus"
IMAGE=voltrus-tshark:4.4.19
CAPTURE=0
DIR="$FIXTURES"
while [ $# -gt 0 ]; do
  case "$1" in
    --capture) CAPTURE=1 ;;
    --dir) DIR="$(cd "$2" && pwd)"; shift ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done

docker build -q -t "$IMAGE" "$HERE" >/dev/null

if [ "$CAPTURE" = 1 ]; then
  # tcpdump on the container's own loopback while the generator talks to itself.
  docker run --rm --user root --network none \
    -v "$HERE:/oracle:ro" -v "$FIXTURES:/out" "$IMAGE" sh -c '
      tcpdump -i lo -U -w /tmp/gen.pcap tcp port 502 2>/dev/null & P=$!
      sleep 1
      python3 -I /oracle/gen_modbus.py
      sleep 1; kill $P; wait $P || true
      cp /tmp/gen.pcap /out/gen-modbus.pcap && chown '"$(id -u):$(id -g)"' /out/gen-modbus.pcap'
fi

VERSION="$(docker run --rm "$IMAGE" tshark --version | head -1)"
shopt -s nullglob
for cap in "$DIR"/*.pcap "$DIR"/*.pcapng; do
  name="$(basename "$cap")"
  echo "golden: $name"
  docker run --rm --network none -v "$DIR:/caps:ro" -v "$HERE:/oracle:ro" "$IMAGE" sh -c '
      tshark -r "/caps/$1" -T json --no-duplicate-keys -o tcp.desegment_tcp_streams:TRUE $3 > /tmp/t.json
      python3 -I /oracle/flatten.py /tmp/t.json "$2" "$1"' sh "$name" "$VERSION" "${TSHARK_ARGS:-}" \
    > "$DIR/${name%.*}.golden.json"
done
