# Protocol traffic

The Traffic view shows the requests Voltrus sends to a device and the
responses it gets back, decoded field by field, each response paired with its
request and timed, errors explained in plain words, and each frame linked to
the tags it read or wrote. It answers the commissioning questions a packet
capture usually answers ("the tag reads 0: is the address off by one, or is
the device returning exception 02?") without a mirror port or Wireshark on the
plant PC, because Voltrus is one end of the conversation.

Status: **Modbus TCP**. DNP3, S7, EtherNet/IP and BACnet are planned on the
same model; pcap import and export are not built yet (see [What is not
built yet](#what-is-not-built-yet)).

## Using it

In **Devices → Modbus**, press **Traffic** on a device. The list fills as the
device is polled:

- **Time, direction, summary, latency, error.** `→ req` is Voltrus asking,
  `← resp` the device answering. Summaries give the wire address and the
  1-based reference (`Read Holding Registers 0 x10 (40001-40010)`).
- **Filters:** function code, data table (coils, discrete inputs, input or
  holding registers), an address range (0-based wire addresses; a frame
  matches when its range overlaps) and *Errors only*. The server applies them.
- **Pause** stops updating the list so you can read it; the capture itself
  keeps running.
- Select a frame for its **field tree** and **hex pane**. Selecting a field
  highlights its bytes; clicking a byte selects the field it belongs to. The
  request and response of a pair are both marked.
- **Tags** lists the configured registers the frame read or wrote, with the
  tag that uses each one; on a response, clicking one highlights the bytes
  its value came from.
- An error shows what it means and the likely cause, for example:

  > **Exception 02: Illegal data address.** The address is outside the
  > device's register map, or the request runs past the end of a block of
  > registers. Most often the address is off by one: manuals number registers
  > from 1 (40001 is wire address 0) while others list 0-based offsets. Check
  > the device manual and the register's zero-based setting; if only a batched
  > read fails, the batch may span a gap between two blocks. The request was
  > Read Holding Registers at wire address 1000 (41001), 1 register.

  Requests that get no answer within 5 s (the poller's timeout), or that were
  open when the connection was replaced, are marked **No response**.

Field names in the tree are Wireshark's display-filter names
(`mbtcp.trans_id`, `modbus.func_code`, `modbus.exception_code`,
`modbus.regval_uint16`, ...): hover a field to see its name and byte range, and
use the same name as a filter if you open a capture in Wireshark later.

## Who can see it

Traffic contains plant data such as setpoints, so it is for the roles that
operate the plant: `operator`, `admin`, `demo` and `integration` API keys (an
integration key with `["read"]` permissions is enough). Viewers and read-only
`api` keys get 403, and the Traffic button is hidden for them.

## Capture and memory

Nothing is recorded until someone looks. Opening the view (or a REST or MCP
call) starts a **10-minute capture** on that device, and every further look
extends it. While it runs, each complete frame is copied into a ring buffer of
the device: up to **2,000 frames or 2 MB**, whichever comes first, oldest
dropped first. When the capture runs out, recording stops and the ring is
freed. Decoding, pairing and tag mapping happen when the frames are read, not
on the polling path.

The tap is a pass-through wrapper around the TCP stream that the Modbus
driver (tokio-modbus) talks over, for polling and for writes. It never
delays, changes or drops bytes, so polling behaves the same whether a device
is watched or not. Measured on a release build, per poll of one 10-register
read (a 12-byte request and a 29-byte response):

| | Cost per poll |
|---|---|
| Device not watched | 7 ns (one atomic load per read and write call, no clock read) |
| Device watched | 2.8 µs: one clock read per frame, a small allocation and an uncontended mutex. The clock read dominates; it costs ~1.4 µs on the measuring machine, whose kernel uses the HPET clock source, and about 20 ns with the usual TSC |
| Decoding on read | 10 µs per frame, so 20 ms for a full ring of 2,000 |

A poll at 1 s intervals of a watched device therefore spends well under
0.01 % of a core on the tap.

Repeat the measurement with
`cargo test --release --lib modbus::tap::tests::overhead -- --ignored --nocapture`.

## REST

`GET /api/v1/devices/{id}/traffic` (Modbus devices; 404 for others).

| Query | Meaning |
|---|---|
| `function` | Only this function code (1-127, without the exception bit). |
| `errors=true` | Only frames with an error. |
| `space` | `coil`, `discrete`, `input` or `holding`. |
| `addr_from`, `addr_to` | Only frames whose address range overlaps these 0-based wire addresses. |
| `after` | Only frames with `seq` greater than this (polling). |
| `limit` | Newest frames to return, 1-2000, default 200. |
| `detail=false` | Leave out `fields` and `raw`. |
| `watch=false` | Read without starting or extending the capture. |

```json
{
  "device_id": "meter-1", "protocol": "modbus_tcp",
  "capturing": true, "capture_until_ms": 1795833000000,
  "capacity": 2000, "kept": 312, "evicted": 0,
  "frames": [{
    "seq": 41, "ts_us": 1795832400123456, "dir": "rx", "proto": "modbus_tcp",
    "summary": "Read Holding Registers response, 2 registers",
    "function": 3, "range": {"space": "holding", "start": 0, "count": 2},
    "pair_id": 40, "latency_us": 2213, "error": null,
    "raw": "0029000000070103040001000a",
    "fields": [{"id": "", "display": "Modbus/TCP, transaction 41, unit 1", "offset": 0, "len": 7,
                "children": [{"id": "mbtcp.trans_id", "label": "Transaction Identifier",
                              "value": 41, "display": "Transaction Identifier: 41", "offset": 0, "len": 2}, "..."]}, "..."],
    "tag_refs": [{"key": "volts", "tag_id": 7, "tag_name": "Voltage L1", "offset": 9, "len": 2}]
  }]
}
```

Frames come oldest first. `seq` increases per device; `pair_id` is the `seq`
of the matching request or response and `latency_us` is set on both. `error`
has `kind` (`exception`, `malformed`, `no_response`), `code`, `title` and
`explanation`. The first call on an idle device returns no frames and a
`note` that the capture has started.

## MCP

`get_device_traffic` (read; operating roles) returns the same data for an
agent: `device_id` plus the filters above, `limit` up to 500 (default 50) and
`detail` (default false: summaries, pairing, errors and tags, no field tree).
Its first call starts the capture, so an agent asks again after a few poll
intervals. See [mcp.md](mcp.md).

## How decoding is checked

The decoders live in the `voltrus-dissect` crate (`dissect/`): pure Rust,
no I/O, builds for `wasm32-unknown-unknown`, written from the protocol
specifications. Wireshark's code is not copied, translated or linked. Instead
tshark is used as an outside oracle:

- `dissect/tests/oracle/gen_modbus.py` talks Modbus/TCP to itself (every
  decoded function, exceptions 01, 02, 03, 04, 06 and 0B, two requests in one
  TCP segment, one request split over two, one request never answered) while
  tcpdump records it; `regen.sh --capture` does this in a container.
- `regen.sh` runs tshark (4.4.19, pinned through `dissect/tests/oracle/Dockerfile`,
  no network) with `-T json` on each committed capture and stores the fields
  as a golden file next to it.
- `cargo test -p voltrus-dissect` decodes the captures and compares every
  field by id and value with the golden files, including tshark's pairing
  (`modbus.request_frame`, `modbus.response_time`). No tshark is needed to run
  it. Other captures can be checked the same way without committing them:
  `regen.sh --dir DIR` then `VOLTRUS_ORACLE_DIR=DIR cargo test -p voltrus-dissect`.

Fully compared: functions 1-6, 15, 16, 22, 23 and 43/14 (Read Device
Identification). Other function codes are shown with their data undecoded
and only their header, function and exception fields are compared.
Known differences from tshark 4.4: tshark does not put the pairing fields on
exception responses (Voltrus pairs those too), and bytes on port 502 that are
not Modbus/TCP are left undissected by tshark while Voltrus shows them as a
malformed frame.

## What is not built yet

- DNP3, S7, EtherNet/IP and BACnet traffic (the drivers have their own
  framing code; DNP3 is next).
- Live push over the WebSocket (the view polls once a second).
- pcap/pcapng import (the reader exists in `voltrus-dissect`), export to
  pcapng/CSV, captures to disk, alarm-triggered captures.
- Per-device statistics (requests/s, latency percentiles, exceptions by code).
- Jumping from a tag or alarm to its frames.
