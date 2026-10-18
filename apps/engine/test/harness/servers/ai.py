"""
An in-process fake of the Ollama endpoints ms-ai calls (crates/ms-ai/src/wire/ollama.rs), for
the AI triage tests (test_ai.py):

  GET  /api/tags    the model list
  POST /api/chat    NDJSON: `{model, message: {content}, done: false}` per chunk, then
                    `{done: true, prompt_eval_count, eval_count}`

`/api/chat` answers from `script`, one entry per request in order, then `answer` for every
request after it. An entry is `{"text": ...}` (streamed in a few chunks) or
`{"status": 429, "retry_after": 1}` (an error status, with `Retry-After` when given). Every
chat request lands in `requests` as its parsed JSON body.
"""
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Optional

MODEL = "harness-triage"
TRIAGE_ANSWER = {"category": "needs-reply", "needsReply": True, "reason": "The sender asks a question."}


class FakeOllama:
    def __init__(self, answer: Optional[str] = None, script: Optional[list] = None):
        self.answer = answer if answer is not None else json.dumps(TRIAGE_ANSWER)
        self.script = list(script or [])
        self.requests: list = []
        self.lock = threading.Lock()
        self.httpd: Optional[ThreadingHTTPServer] = None

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.httpd.server_address[1]}"

    def start(self) -> "FakeOllama":
        fake = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *args):
                pass

            def _send(self, status: int, body: bytes, content_type: str, headers: Optional[dict] = None):
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                for k, v in (headers or {}).items():
                    self.send_header(k, v)
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                if self.path.rstrip("/") == "/api/tags":
                    body = json.dumps({"models": [{"name": MODEL, "model": MODEL}]}).encode()
                    self._send(200, body, "application/json")
                else:
                    self._send(404, b'{"error": "not found"}', "application/json")

            def do_POST(self):
                length = int(self.headers.get("Content-Length") or 0)
                raw = self.rfile.read(length)
                if self.path.rstrip("/") != "/api/chat":
                    self._send(404, b'{"error": "not found"}', "application/json")
                    return
                try:
                    body = json.loads(raw or b"{}")
                except ValueError:
                    body = {"unparsed": raw.decode(errors="replace")}
                with fake.lock:
                    fake.requests.append(body)
                    entry = fake.script.pop(0) if fake.script else {"text": fake.answer}
                if "status" in entry:
                    headers = {"Retry-After": str(entry["retry_after"])} if "retry_after" in entry else {}
                    err = json.dumps({"error": entry.get("error", "rate limited")}).encode()
                    self._send(int(entry["status"]), err, "application/json", headers)
                    return
                text = entry["text"]
                step = max(1, len(text) // 3)
                lines = [{"model": MODEL, "message": {"role": "assistant", "content": text[i:i + step]}, "done": False}
                         for i in range(0, len(text), step)]
                lines.append({"model": MODEL, "done": True, "prompt_eval_count": 120, "eval_count": 30})
                self._send(200, "".join(json.dumps(l) + "\n" for l in lines).encode(), "application/x-ndjson")

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.httpd.daemon_threads = True
        threading.Thread(target=self.httpd.serve_forever, daemon=True, name="fake-ollama").start()
        return self

    def stop(self):
        if self.httpd:
            self.httpd.shutdown()
            self.httpd.server_close()
            self.httpd = None

    def chat_count(self) -> int:
        with self.lock:
            return len(self.requests)

    def provider(self, provider_id: str = "harness") -> dict:
        """The `core.ai.providers` entry for this fake."""
        return {"id": provider_id, "kind": "ollama", "name": "Harness", "url": self.url, "model": MODEL}
