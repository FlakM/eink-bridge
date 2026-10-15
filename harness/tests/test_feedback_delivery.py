"""Contract tests for portable watcher and Claude MCP-channel delivery."""
import http.server
import json
import os
from pathlib import Path
import queue
import socket
import subprocess
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DeliveryTests(unittest.TestCase):
    def test_portable_watcher_ignores_replay_and_returns_one_compact_event(self):
        with tempfile.TemporaryDirectory() as directory:
            fake = Path(directory) / "eink-review"
            events = [
                {"type": "annotation_result", "feedback_id": "old", "annotations": []},
                {"type": "annotation_result", "feedback_id": "new", "annotations": [
                    {"recognized_text": "larger", "strokes": [[1]], "pressures": [[1]]}]},
            ]
            fake.write_text("#!/usr/bin/env python3\n" + "\n".join(
                f"print({'EVENT:ANNOTATION_RESULT ' + json.dumps(e)!r}, flush=True)" for e in events))
            fake.chmod(0o755)
            result = subprocess.run(["python3", str(ROOT / "harness/skills/eink/scripts/watch-feedback.py"),
                                     "session", "--after-feedback", "old", "--eink-review", str(fake),
                                     "--ready-file", str(Path(directory)/"ready.json")],
                                    env={**os.environ, "PATH": directory + ":" + os.environ["PATH"]},
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(result.stdout.splitlines()), 1)
            event = json.loads(result.stdout.partition(" ")[2])
            self.assertEqual(event["feedback_id"], "new")
            self.assertNotIn("strokes", event["annotations"][0])
            self.assertNotIn("pressures", event["annotations"][0])
            self.assertNotIn("TIMEOUT", result.stdout)
            self.assertEqual(json.loads((Path(directory)/"ready.json").read_text())["state"], "event")

    def test_ready_receipt_requires_a_live_handshake_and_observer_cleans_up(self):
        with tempfile.TemporaryDirectory() as directory:
            fake = Path(directory)/"eink-review"
            fake.write_text("#!/usr/bin/env python3\nimport sys,time\nprint('EVENT:WATCH_READY session_id=session',file=sys.stderr,flush=True)\ntime.sleep(30)\n")
            fake.chmod(0o755)
            ready = Path(directory)/"ready.json"
            script = str(ROOT/"harness/skills/eink/scripts/watch-feedback.py")
            observer = subprocess.Popen(["python3",script,"session","--eink-review",str(fake),"--ready-file",str(ready)],
                env={**os.environ,"PATH":directory+":"+os.environ["PATH"]},stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            try:
                checked = subprocess.run(["python3",script,"--wait-ready",str(ready)],capture_output=True,text=True,timeout=8)
                self.assertEqual(checked.returncode,0,checked.stderr)
                self.assertEqual(json.loads(checked.stdout)["state"],"ready")
                self.assertIsNone(observer.poll())
                observer.terminate()
                observer.wait(timeout=5)
                self.assertEqual(json.loads(ready.read_text())["state"],"ended")
                checked = subprocess.run(["python3",script,"--wait-ready",str(ready)],capture_output=True,text=True,timeout=2)
                self.assertNotEqual(checked.returncode,0,"Ended observers must not be announced as connected")
            finally:
                if observer.poll() is None:
                    observer.terminate(); observer.wait(timeout=5)
                observer.stdout.close(); observer.stderr.close()

    def test_explicit_binary_beats_a_stale_path_shadow(self):
        """A long-lived shell keeps the PATH it started with, so a freshly deployed
        CLI can be shadowed by a stale store path. An explicit override must win, and
        the watcher must report which CLI it used."""
        with tempfile.TemporaryDirectory() as directory:
            stale = Path(directory) / "stale"
            stale.mkdir()
            (stale / "eink-review").write_text("#!/usr/bin/env python3\nprint('WRONG CLI')\n")
            (stale / "eink-review").chmod(0o755)
            real = Path(directory) / "real-eink-review"
            real.write_text("#!/usr/bin/env python3\nprint('EVENT:WATCH_ENDED session_id=session exit=1', flush=True)\n")
            real.chmod(0o755)
            result = subprocess.run(["python3", str(ROOT / "harness/skills/eink/scripts/watch-feedback.py"),
                                     "session", "--eink-review", str(real)],
                                    env={**os.environ, "PATH": str(stale) + ":" + os.environ["PATH"]},
                                    capture_output=True, text=True, timeout=10)
            self.assertNotIn("WRONG CLI", result.stdout)
            self.assertIn(f"cli={real}", result.stdout)

    def test_stale_ready_file_cannot_be_reused_as_proof_of_connection(self):
        with tempfile.TemporaryDirectory() as directory:
            ready=Path(directory)/"ready.json"
            ready.write_text('{"state":"ready","session_id":"old"}')
            result=subprocess.run(["python3",str(ROOT/"harness/skills/eink/scripts/watch-feedback.py"),
                "new","--ready-file",str(ready)],capture_output=True,text=True,timeout=2)
            self.assertNotEqual(result.returncode,0)
            self.assertIn("fresh ready file",result.stderr)

    def test_claude_subscription_recovers_feedback_sent_before_subscription(self):
        pending = {"type": "annotation_result", "feedback_id": "recovered", "version": 1,
                   "annotations": [{"recognized_text":"say more","anchor":None,"strokes":[[[1,2],[3,4]]],"pressures":[[.5,.8]]}]}
        updates = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(json.dumps({"pending": pending}).encode())

            def log_message(self, *args):
                pass

            def do_PUT(self):
                updates.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{"version":2}')

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        process = subprocess.Popen(["node", "--experimental-strip-types", "channel.ts"],
            cwd=ROOT / "harness/channels/eink-channel", stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            env={**os.environ, "EINK_CHANNEL_PORT": str(port),
                 "EINK_SERVER_URL": f"http://127.0.0.1:{server.server_port}"})
        lines = queue.Queue()
        threading.Thread(target=lambda: [lines.put(line) for line in process.stdout], daemon=True).start()

        def send(payload):
            process.stdin.write(json.dumps({"jsonrpc": "2.0", **payload}) + "\n")
            process.stdin.flush()

        try:
            send({"id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05",
                 "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}})
            self.assertEqual(json.loads(lines.get(timeout=10))["id"], 1)
            send({"method": "notifications/initialized"})
            send({"id": 2, "method": "tools/call", "params": {"name": "subscribe_session",
                 "arguments": {"session_id": "session"}}})
            messages = [json.loads(lines.get(timeout=10)), json.loads(lines.get(timeout=10))]
            notification = next(m for m in messages if m.get("method") == "notifications/claude/channel")
            event=json.loads(notification["params"]["content"])
            self.assertEqual(event["feedback_id"], "recovered")
            self.assertEqual(event["annotations"][0]["index"],0)
            self.assertEqual(event["annotations"][0]["stroke_count"],1)
            self.assertNotIn("strokes",event["annotations"][0])
            self.assertNotIn("pressures",event["annotations"][0])
            self.assertTrue(any(m.get("id") == 2 for m in messages))
            send({"id": 3, "method": "tools/call", "params": {"name": "update_session", "arguments": {
                "session_id": "session", "content": "# Fixed", "expected_version": 1,
                "feedback_id": "recovered", "consumed_annotations": [0]}}})
            result = json.loads(lines.get(timeout=10))
            self.assertEqual(result["id"], 3)
            self.assertFalse(result["result"].get("isError", False))
            self.assertEqual(updates, [{"content": "# Fixed", "expected_version": 1,
                                       "feedback_id": "recovered", "consumed_annotations": [0]}])
        finally:
            process.terminate()
            process.wait(timeout=5)
            process.stdin.close(); process.stdout.close(); process.stderr.close()
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    unittest.main()
