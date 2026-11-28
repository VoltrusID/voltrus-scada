#!/usr/bin/env python3
"""Turn `tshark -T json --no-duplicate-keys` output into a golden file.

Keeps, per frame, the Modbus/TCP PDUs as lists of [field id, value] in tree
order. Only filter-named fields (mbtcp.*, modbus.*) are kept; text nodes
such as "Register 0 (UINT16): 1" are walked into but not recorded.
Usage: flatten.py <tshark.json> <tshark version> <capture name> > golden.json
"""

import json
import sys

PROTOS = ("mbtcp", "modbus")


def walk(node, out):
    for key, value in node.items():
        if isinstance(value, dict):
            walk(value, out)
        elif isinstance(value, list) and value and isinstance(value[0], dict):
            for v in value:
                walk(v, out)
        elif key.split(".")[0] in PROTOS and not key.endswith("_raw"):
            values = value if isinstance(value, list) else [value]
            for v in values:
                out.append([key, v])


def as_list(v):
    if v is None:
        return []
    return v if isinstance(v, list) else [v]


def main():
    packets = json.load(open(sys.argv[1]))
    frames = []
    for p in packets:
        layers = p["_source"]["layers"]
        mbtcp = as_list(layers.get("mbtcp"))
        modbus = as_list(layers.get("modbus"))
        if not mbtcp:
            continue
        pdus = []
        for i, header in enumerate(mbtcp):
            fields = []
            walk(header, fields)
            if i < len(modbus):
                walk(modbus[i], fields)
            pdus.append(fields)
        frames.append({"frame": int(layers["frame"]["frame.number"]), "pdus": pdus})
    # One frame per line keeps diffs of regenerated goldens readable.
    out = sys.stdout
    out.write('{"tshark": %s, "capture": %s, "frames": [\n' % (json.dumps(sys.argv[2]), json.dumps(sys.argv[3])))
    out.write(",\n".join(json.dumps(f, separators=(",", ":")) for f in frames))
    out.write("\n]}\n")


if __name__ == "__main__":
    main()
