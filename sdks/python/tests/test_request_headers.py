"""Regression test: cookie-authenticated requests must keep Content-Type.

The Python client stores session cookies and replays them as per-request
headers. urllib3 replaces the pool-level default headers with per-request
ones, so once a Cookie header is attached the pool's
``Content-Type: application/json`` default disappears — and every
authenticated POST fails with 415. See the header-merge in
``VoltrusClient._request``.

Runs against a local stdlib HTTP server; no Voltrus instance needed.
"""

import json
import os
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from voltrus import VoltrusClient  # noqa: E402


class _CaptureHandler(BaseHTTPRequestHandler):
    """Records headers of every request, answers JSON, sets a session cookie."""

    seen: list = []

    def _respond(self):
        _CaptureHandler.seen.append(
            {k.lower(): v for k, v in self.headers.items()}
        )
        body = json.dumps({"ok": True}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Set-Cookie", "voltrus_token=t-123; Path=/; HttpOnly")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_POST = _respond
    do_GET = _respond

    def log_message(self, *args):  # silence request logging
        pass


class CookieHeaderTests(unittest.TestCase):
    def setUp(self):
        _CaptureHandler.seen = []
        self.server = HTTPServer(("127.0.0.1", 0), _CaptureHandler)
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()

    def test_post_after_login_keeps_content_type(self):
        client = VoltrusClient(f"http://127.0.0.1:{self.port}")
        client.login("user", "pass")  # captures voltrus_token cookie

        client.request("POST", "/api/v1/anything", body={"hello": "world"})

        headers = _CaptureHandler.seen[-1]
        self.assertEqual(headers.get("content-type"), "application/json")
        self.assertIn("voltrus_token=t-123", headers.get("cookie", ""))

    def test_get_after_login_keeps_cookie(self):
        client = VoltrusClient(f"http://127.0.0.1:{self.port}")
        client.login("user", "pass")

        client.request("GET", "/api/v1/auth/me")

        headers = _CaptureHandler.seen[-1]
        self.assertIn("voltrus_token=t-123", headers.get("cookie", ""))


if __name__ == "__main__":
    unittest.main()
