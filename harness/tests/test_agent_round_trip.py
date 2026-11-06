"""The agent's half of a review turn, through the channel's MCP tools, against a real server.

`test_feedback_delivery` pins the channel against a stub; this one runs the real
`eink-serve`, so a request shape the tool sends and the route refuses (a renamed field, an
index the server reads differently) fails here instead of in front of the reader.
"""
import json
import os
import queue
import socket
import subprocess
import tempfile
import threading
import time
import unittest
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SERVE = ROOT / "server" / "target" / "debug" / "eink-serve"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def call(method, url, body=None):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(url, data=data, method=method,
                                     headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=10) as response:
        raw = response.read()
        return json.loads(raw) if raw else None


class AgentRoundTripTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Always build: a stale binary would test yesterday's server and pass.
        subprocess.run(["cargo", "build", "--bin", "eink-serve"], cwd=ROOT / "server",
                       check=True, capture_output=True)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        tmp = Path(self.tmp.name)
        port = free_port()
        (tmp / "config/eink-bridge").mkdir(parents=True)
        (tmp / "config/eink-bridge/config.toml").write_text(
            f'[server]\nhost = "127.0.0.1"\nport = {port}\nstate_dir = "{tmp}/state"\n')
        env = {k: v for k, v in os.environ.items() if not k.startswith("EINK_")}
        self.server = subprocess.Popen(
            [str(SERVE)], env={**env, "XDG_CONFIG_HOME": str(tmp / "config"),
                               "XDG_DATA_HOME": str(tmp / "data"), "RUST_LOG": "warn"},
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.url = f"http://127.0.0.1:{port}"
        for _ in range(100):
            try:
                call("GET", f"{self.url}/api/health")
                return
            except OSError:
                time.sleep(0.1)
        self.fail("eink-serve never answered")

    def tearDown(self):
        self.server.terminate()
        self.server.wait(timeout=10)
        self.tmp.cleanup()

    def test_an_agent_answers_a_spoken_note_through_the_channel(self):
        session = call("POST", f"{self.url}/api/sessions",
                       {"title": "Turn", "content": "# Turn\n\nFirst.\n\nSecond.\n"})["id"]
        note = {"anchor": {"type": "explicit",
                           "elements": [{"section_id": None, "tag": "P", "text": "Second."}]},
                "strokes": [], "recognized_text": "needs an example", "source": "voice",
                "voice": {"raw": "needs an example"}}
        call("POST", f"{self.url}/api/sessions/{session}/request_update",
             {"annotations": [note], "document_version": 1})

        channel = subprocess.Popen(
            ["node", "--experimental-strip-types", "channel.ts"],
            cwd=ROOT / "harness/channels/eink-channel", stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
            env={**os.environ, "EINK_CHANNEL_PORT": str(free_port()), "EINK_SERVER_URL": self.url})
        lines = queue.Queue()
        threading.Thread(target=lambda: [lines.put(json.loads(l)) for l in channel.stdout],
                         daemon=True).start()

        def send(payload):
            channel.stdin.write(json.dumps({"jsonrpc": "2.0", **payload}) + "\n")
            channel.stdin.flush()

        def answer(request_id):
            while True:
                message = lines.get(timeout=15)
                if message.get("id") == request_id:
                    self.assertFalse(message["result"].get("isError", False), message)
                    return message

        try:
            send({"id": 1, "method": "initialize", "params": {
                "protocolVersion": "2024-11-05", "capabilities": {},
                "clientInfo": {"name": "test", "version": "1"}}})
            answer(1)
            send({"method": "notifications/initialized"})
            send({"id": 2, "method": "tools/call", "params": {
                "name": "subscribe_session", "arguments": {"session_id": session}}})
            event = None
            while event is None:
                message = lines.get(timeout=15)
                if message.get("method") == "notifications/claude/channel":
                    event = json.loads(message["params"]["content"])
            self.assertEqual(event["annotations"][0]["recognized_text"], "needs an example")

            send({"id": 3, "method": "tools/call", "params": {"name": "report_progress", "arguments": {
                "session_id": session, "feedback_id": event["feedback_id"],
                "message": "Writing the example", "step": 1, "of": 2}}})
            answer(3)
            send({"id": 4, "method": "tools/call", "params": {"name": "update_session", "arguments": {
                "session_id": session, "content": "# Turn\n\nFirst.\n\nSecond, for example.\n",
                "feedback_id": event["feedback_id"], "expected_version": event["version"],
                "consumed_annotations": [event["annotations"][0]["index"]]}}})
            answer(4)
        finally:
            channel.terminate()
            channel.wait(timeout=5)
            channel.stdin.close()
            channel.stdout.close()

        state = call("GET", f"{self.url}/api/sessions/{session}/feedback")
        self.assertIsNone(state["pending"], "the request is still open after the answer")
        self.assertEqual(state["version"], 2)
        self.assertEqual(state["last_content_update"]["consumed_notes"],
                         [{"text": "needs an example", "anchor": ["Second."]}])


if __name__ == "__main__":
    unittest.main()
