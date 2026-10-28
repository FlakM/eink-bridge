#!/usr/bin/env python3
"""Set Neovim global named marks (A-Z) from outside a running Neovim.

The skill opens the PR in Neovim, then marks the most important locations so the
reviewer can jump to them with `'A`, `'B`, ... Each mark is set on its target
buffer, so the current window and an open diff view stay as they are.

Two traps this script defends against, both verified against real Neovim:

1. `nvim --server ...` exits 0 even when it failed to connect (E247). So success is
   decided by reading the marks back, never by the exit code.
2. `--remote-send` feeds keys, so a `<Cmd>` never executes while the reader is in insert
   mode. Every change here goes through `--remote-expr luaeval(...)` instead, which runs
   in any mode and cannot touch the buffer.

Marks also persist in shada, so a silent miss would read back as a stale mark from an
earlier session. Verification is therefore authoritative, not a nicety.

A note (`--comment A:text`) is deliberately quiet: a `▶` in the sign column, no extra
rows over the code, and `:EinkNote` on that line prints it. `--inline A,B` draws those
notes above their lines for the findings that must not be missed. Notes are extmarks:
display only, never written to the file, and they move with the text.

Usage:
  nvim_marks.py --socket /tmp/x.sock --marks "A:src/app.rs:42,B:src/lib.rs:7"
  nvim_marks.py --socket /tmp/x.sock --comment "A:this trusts the header" --verify-comments A
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
parser.add_argument("--comment", action="append", default=[], metavar="LETTER:TEXT",
                    help="attach TEXT to the mark: a gutter sign now, one :EinkNote away (repeatable)")
parser.add_argument("--inline", default="", help="comma list of letters whose note is drawn above its line")
parser.add_argument("--verify-comments", default="", help="comma list of letters whose note to read back")
parser.add_argument("--clear-comments", action="store_true", help="remove every note this script drew")
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


NOTES_NS = "eink-review-notes"


def run_lua(body):
    """Run a Lua chunk in the editor and return what it returns.

    Not --remote-send: that feeds keys, and a <Cmd> mapping never executes while the
    reader is in insert mode, so a mark or a note would silently not appear.
    """
    handle = tempfile.NamedTemporaryFile("w", suffix=".lua", delete=False)
    try:
        handle.write(body.rstrip("\n") + "\n")
        handle.close()
        out = nvim("--remote-expr", f'luaeval(\'dofile("{handle.name}")\')')
        check(out.stdout, out.stderr, "lua")
        if out.returncode != 0 or "E5108" in out.stderr or "Error executing" in out.stderr:
            sys.exit(f"lua failed: {(out.stderr or out.stdout).strip()}")
        return out.stdout.strip()
    finally:
        os.unlink(handle.name)


def comment_spec(spec):
    letter, _, text = spec.partition(":")
    letter, text = letter.strip(), text.strip()
    if len(letter) != 1 or not letter.isalpha():
        sys.exit(f"comment letter must be a single letter: {letter!r}")
    if not text:
        sys.exit(f"comment {letter} has no text")
    return {"letter": letter, "text": text}


def comments_lua(items):
    """Register each note and mark its line in the gutter.

    The default is deliberately quiet: a `▶` in the sign column, no extra rows. The text
    is one `:EinkNote` away on the line the cursor is on, and `--inline` puts it on the
    page for the letters that matter most.
    """
    lines = [
        'vim.api.nvim_set_hl(0, "EinkNote", { link = "DiagnosticWarn", default = true })',
        'vim.api.nvim_set_hl(0, "EinkNoteMark", { link = "Todo", default = true })',
        f'local ns = vim.api.nvim_create_namespace("{NOTES_NS}")',
        "_G.__eink_notes = _G.__eink_notes or {}",
        "local items = {",
    ]
    lines += [
        f"  {{ letter = {lua_string(i['letter'])}, text = {lua_string(i['text'])}, buf = {i['buf']}, line = {i['line']} }},"
        for i in items
    ]
    lines += [
        "}",
        "for _, it in ipairs(items) do",
        "  _G.__eink_notes[it.buf] = _G.__eink_notes[it.buf] or {}",
        "  _G.__eink_notes[it.buf][it.line] = { letter = it.letter, text = it.text }",
        "  -- one sign per line: a second call must not stack a second marker",
        "  for _, old in ipairs(vim.api.nvim_buf_get_extmarks(it.buf, ns, { it.line - 1, 0 }, { it.line - 1, -1 }, {})) do",
        "    vim.api.nvim_buf_del_extmark(it.buf, ns, old[1])",
        "  end",
        "  vim.api.nvim_buf_set_extmark(it.buf, ns, it.line - 1, 0, {",
        '    sign_text = "▶",',
        '    sign_hl_group = "EinkNoteMark",',
        "  })",
        "end",
        'vim.api.nvim_create_user_command("EinkNote", function()',
        "  local buf = vim.api.nvim_get_current_buf()",
        "  local lnum = vim.api.nvim_win_get_cursor(0)[1]",
        "  local note = ((_G.__eink_notes or {})[buf] or {})[lnum]",
        "  if not note then",
        '    vim.notify("no eink note on this line", vim.log.levels.INFO)',
        "    return",
        "  end",
        '  vim.api.nvim_echo({ { "▶ " .. note.letter .. "  ", "EinkNoteMark" }, { note.text, "EinkNote" } }, false, {})',
        'end, { force = true, desc = "show the eink review note on the cursor line" })',
        'return "marked " .. #items .. " line(s): " .. table.concat(vim.tbl_map(function(i) return i.letter end, items), ",")',
    ]
    return "\n".join(lines)


def inline_lua(items):
    """Draw the notes above their lines, for a reader who asked to see them."""
    lines = [
        'vim.api.nvim_set_hl(0, "EinkNote", { link = "DiagnosticWarn", default = true })',
        'vim.api.nvim_set_hl(0, "EinkNoteMark", { link = "Todo", default = true })',
        'local ns = vim.api.nvim_create_namespace("eink-review-inline")',
        "local items = {",
    ]
    lines += [
        f"  {{ letter = {lua_string(i['letter'])}, buf = {i['buf']}, line = {i['line']} }},"
        for i in items
    ]
    lines += [
        "}",
        "local seen = {}",
        "for _, it in ipairs(items) do",
        "  if not seen[it.buf] then",
        "    seen[it.buf] = true",
        "    vim.api.nvim_buf_clear_namespace(it.buf, ns, 0, -1)",
        "  end",
        "end",
        "for _, it in ipairs(items) do",
        "  local note = ((_G.__eink_notes or {})[it.buf] or {})[it.line]",
        '  local text = note and note.text or "(no note)"',
        "  vim.api.nvim_buf_set_extmark(it.buf, ns, it.line - 1, 0, {",
        '    virt_lines = { { { "▌ ", "EinkNoteMark" }, { "▶ " .. it.letter .. "  ", "EinkNoteMark" }, { text, "EinkNote" } } },',
        "    virt_lines_above = true,",
        "  })",
        "end",
        'return "inlined " .. #items .. " note(s): " .. table.concat(vim.tbl_map(function(i) return i.letter end, items), ",")',
    ]
    return "\n".join(lines)


def verify_comments_lua(items):
    lines = [
        "_G.__eink_notes = _G.__eink_notes or {}",
        "local items = {",
    ]
    lines += [
        f"  {{ letter = {lua_string(i['letter'])}, buf = {i['buf']}, line = {i['line']} }},"
        for i in items
    ]
    lines += [
        "}",
        "local out = {}",
        "for _, it in ipairs(items) do",
        "  local note = ((_G.__eink_notes or {})[it.buf] or {})[it.line]",
        '  out[#out + 1] = it.letter .. "=" .. (note and note.text or "")',
        "end",
        'return table.concat(out, "\\n")',
    ]
    return "\n".join(lines)


CLEAR_LUA = (
    'local ns = vim.api.nvim_create_namespace("' + NOTES_NS + '")\n'
    'local inline = vim.api.nvim_create_namespace("eink-review-inline")\n'
    "local cleared = 0\n"
    "for _, buf in ipairs(vim.api.nvim_list_bufs()) do\n"
    "  if vim.api.nvim_buf_is_loaded(buf) then\n"
    "    local found = vim.api.nvim_buf_get_extmarks(buf, ns, 0, -1, {})\n"
    "    cleared = cleared + #found\n"
    "    vim.api.nvim_buf_clear_namespace(buf, ns, 0, -1)\n"
    "    vim.api.nvim_buf_clear_namespace(buf, inline, 0, -1)\n"
    "  end\n"
    "end\n"
    "_G.__eink_notes = {}\n"
    'return "cleared " .. cleared .. " note(s)"'
)


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


def note_items(letters, texts=None):
    """Resolve letters to their buffer and line, so a note lands on the mark itself."""
    items = []
    for letter in letters:
        buf, line = position(letter)
        if not buf:
            sys.exit(f"mark {letter} is not set; set it with --marks first")
        item = {"letter": letter, "buf": buf, "line": line}
        if texts is not None:
            item["text"] = texts.get(letter, "")
        items.append(item)
    return items


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
    run_lua("\n".join(script))
    # Read the marks back: a mark that silently missed reads as one from an older
    # review, and marks are shared through shada.
    pending = [i for i in items if not landed(i)]
    if pending:
        detail = []
        for i in pending:
            buffer_number, line = position(i["letter"])
            where = buffer_path(buffer_number) if buffer_number else "unset"
            detail.append(f"{i['letter']}: expected {i['file']}:{i['line']}, got {where}:{line}")
        sys.exit("marks not set: " + "; ".join(detail))
    print(f"set {len(items)} mark(s): " + ", ".join(i["letter"] for i in items))

if args.verify:
    for letter in filter(None, (s.strip() for s in args.verify.split(","))):
        buffer_number, line = position(letter)
        print(f"{letter}=unset" if not buffer_number else f"{letter}={buffer_path(buffer_number)}:{line}")

if args.clear_comments:
    print(run_lua(CLEAR_LUA))

comments = {}
for spec in args.comment:
    parsed = comment_spec(spec)
    comments[parsed["letter"]] = parsed["text"]

if comments:
    print(run_lua(comments_lua(note_items(sorted(comments), comments))))

if args.inline:
    letters = [s.strip() for s in args.inline.split(",") if s.strip()]
    print(run_lua(inline_lua(note_items(letters))))

if args.verify_comments:
    letters = [s.strip() for s in args.verify_comments.split(",") if s.strip()]
    out = run_lua(verify_comments_lua(note_items(letters)))
    for line in out.splitlines():
        print(line)

if args.jump:
    letter = args.jump.strip()
    if len(letter) != 1 or not letter.isalpha():
        sys.exit(f"mark letter must be a single letter: {letter!r}")
    buffer_number, line = position(letter)
    if not buffer_number:
        sys.exit(f"mark {letter} is not set in this Neovim")
    # An expression, not keys: jumping must not depend on the mode the reader is in.
    run_lua(f'vim.cmd("normal! `" .. {lua_string(letter)})')
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
    run_lua(f'vim.cmd("edit +{int(line)} " .. vim.fn.fnameescape({lua_string(resolved)}))')
    here = nvim("--remote-expr", "expand('%:p') .. ':' .. line('.')")
    print(f"opened {here.stdout.strip()}")
