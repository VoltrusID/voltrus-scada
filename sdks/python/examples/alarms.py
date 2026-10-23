"""List alarms and acknowledge them.

Env vars:
    VOLTRUS_URL       base URL of the server (default http://localhost:3000)
    VOLTRUS_API_KEY   API key; or set USERNAME + PASSWORD instead
    VOLTRUS_USERNAME  login user (used when VOLTRUS_API_KEY is not set)
    VOLTRUS_PASSWORD  login password
    VOLTRUS_ACK       set to 'yes' to acknowledge the listed active alarms

Usage: python alarms.py
"""

import os
import sys

from quickstart import make_client

from voltrus import VoltrusError


def main() -> None:
    client = make_client()

    summary = client.get_alarm_summary()
    print(
        f"Alarms — active: {summary.active}, acked: {summary.acknowledged}, "
        f"shelved: {summary.shelved} (critical: {summary.critical}, "
        f"warning: {summary.warning}, info: {summary.info})"
    )

    alarms = client.list_alarms(status="active")
    if not alarms:
        print("No active alarms.")
        return

    for a in alarms:
        acked = f" ack by {a.acknowledged_by}" if a.acknowledged_by else ""
        print(f"  #{a.id} [{a.severity:>8}] {a.source_name}: {a.message}{acked}")

    if os.environ.get("VOLTRUS_ACK", "").lower() not in ("yes", "1", "true"):
        print("\nSet VOLTRUS_ACK=yes to acknowledge these alarms.")
        return

    for a in alarms:
        client.acknowledge_alarm(a.id)
        print(f"Acknowledged #{a.id} ({a.source_name})")


if __name__ == "__main__":
    try:
        main()
    except VoltrusError as e:
        sys.exit(f"Voltrus API error {e.status}: {e.message}")
