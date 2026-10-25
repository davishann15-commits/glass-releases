#!/usr/bin/env python3
"""A scripted stand-in for OpenAI's API, for the assistant's UI tests in the
simulator (the app's debug-only `-assistantMock http://127.0.0.1:PORT/v1`).

It answers GET /v1/models and streams POST /v1/responses like the real API
(server-sent events), playing one conversation: search the local library, propose
a playlist from what came back, ask to save it, then reply in words. Each request
body is appended to LOG (one JSON object per line) so a test can check what Glass
sent — that no file paths or credentials left the app, for one.

    python3 -I ios/scripts/mock-openai.py PORT LOG
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
LOG = sys.argv[2] if len(sys.argv) > 2 else "mock-openai.log"


def events_for(body):
    """What the scripted model says next, from what the request carries."""
    items = body.get("input", [])
    # The outputs of this turn's tool calls (after the last user message).
    last_user = max((i for i, it in enumerate(items) if it.get("role") == "user"), default=-1)
    outputs = [it for it in items[last_user + 1:] if it.get("type") == "function_call_output"]
    calls = {it.get("call_id"): it.get("name") for it in items[last_user + 1:] if it.get("type") == "function_call"}
    if not outputs:
        return call("c1", "music.search_library", {"query": "", "artist": None, "genre": "city pop", "year_from": None, "year_to": None, "limit": 3})
    last = outputs[-1]
    name = calls.get(last.get("call_id"), "")
    out = json.loads(last.get("output") or "{}")
    if name.endswith("search_library"):
        ids = [s["id"] for s in out.get("songs", [])][:3]
        return call("c2", "music.propose_playlist", {"name": "Night Drive", "description": "City pop for late roads", "track_ids": ids, "replaces_playlist_id": None})
    if name.endswith("propose_playlist"):
        return call("c3", "music.request_save", {"draft_id": out.get("draft_id") or out.get("id") or "d1"})
    return text("Here's “Night Drive”: three city pop songs. Save it when you're ready.")


def call(call_id, name, args):
    item = {"type": "function_call", "id": "fc_" + call_id, "call_id": call_id, "name": name, "arguments": json.dumps(args), "status": "completed"}
    return [{"type": "response.output_item.done", "item": item}, completed([item])]


def text(t):
    evs = [{"type": "response.output_text.delta", "delta": t[i:i + 12]} for i in range(0, len(t), 12)]
    item = {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": t}]}
    return evs + [completed([item])]


def completed(output):
    return {"type": "response.completed", "response": {"output": output, "usage": {"input_tokens": 1200, "output_tokens": 80}}}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        # The sign-in page (the mock's authorize endpoint is its base URL): consent
        # at once and send the browser back to Glass's loopback with a code.
        if "?" in self.path and "redirect_uri=" in self.path:
            from urllib.parse import parse_qs, urlencode, urlparse
            q = parse_qs(urlparse(self.path).query)
            back = q["redirect_uri"][0] + "?" + urlencode({"code": "mock-code", "state": q.get("state", [""])[0], "client_id": "mock-client"})
            with open(LOG, "a") as f:
                f.write(json.dumps({"path": "authorize", "query": {k: v[0] for k, v in q.items() if k in ("client_id", "scope", "response_type", "code_challenge_method", "redirect_uri")}}) + "\n")
            self.send_response(302)
            self.send_header("location", back)
            self.end_headers()
            return
        if self.path.endswith("/models"):
            body = json.dumps({"data": [{"id": "gpt-6-luna"}, {"id": "gpt-5-mini"}, {"id": "whisper-1"}]}).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        n = int(self.headers.get("content-length", "0"))
        raw = self.rfile.read(n) or b"{}"
        if "json" in self.headers.get("content-type", ""):
            body = json.loads(raw)
        else:
            # The sign-in's token request is a form; only its field names are logged.
            from urllib.parse import parse_qs
            body = {"form_fields": sorted(parse_qs(raw.decode()).keys())}
        with open(LOG, "a") as f:
            f.write(json.dumps({"path": self.path, "auth": self.headers.get("authorization", "")[:12], "body": body}) + "\n")
        if not self.path.endswith("/responses"):
            # The token endpoint: refuse the code, so sign-in ends with OpenAI's error.
            body = json.dumps({"error": "invalid_grant", "error_description": "mock: codes from the test server aren't real"}).encode()
            self.send_response(400)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        for ev in events_for(body):
            self.wfile.write(("data: " + json.dumps(ev) + "\n\n").encode())
            self.wfile.flush()


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
