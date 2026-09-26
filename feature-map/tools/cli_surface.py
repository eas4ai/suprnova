"""CLI surface, read from the built binaries' own help output.

`suprnova` is the developer CLI (suprnova-cli). `app` is a binary built on
`suprnova::Application`, so its subcommands are the framework's app-runner
commands. `console` dispatches the framework's registered console commands
plus the demo app's own; commands defined under `app/src/commands` are
excluded as demo-app code. Every subcommand's `--help` is read, so each
flag and argument is listed exactly as clap renders it.
"""
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(sys.argv[1])
BIN = Path(sys.argv[2])
OUT = Path(sys.argv[3])
EXCL = Path(sys.argv[4])


def run(args):
    p = subprocess.run(args, capture_output=True, text=True, timeout=60, cwd=ROOT / "app",
                       env={"PATH": "/usr/bin:/bin", "HOME": "/tmp", "APP_ENV": "local"})
    return (p.stdout + p.stderr).replace("\r", "")


def sections(help_text):
    """Split clap help into {"Commands": [...], "Options": [...], "Arguments": [...]}."""
    out, cur = {}, None
    for line in help_text.splitlines():
        m = re.match(r'^([A-Z][A-Za-z ]+):\s*$', line)
        if m:
            cur = m.group(1).strip()
            out[cur] = []
            continue
        if cur and line.startswith("  ") and line.strip():
            out[cur].append(line.rstrip())
        elif cur and not line.strip():
            continue
    return out


def entries(lines):
    """(name, description) pairs from a clap two-column section."""
    items = []
    for line in lines:
        m = re.match(r'^  (\S.*?)(?:\s{2,}(.*))?$', line)
        if not m:
            continue
        if line.startswith("    ") or line.startswith("\t"):
            if items:
                items[-1] = (items[-1][0], (items[-1][1] + " " + line.strip()).strip())
            continue
        items.append((m.group(1).strip(), (m.group(2) or "").strip()))
    return items


def kebab(name):
    return re.sub(r'(?<!^)(?=[A-Z])', '-', name).lower()


def enum_commands(rel, enum):
    """(command name, line) for each variant of a clap Subcommand enum."""
    text = (ROOT / rel).read_text().splitlines()
    start = next(i for i, l in enumerate(text) if re.match(rf'\s*(pub(\([^)]*\))?\s+)?enum {enum}\b', l))
    out, pending, depth = [], None, 0
    for i in range(start + 1, len(text)):
        line = text[i]
        if depth == 0:
            m = re.search(r'#\[command\(name\s*=\s*"([^"]+)"', line)
            if m:
                pending = (m.group(1), i + 1)
            v = re.match(r'^\s{4}([A-Z][A-Za-z0-9]*)\b', line)
            if v:
                out.append(pending if pending else (kebab(v.group(1)), i + 1))
                pending = None
        depth += line.count("{") - line.count("}")
        if depth < 0:
            break
    return out


def hidden_console_entries():
    """`inventory::submit!` CommandEntry registrations whose clap builder hides them."""
    consts = {}
    for f in (ROOT / "framework/src").rglob("*.rs"):
        for m in re.finditer(r'const\s+([A-Z_]+)\s*:\s*&str\s*=\s*"([^"]+)"', f.read_text()):
            consts[m.group(1)] = m.group(2)
    out = []
    for f in sorted((ROOT / "framework/src").rglob("*.rs")):
        text = f.read_text()
        if ".hide(true)" not in text:
            continue
        for m in re.finditer(r'CommandEntry\s*\{\s*name:\s*([A-Z_]+|"[^"]+")', text):
            raw = m.group(1)
            name = raw.strip('"') if raw.startswith('"') else consts.get(raw)
            if name:
                out.append((name, f"{f.relative_to(ROOT)}:{text[:m.start()].count(chr(10)) + 1}"))
    return out


def find_line(roots, name):
    """First declaration-looking occurrence of a command name under roots."""
    for root in roots:
        for f in sorted((ROOT / root).rglob("*.rs")):
            text = f.read_text()
            for pat in (f'name = "{name}"', f'"{name}"'):
                i = text.find(pat)
                if i != -1:
                    return f"{f.relative_to(ROOT)}:{text[:i].count(chr(10)) + 1}"
    return "(declaration not found)"


def digest(text):
    return hashlib.sha256(" ".join(text.split()).encode()).hexdigest()[:16]


demo_cmds = set()
for f in (ROOT / "app/src/commands").glob("*.rs"):
    demo_cmds.update(n for n in re.findall(r'name\s*=\s*"([^"]+)"', f.read_text())
                     if re.fullmatch(r'[a-z][a-z0-9:_-]*', n))

recs, counts, rejected_all = [], {}, {}
APP_ENUM = [(n, f"framework/src/app/mod.rs:{ln}") for n, ln in enum_commands("framework/src/app/mod.rs", "Commands")]
CLI_ENUM = [(n, f"suprnova-cli/src/main.rs:{ln}") for n, ln in enum_commands("suprnova-cli/src/main.rs", "Commands")]
for binary in ("suprnova", "app", "console"):
    listed = None
    if binary == "suprnova":
        cands = CLI_ENUM
    elif binary == "app":
        cands = APP_ENUM
    else:
        top = run([str(BIN / binary), "help"]) + run([str(BIN / binary), "--help"])
        names = [c[0].split(",")[0].split()[0] for c in entries(sections(top).get("Commands", []))]
        names = [n for n in dict.fromkeys(names) if n != "help" and n not in demo_cmds]
        listed = set(names)
        cands = [(n, find_line(["framework/src"], n)) for n in names]
        cands += [(n, at) for n, at in hidden_console_entries() if n not in names]
    n = 0
    rejected = []
    for name, at in cands:
        sub = run([str(BIN / binary), name, "--help"])
        if "Usage:" not in sub and "USAGE" not in sub.upper():
            rejected.append(name)
            continue
        sec = sections(sub)
        first = [l for l in sub.splitlines() if l.strip()]
        desc = first[0].strip() if first and not first[0].startswith("Usage") else ""
        args = [{"spec": a_, "help": d_} for a_, d_ in entries(sec.get("Arguments", []))]
        opts = [{"spec": a_, "help": d_} for a_, d_ in entries(sec.get("Options", []))
                if not a_.startswith(("-h, --help", "-V, --version"))]
        file, _, line = at.rpartition(":")
        recs.append({"id": f"{binary} {name}", "kind": "command", "family": "cli", "parent": None,
                     "crate": {"suprnova": "suprnova-cli"}.get(binary, "suprnova"), "module": binary,
                     "file": file or None, "line": int(line) if line.isdigit() else None,
                     "details": {"binary": binary, "description": desc, "arguments": args, "options": opts,
                                 "hidden_from_help": listed is not None and name not in listed},
                     "sig_hash": digest(sub), "body_hash": None})
        n += 1
    counts[binary] = n
    rejected_all[binary] = rejected

OUT.write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in recs))
EXCL.write_text(json.dumps({"demo_app_commands": sorted(demo_cmds),
                            "declared_but_rejected_by_binary": rejected_all}, indent=1, sort_keys=True))
print(json.dumps({"records": len(recs), "counts": counts}))
