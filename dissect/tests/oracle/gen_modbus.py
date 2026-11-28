#!/usr/bin/env python3
"""Generate Modbus/TCP traffic on 127.0.0.1:502 for the oracle captures.

A small Modbus/TCP server and client written from the Modbus Application
Protocol Specification v1.1b3 and Modbus Messaging on TCP/IP v1.0b. The
client sends every function code voltrus-dissect decodes, the exceptions it
explains, pipelined and split requests (TCP reassembly) and one request the
server never answers. Run inside the oracle container while tcpdump records
`lo`; see regen.sh. Standard library only.
"""

import socket
import struct
import sys
import threading
import time

HOST, PORT = "127.0.0.1", 502
MAP_SIZE = 1000          # addresses 0..999 exist, 1000+ answer exception 02
SILENT_UNIT = 0x63       # requests to this unit id are never answered
BUSY_UNIT = 0x62         # answered with exception 06 (server busy)
GATEWAY_UNIT = 0x61      # answered with exception 0B (gateway target failed)


def reg(addr):
    return (addr * 7 + 0x100) & 0xFFFF


def coil(addr):
    return addr % 3 == 0


def pack_bits(bits):
    out = bytearray((len(bits) + 7) // 8)
    for i, b in enumerate(bits):
        if b:
            out[i // 8] |= 1 << (i % 8)
    return bytes(out)


def exception(fc, code):
    return bytes([fc | 0x80, code])


def in_map(start, qty):
    return start + qty <= MAP_SIZE


def handle(unit, pdu):
    """Answer one request PDU; None means stay silent."""
    if unit == SILENT_UNIT:
        return None
    fc = pdu[0]
    if unit == BUSY_UNIT:
        return exception(fc, 0x06)
    if unit == GATEWAY_UNIT:
        return exception(fc, 0x0B)
    body = pdu[1:]
    if fc in (1, 2):
        start, qty = struct.unpack(">HH", body[:4])
        if not 1 <= qty <= 2000:
            return exception(fc, 3)
        if not in_map(start, qty):
            return exception(fc, 2)
        data = pack_bits([coil(a) if fc == 1 else not coil(a) for a in range(start, start + qty)])
        return bytes([fc, len(data)]) + data
    if fc in (3, 4):
        start, qty = struct.unpack(">HH", body[:4])
        if not 1 <= qty <= 125:
            return exception(fc, 3)
        if not in_map(start, qty):
            return exception(fc, 2)
        data = b"".join(struct.pack(">H", reg(a) ^ (0x8000 if fc == 4 else 0)) for a in range(start, start + qty))
        return bytes([fc, len(data)]) + data
    if fc == 5:
        addr, value = struct.unpack(">HH", body[:4])
        if value not in (0xFF00, 0x0000):
            return exception(fc, 3)
        if addr >= MAP_SIZE:
            return exception(fc, 2)
        return pdu
    if fc == 6:
        addr, _ = struct.unpack(">HH", body[:4])
        if addr >= MAP_SIZE:
            return exception(fc, 2)
        if addr == 999:
            return exception(fc, 4)  # this one register is "faulty"
        return pdu
    if fc in (15, 16):
        start, qty = struct.unpack(">HH", body[:4])
        if not in_map(start, qty):
            return exception(fc, 2)
        return bytes([fc]) + body[:4]
    if fc == 22:
        addr = struct.unpack(">H", body[:2])[0]
        if addr >= MAP_SIZE:
            return exception(fc, 2)
        return pdu
    if fc == 23:
        rstart, rqty, wstart, wqty = struct.unpack(">HHHH", body[:8])
        if not (in_map(rstart, rqty) and in_map(wstart, wqty)):
            return exception(fc, 2)
        data = b"".join(struct.pack(">H", reg(a)) for a in range(rstart, rstart + rqty))
        return bytes([fc, len(data)]) + data
    if fc == 43 and body[:1] == b"\x0e":
        code, obj = body[1], body[2]
        objects = {0: b"JoyoDigitama", 1: b"VTS-OR4CLE", 2: b"v1.2.3"}
        if code != 1:
            return exception(fc, 3)
        if obj not in objects:
            return exception(fc, 2)
        listed = [(i, v) for i, v in sorted(objects.items()) if i >= obj]
        out = bytes([fc, 0x0E, code, 0x81, 0x00, 0x00, len(listed)])
        for i, v in listed:
            out += bytes([i, len(v)]) + v
        return out
    return exception(fc, 1)


def recv_exact(sock, n):
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("closed")
        buf += chunk
    return buf


def serve(listener):
    conn, _ = listener.accept()
    conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    try:
        while True:
            head = recv_exact(conn, 7)
            tid, proto, length, unit = struct.unpack(">HHHB", head)
            pdu = recv_exact(conn, length - 1)
            reply = handle(unit, pdu)
            if reply is None:
                continue
            time.sleep(0.002)  # a measurable, stable response time
            conn.sendall(struct.pack(">HHHB", tid, proto, len(reply) + 1, unit) + reply)
    except ConnectionError:
        pass
    finally:
        conn.close()


class Client:
    def __init__(self):
        self.sock = socket.create_connection((HOST, PORT))
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.tid = 0

    def adu(self, pdu, unit=1):
        self.tid += 1
        return struct.pack(">HHHB", self.tid, 0, len(pdu) + 1, unit) + pdu

    def read_reply(self):
        head = recv_exact(self.sock, 7)
        length = struct.unpack(">H", head[4:6])[0]
        return head + recv_exact(self.sock, length - 1)

    def call(self, pdu, unit=1):
        self.sock.sendall(self.adu(pdu, unit))
        self.read_reply()
        time.sleep(0.01)


def main():
    listener = socket.socket()
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind((HOST, PORT))
    listener.listen(1)
    server = threading.Thread(target=serve, args=(listener,), daemon=True)
    server.start()

    c = Client()
    p = struct.pack
    # Reads.
    c.call(b"\x01" + p(">HH", 0, 10))
    c.call(b"\x01" + p(">HH", 20, 19))
    c.call(b"\x02" + p(">HH", 100, 8))
    c.call(b"\x03" + p(">HH", 0, 10))
    c.call(b"\x03" + p(">HH", 500, 2))
    c.call(b"\x04" + p(">HH", 7, 4))
    # Writes.
    c.call(b"\x05" + p(">HH", 12, 0xFF00))
    c.call(b"\x05" + p(">HH", 13, 0x0000))
    c.call(b"\x06" + p(">HH", 40, 1234))
    c.call(b"\x0f" + p(">HHB", 30, 10, 2) + bytes([0xCD, 0x01]))
    c.call(b"\x10" + p(">HHB", 60, 3, 6) + p(">HHH", 1, 0xFFFF, 0x8000))
    c.call(b"\x16" + p(">HHH", 4, 0xF2F2, 0x0025))
    c.call(b"\x17" + p(">HHHHB", 10, 3, 70, 2, 4) + p(">HH", 0x00AA, 0x5500))
    # Device identification (MEI 0x0E), basic stream.
    c.call(b"\x2b\x0e\x01\x00")
    # Exceptions.
    c.call(b"\x03" + p(">HH", 1000, 1))      # 02 illegal data address
    c.call(b"\x03" + p(">HH", 995, 10))      # 02, range runs past the map
    c.call(b"\x03" + p(">HH", 0, 200))       # 03 illegal data value (qty > 125)
    c.call(b"\x05" + p(">HH", 1, 0x1234))    # 03 illegal coil value
    c.call(b"\x06" + p(">HH", 999, 7))       # 04 server device failure
    c.call(b"\x07")                          # 01 illegal function
    c.call(b"\x03" + p(">HH", 0, 1), unit=BUSY_UNIT)     # 06 busy
    c.call(b"\x04" + p(">HH", 0, 1), unit=GATEWAY_UNIT)  # 0B gateway target
    c.call(b"\x2b\x0e\x01\x09")              # 02 unknown object id
    # Two requests in one TCP segment.
    c.sock.sendall(c.adu(b"\x03" + p(">HH", 1, 1)) + c.adu(b"\x04" + p(">HH", 2, 2)))
    c.read_reply()
    c.read_reply()
    time.sleep(0.01)
    # One request split over two TCP segments.
    split = c.adu(b"\x10" + p(">HHB", 80, 2, 4) + p(">HH", 0x0102, 0x0304))
    c.sock.sendall(split[:9])
    time.sleep(0.05)
    c.sock.sendall(split[9:])
    c.read_reply()
    time.sleep(0.01)
    # A request nobody answers, then a normal one that must still pair.
    c.sock.sendall(c.adu(b"\x03" + p(">HH", 0, 1), unit=SILENT_UNIT))
    time.sleep(0.05)
    c.call(b"\x03" + p(">HH", 2, 1))
    c.sock.close()
    server.join(timeout=2)
    return 0


if __name__ == "__main__":
    sys.exit(main())
