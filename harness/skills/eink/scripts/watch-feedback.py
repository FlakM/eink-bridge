#!/usr/bin/env python3
"""One review event per background job, for harnesses without Claude channels."""
import argparse
import json
import os
import subprocess
import sys

parser = argparse.ArgumentParser()
parser.add_argument("session_id")
parser.add_argument("--server", default=os.environ.get("EINK_SERVER_URL", "http://localhost:3333"))
parser.add_argument("--after-feedback", default="")
parser.add_argument("--timeout", type=int, default=30, help="minutes, like eink-review watch")
args = parser.parse_args()
process = subprocess.Popen(
    ["eink-review", "--server", args.server, "watch", args.session_id, "--timeout", str(args.timeout)],
    stdout=subprocess.PIPE, text=True,
)
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
        for annotation in payload.get("annotations", []):
            annotation.pop("strokes", None)
            annotation.pop("pressures", None)
        payload.pop("annotation_images", None)
        print(marker + " " + json.dumps({"session_id": args.session_id, **payload}), flush=True)
        break
    else:
        code = process.wait()
        print(f"EVENT:WATCH_ENDED session_id={args.session_id} exit={code}", flush=True)
        sys.exit(code or 1)
finally:
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
