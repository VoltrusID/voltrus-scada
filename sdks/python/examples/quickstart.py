"""Quickstart: connect to Voltrus and read live data.

Reads a screen's live values, or everything when no screen is given.

Env vars:
    VOLTRUS_URL       base URL of the server (default http://localhost:3000)
    VOLTRUS_API_KEY   API key (X-API-Key); or set USERNAME + PASSWORD instead
    VOLTRUS_USERNAME  login user (used when VOLTRUS_API_KEY is not set)
    VOLTRUS_PASSWORD  login password

Usage: python quickstart.py
"""

import os
import sys

from voltrus import VoltrusClient, VoltrusError


def make_client() -> VoltrusClient:
    url = os.environ.get("VOLTRUS_URL", "http://localhost:3000")
    api_key = os.environ.get("VOLTRUS_API_KEY")
    username = os.environ.get("VOLTRUS_USERNAME")
    password = os.environ.get("VOLTRUS_PASSWORD")

    client = VoltrusClient(url, api_key=api_key)
    if not api_key:
        if not username or not password:
            sys.exit("Set VOLTRUS_API_KEY, or VOLTRUS_USERNAME + VOLTRUS_PASSWORD")
        client.login(username, password)
    return client


def main() -> None:
    client = make_client()

    version = client.get_version()
    print(f"Connected to Voltrus {version}")

    screen_id = os.environ.get("VOLTRUS_SCREEN")
    scope = f"screen '{screen_id}'" if screen_id else "all screens"
    values = client.get_live_data(screen_id=screen_id)

    print(f"Live values for {scope}: {len(values)}")
    for v in values:
        marker = {"normal": " ", "warning": "!", "critical": "!!"}.get(v.status, "?")
        print(f"  [{marker:>2}] {v.key:<40} {v.value:>12.2f}  {v.status}")


if __name__ == "__main__":
    try:
        main()
    except VoltrusError as e:
        sys.exit(f"Voltrus API error {e.status}: {e.message}")
