#!/usr/bin/env python3
"""Loopback mock serving the four providers' fixture responses (dev/e2e only).

Usage: scripts/dev-mock-server.py PORT
Prints the port when ready (or uses the given one).
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

RESPONSES = {
    "/claude": {
        "usage": [
            {"key": "five_hour", "used_pct": 62.5, "resets_in_sec": 7200},
            {"key": "seven_day", "used_percent": 31.0},
        ],
        "plan": {"type": "pro"},
    },
    "/codex": {
        "plan_type": "plus",
        "rate_limit": {
            "primary_window": {"used_percent": 44.0, "window_minutes": 300, "resets_at": 1759000000},
            "secondary_window": {"used_percent": 12.0, "window_minutes": 10080},
        },
    },
    "/zai": {
        "data": {
            "limits": [
                {"type": "TOKENS_LIMIT", "unit": 3, "percentage": 42.0, "nextResetTime": 1758900000000},
                {"type": "TOKENS_LIMIT", "unit": 6, "percentage": 10.0},
            ]
        }
    },
    "/opencode": {"usage": {"rolling": {"percent": 55, "resetInSec": 3600}, "weekly": {"percent": 12}}},
}


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps(RESPONSES.get(self.path, {})).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 0
    server = HTTPServer(("127.0.0.1", port), Handler)
    print(server.server_address[1], flush=True)
    server.serve_forever()
