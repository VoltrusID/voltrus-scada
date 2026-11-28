# The oracle: Wireshark's tshark, used only as an external program that
# reads capture files. Nothing from Wireshark is linked or shipped. The base
# image is pinned so the tshark version (and its field names) stays fixed:
# debian:trixie-20251117 gives TShark 4.4.19.
FROM debian:trixie-20251117
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends tshark tcpdump python3 \
 && rm -rf /var/lib/apt/lists/*
RUN useradd -m oracle
USER oracle
