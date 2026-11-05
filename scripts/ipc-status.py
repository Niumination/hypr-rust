#!/usr/bin/env python3
"""Read a Hypr-Rust v1 status snapshot from the local macOS Unix socket."""

import json
import os
import socket
import sys

SOCKET_PATH = os.path.join(os.path.expanduser("~"), ".hypr-rust", "ipc.sock")
REQUEST_ID = "python-status-1"
MAX_RESPONSE_BYTES = 4096


def main() -> int:
    request = {
        "version": 1,
        "id": REQUEST_ID,
        "method": "status.get",
        "params": {},
    }
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.settimeout(3.0)
            client.connect(SOCKET_PATH)
            client.sendall((json.dumps(request, separators=(",", ":")) + "\n").encode("ascii"))
            response = bytearray()
            while len(response) <= MAX_RESPONSE_BYTES:
                chunk = client.recv(min(1024, MAX_RESPONSE_BYTES + 1 - len(response)))
                if not chunk:
                    break
                response.extend(chunk)
                if b"\n" in response:
                    break
    except OSError as error:
        print(f"IPC unavailable: {error}", file=sys.stderr)
        return 1

    if len(response) > MAX_RESPONSE_BYTES or not response.endswith(b"\n"):
        print("invalid or oversized IPC response", file=sys.stderr)
        return 1
    try:
        message = json.loads(response.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        print(f"invalid IPC JSON response: {error}", file=sys.stderr)
        return 1
    if not isinstance(message, dict):
        print("IPC response must be a JSON object", file=sys.stderr)
        return 1
    if message.get("version") != 1 or message.get("id") != REQUEST_ID:
        print("IPC version or request ID mismatch", file=sys.stderr)
        return 1
    if message.get("ok") is not True:
        print(json.dumps(message.get("error", {}), indent=2), file=sys.stderr)
        return 1
    print(json.dumps(message.get("result", {}), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
