#!/usr/bin/env python3
"""One review event per background job, for harnesses without Claude channels."""
import argparse
import json
import os
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("session_id", nargs="?")
parser.add_argument("--server", default=os.environ.get("EINK_SERVER_URL", "http://localhost:3333"))
parser.add_argument("--after-feedback", default="")
parser.add_argument("--timeout", type=int, default=30, help="minutes, like eink-review watch")
parser.add_argument("--ready-file", help="unique file to receive the live WebSocket handshake receipt")
parser.add_argument("--wait-ready", metavar="FILE", help="wait up to five seconds for a handshake receipt, then exit")
args = parser.parse_args()
if args.wait_ready:
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        try:
            receipt = json.loads(Path(args.wait_ready).read_text())
            print(json.dumps(receipt), flush=True)
            sys.exit(0 if receipt["state"] in ("ready", "event") else 1)
        except (FileNotFoundError, json.JSONDecodeError):
            time.sleep(0.01)
    sys.exit("Observer handshake not confirmed; do not announce that feedback is connected.")
if not args.session_id:
    parser.error("session_id is required unless --wait-ready is used")
if args.ready_file and Path(args.ready_file).exists() and Path(args.ready_file).read_text().strip():
    sys.exit("Choose a fresh ready file for this observer; existing receipts are not proof of a live connection.")

receipt_lock = threading.Lock()
receipt_state = "starting"
def receipt(state):
    global receipt_state
    with receipt_lock:
        if receipt_state in ("event", "ended"):
            return
        receipt_state = state
        if args.ready_file:
            path = Path(args.ready_file)
            temporary = path.with_name(path.name + ".tmp")
            temporary.write_text(json.dumps({"state": state, "session_id": args.session_id, "pid": os.getpid()}))
            temporary.replace(path)

process = subprocess.Popen(
    ["eink-review", "--server", args.server, "watch", args.session_id, "--timeout", str(args.timeout)],
    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
)

def forward_status():
    for line in process.stderr:
        if line.startswith("EVENT:WATCH_READY"):
            receipt("ready")
        print(line, end="", file=sys.stderr, flush=True)

status_thread = threading.Thread(target=forward_status, daemon=True)
status_thread.start()
signal.signal(signal.SIGTERM, lambda signum, frame: sys.exit(128 + signum))
try:
    for line in process.stdout:
        marker, _, raw = line.partition(" ")
        if marker not in ("EVENT:ANNOTATION_RESULT", "EVENT:SUBMITTED"):
            continue
        payload = json.loads(raw)
        if marker == "EVENT:ANNOTATION_RESULT" and payload.get("feedback_id") == args.after_feedback:
            continue
        payload = payload.get("result", payload)
        # No coordinates/pressure arrays or image blobs in the agent's context.
        for index, annotation in enumerate(payload.get("annotations", [])):
            annotation["index"] = index
            annotation.setdefault("stroke_count", len(annotation.get("strokes", [])))
            annotation.pop("strokes", None)
            annotation.pop("pressures", None)
        if "annotation_images" in payload:
            payload["annotation_image_count"] = len(payload.pop("annotation_images"))
        payload.pop("stroke_data", None)
        receipt("event")
        print(marker + " " + json.dumps({"session_id": args.session_id, **payload}), flush=True)
        break
    else:
        code = process.wait()
        print(f"EVENT:WATCH_ENDED session_id={args.session_id} exit={code}", flush=True)
        receipt("ended")
        sys.exit(code or 1)
finally:
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    status_thread.join(timeout=1)
    if receipt_state == "ready":
        receipt("ended")
