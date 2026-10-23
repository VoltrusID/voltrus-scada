"""24/7 device poller: run forever, log device snapshots and live values.

The always-on production pattern: fixed-interval polling, transient errors
logged and retried (the loop never dies on a network blip), clean Ctrl-C.

Env vars:
    VOLTRUS_URL       base URL of the server (default http://localhost:3000)
    VOLTRUS_API_KEY   API key; or set USERNAME + PASSWORD instead
    VOLTRUS_USERNAME  login user (used when VOLTRUS_API_KEY is not set)
    VOLTRUS_PASSWORD  login password
    VOLTRUS_INTERVAL  poll interval in seconds (default 5)
    VOLTRUS_DEVICE    optional device id to filter live values by

Usage: python devices_24x7.py
"""

import os
import signal
import sys
import time

from quickstart import make_client

from voltrus import VoltrusError

_running = True


def _stop(signum: int, frame: object) -> None:
    global _running
    _running = False


def poll_once(client: object, device_id: str) -> None:
    snapshots = client.get_live_snapshots()
    online = sum(1 for s in snapshots if s.status == "online")
    print(
        f"[{time.strftime('%H:%M:%S')}] devices {online}/{len(snapshots)} online"
    )
    for s in snapshots:
        metrics = ", ".join(f"{k}={v:g}" for k, v in sorted(s.metrics.items())[:4])
        print(f"    {s.device_id:<24} {s.status:<8} {metrics}")

    values = client.get_live_data(device_id=device_id)
    warnings = [v for v in values if v.status in ("warning", "critical")]
    for v in warnings:
        print(f"    ALARM-RISK {v.key} = {v.value:g} ({v.status})")


def main() -> None:
    client = make_client()
    interval = int(os.environ.get("VOLTRUS_INTERVAL", "5"))
    device_id = os.environ.get("VOLTRUS_DEVICE", "")

    signal.signal(signal.SIGINT, _stop)
    signal.signal(signal.SIGTERM, _stop)

    print(f"Polling every {interval}s — Ctrl-C to stop.")
    while _running:
        started = time.monotonic()
        try:
            poll_once(client, device_id)
        except VoltrusError as e:
            # Transient API errors must not kill a 24/7 process; log and retry.
            print(f"API error {e.status}: {e.message} — retrying next cycle")
        except OSError as e:
            print(f"Network error: {e} — retrying next cycle")

        delay = interval - (time.monotonic() - started)
        if delay > 0 and _running:
            time.sleep(delay)

    print("Stopped.")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
    except SystemExit as e:
        sys.exit(e.code)
