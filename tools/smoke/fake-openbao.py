"""A small fake OpenBao (KV v2 + token lookup/renew) for the guardrails two-PC trial.

Modelled on crates/dots-core/tests/common/fake_openbao.rs. KV lives in a JSON file,
kv.json, read on every request, so it can be edited between steps.

  python -I tools/smoke/fake-openbao.py <port> <kv.json> <token> [<token> ...]

A KV write is POST /v1/<mount>/data/<path> with {"data": {...}} (as `bao kv put` sends).
"""
import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote, urlparse

PORT = int(sys.argv[1])
KV_FILE = sys.argv[2]
TOKENS = set(sys.argv[3:])
LOCK = threading.Lock()


def load():
    try:
        with open(KV_FILE, encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        return {}


def save(kv):
    with open(KV_FILE, "w", encoding="utf-8") as f:
        json.dump(kv, f, indent=1)


def envelope(data, auth=None):
    return {"request_id": "r", "lease_id": "", "renewable": False, "lease_duration": 0,
            "data": data, "wrap_info": None, "warnings": None, "auth": auth}


class H(BaseHTTPRequestHandler):
    def send(self, status, body):
        raw = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def log_message(self, fmt, *args):
        sys.stderr.write("%s %s\n" % (self.command, self.path))

    def handle_any(self):
        path = unquote(urlparse(self.path).path)
        length = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(length) if length else b""
        if not path.startswith("/v1/"):
            return self.send(404, {"errors": []})
        rest = path[4:]
        token = self.headers.get("x-vault-token")
        if token not in TOKENS:
            return self.send(403, {"errors": ["permission denied"]})
        if rest == "auth/token/lookup-self" and self.command == "GET":
            return self.send(200, envelope({"renewable": False, "ttl": 0, "policies": ["default"]}))
        if rest == "auth/token/renew-self":
            return self.send(400, {"errors": ["lease is not renewable"]})
        with LOCK:
            kv = load()
            if self.command in ("POST", "PUT") and "/data/" in rest:
                try:
                    w = json.loads(body)
                    data = w["data"]
                    assert isinstance(data, dict)
                except Exception:
                    return self.send(400, {"errors": ["no data provided"]})
                kv[rest] = data
                save(kv)
                return self.send(200, envelope({"created_time": "2026-11-11T00:00:00Z",
                                                "custom_metadata": None, "deletion_time": "",
                                                "destroyed": False, "version": 2}))
            if self.command == "GET":
                if rest in kv:
                    return self.send(200, envelope({"data": kv[rest], "metadata": {
                        "created_time": "2026-11-11T00:00:00Z", "custom_metadata": None,
                        "deletion_time": "", "destroyed": False, "version": 1}}))
                return self.send(404, {"errors": []})
        return self.send(405, {"errors": []})

    do_GET = handle_any
    do_POST = handle_any
    do_PUT = handle_any


ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
