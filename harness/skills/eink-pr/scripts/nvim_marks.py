#!/usr/bin/env python3
"""Set Neovim global named marks (A-Z) from outside a running Neovim.

The skill opens the PR in Neovim, then marks the most important locations so the
reviewer can jump to them with `'A`, `'B`, ... Each mark is set on its target
buffer, so the current window and an open diff view stay as they are.

Two traps this script defends against, both verified against real Neovim:

1. `nvim --server ...` exits 0 even when it failed to connect (E247). So success is
   decided by reading the marks back, never by the exit code.
2. `--remote-send` returns before Neovim runs the command. Removing the Lua file
   immediately can race the execution, so the script keeps it until the marks are
   confirmed and retries while Neovim catches up.

Marks also persist in shada, so a silent miss would read back as a stale mark from an
earlier session. Verification is therefore authoritative, not a nicety.

Usage:
  nvim_marks.py --socket /tmp/x.sock --marks "A:src/app.rs:42,B:src/lib.rs:7"
  nvim_marks.py --socket /tmp/x.sock --verify A,B
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument("--socket", required=True, help="Neovim --listen socket path")
parser.add_argument("--marks", default="", help="comma list of LETTER:path:line")
parser.add_argument("--verify", default="", help="comma list of letters to read back")
parser.add_argument("--jump", metavar="LETTER", help="move the cursor to that global mark")
parser.add_argument("--open", metavar="FILE:LINE", help="open a file at a line in the current window")
args = parser.parse_args()

if not os.path.exists(args.socket):
    sys.exit(f"Neovim socket not found: {args.socket}")


def nvim(*extra):
    return subprocess.run(["nvim", "--server", args.socket, *extra],
                          capture_output=True, text=True, timeout=20)


def file_line_count(path):
    with open(path, "rb") as handle:
        return handle.read().count(b"\n") + 1


def check(stdout, stderr, what):
    if "Failed to connect" in stderr or "E247" in stderr:
        sys.exit(f"cannot reach Neovim at {args.socket}: {stderr.strip()}")


def lua_string(value):
    """Emit a Lua string literal. JSON is not Lua: `[{...}]` is a syntax error."""
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def parse(spec):
    letter, path, line = spec.split(":", 2)
    if len(letter) != 1 or not letter.isalpha():
        sys.exit(f"mark letter must be a single letter: {letter!r}")
    if int(line) < 1:
        sys.exit(f"line must be >= 1 for mark {letter}: {line}")
    resolved = os.path.realpath(os.path.abspath(path))
    if not os.path.isfile(resolved):
        sys.exit(f"mark {letter}: no such file: {resolved}")
    # Validate the range here rather than letting Neovim raise an error prompt, which
    # blocks the editor and hangs every later remote request.
    with open(resolved, "rb") as handle:
        lines = handle.read().count(b"\n") + 1
    if int(line) > lines:
        sys.exit(f"mark {letter}: {resolved} has {lines} lines, cannot mark line {line}")
    return {"letter": letter, "file": resolved, "line": int(line)}


def position(letter):
    out = nvim("--remote-expr", f"string(getpos(\"'{letter}\"))")
    numbers = [int(n) for n in re.findall(r"-?\d+", out.stdout)]
    if len(numbers) < 2 or not numbers[0]:
        return None, None
    return numbers[0], numbers[1]


def buffer_path(buffer_number):
    out = nvim("--remote-expr", f"fnamemodify(bufname({buffer_number}), ':p')")
    return os.path.realpath(out.stdout.strip())


def landed(item):
    buffer_number, line = position(item["letter"])
    return bool(buffer_number) and line == item["line"] and buffer_path(buffer_number) == item["file"]


if args.marks:
    items = [parse(spec) for spec in filter(None, (s.strip() for s in args.marks.split(",")))]
    script = ["local items = {"]
    script += [
        f"  {{ letter = {lua_string(i['letter'])}, file = {lua_string(i['file'])}, line = {i['line']} }},"
        for i in items
    ]
    script += [
        "}",
        "for _, it in ipairs(items) do",
        "  pcall(function()",
        "    local buf = vim.fn.bufadd(it.file)",
        "    vim.fn.bufload(buf)",
        "    vim.api.nvim_buf_set_mark(buf, it.letter, it.line, 1, {})",
        "  end)",
        "end",
    ]
    handle = tempfile.NamedTemporaryFile("w", suffix=".lua", delete=False)
    try:
        handle.write("\n".join(script) + "\n")
        handle.close()
        sent = nvim("--remote-send", f"<Cmd>luafile {handle.name}<CR>")
        check(sent.stdout, sent.stderr, "send marks")
        # --remote-send returns before Neovim executes it; give it a moment and retry.
        pending = list(items)
        for _ in range(40):
            pending = [i for i in pending if not landed(i)]
            if not pending:
                break
            time.sleep(0.05)
        if pending:
            detail = []
            for i in pending:
                buffer_number, line = position(i["letter"])
                where = buffer_path(buffer_number) if buffer_number else "unset"
                detail.append(f"{i['letter']}: expected {i['file']}:{i['line']}, got {where}:{line}")
            sys.exit("marks not set: " + "; ".join(detail))
        print(f"set {len(items)} mark(s): " + ", ".join(i["letter"] for i in items))
    finally:
        os.unlink(handle.name)

if args.verify:
    for letter in filter(None, (s.strip() for s in args.verify.split(","))):
        buffer_number, line = position(letter)
        print(f"{letter}=unset" if not buffer_number else f"{letter}={buffer_path(buffer_number)}:{line}")

if args.jump:
    letter = args.jump.strip()
    if len(letter) != 1 or not letter.isalpha():
        sys.exit(f"mark letter must be a single letter: {letter!r}")
    buffer_number, line = position(letter)
    if not buffer_number:
        sys.exit(f"mark {letter} is not set in this Neovim")
    # `` opens exactly at the mark; ': jumps to the start of the line.
    sent = nvim("--remote-send", f"<Cmd>normal! `{letter}<CR>")
    check(sent.stdout, sent.stderr, "jump")
    here = nvim("--remote-expr", "expand('%:p') .. ':' .. line('.')")
    print(f"jumped to {here.stdout.strip()}")

if args.open:
    path, _, line = args.open.rpartition(":")
    if not path or not line.isdigit():
        sys.exit(f"--open expects FILE:LINE, got {args.open!r}")
    resolved = os.path.realpath(os.path.abspath(path))
    if not os.path.isfile(resolved):
        sys.exit(f"no such file: {resolved}")
    count = file_line_count(resolved)
    if int(line) > count:
        sys.exit(f"{resolved} has {count} lines, cannot open line {line}")
    sent = nvim("--remote-send", f"<Cmd>edit +{int(line)} {resolved}<CR>")
    check(sent.stdout, sent.stderr, "open")
    here = nvim("--remote-expr", "expand('%:p') .. ':' .. line('.')")
    print(f"opened {here.stdout.strip()}")
