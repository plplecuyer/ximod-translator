#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Stand-in for the online services (lot 5 recette): DeepL, Azure, OpenAI
and Google endpoints on one local port, answering « [XX] text » with the
protection tokens kept. Usage: python3 tools/mock_online.py 8765 [delay]
(delay: seconds before each translation answer, to try « Arrêter »)."""
import json, re, sys, time
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
DELAY = float(sys.argv[2]) if len(sys.argv) > 2 else 0.0

def fake(text, dst):
    # Keep <a_A> … tokens; pseudo-translate the words.
    parts = re.split(r"(<[a-z]+_[A-Z]+>)", text)
    out = []
    for p in parts:
        if re.fullmatch(r"<[a-z]+_[A-Z]+>", p or ""):
            out.append(p)
        else:
            out.append(p.replace("the ", "le ").replace("The ", "Le ").replace("sword", "épée").replace("gold", "or").replace("Hello", "Bonjour").replace("you", "vous"))
    return "[%s] " % dst.upper() + "".join(out)

class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass
    def _send(self, obj, code=200):
        body = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        if self.path.startswith("/v2/usage"):
            return self._send({"character_count": 18768, "character_limit": 500000})
        if self.path.startswith("/v1/models"):
            return self._send({"data": [{"id": "gpt-4o-mini"}]})
        self._send({"error": "not found"}, 404)
    def do_POST(self):
        if DELAY > 0:
            time.sleep(DELAY)
        n = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(n).decode("utf-8") if n else "{}"
        try:
            body = json.loads(raw)
        except Exception:
            body = {}
        p = self.path
        if p.startswith("/v2/translate"):
            texts = body.get("text", [])
            dst = body.get("target_lang", "fr")
            return self._send({"translations": [{"detected_source_language": "EN", "text": fake(t, dst)} for t in texts]})
        if p.startswith("/translate"):
            dst = "fr"
            m = re.search(r"[?&]to=([a-z-]+)", p)
            if m:
                dst = m.group(1)
            return self._send([{"translations": [{"text": fake(item.get("Text", ""), dst), "to": dst}]} for item in body])
        if p.startswith("/v1/chat/completions"):
            msgs = body.get("messages", [])
            user = msgs[-1]["content"] if msgs else ""
            try:
                items = json.loads(user)
            except Exception:
                items = []
            answer = json.dumps([fake(t, "fr") for t in items], ensure_ascii=False) if isinstance(items, list) else "[]"
            return self._send({"choices": [{"message": {"role": "assistant", "content": answer}}]})
        if p.startswith("/language/translate/v2"):
            q = body.get("q", [])
            if isinstance(q, str):
                q = [q]
            dst = body.get("target", "fr")
            return self._send({"data": {"translations": [{"translatedText": fake(t, dst)} for t in q]}})
        self._send({"error": "not found"}, 404)

print("mock online services on http://127.0.0.1:%d" % PORT, flush=True)
HTTPServer(("127.0.0.1", PORT), H).serve_forever()
