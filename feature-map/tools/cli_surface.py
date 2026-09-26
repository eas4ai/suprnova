"""CLI surface, read from the built binaries' own help output.

`suprnova` is the developer CLI (suprnova-cli). `app` is a binary built on
`suprnova::Application`, so its subcommands are the framework's app-runner
commands. `console` dispatches the framework's registered console commands
plus the demo app's own; commands defined under `app/src/commands` are
excluded as demo-app code. Every subcommand's `--help` is read, so each
flag and argument is listed exactly as clap renders it.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(sys.argv[1])
REV = sys.argv[2]
BIN = Path(sys.argv[3])
OUT = Path(sys.argv[4])


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


checked = set()
if OUT.exists():
    for line in OUT.read_text().splitlines():
        m = re.match(r'\s*- \[x\] (?:[a-z ]+ )?`([^`]+)`', line)
        if m:
            checked.add(m.group(1))


def box(k):
    return "x" if k in checked else " "


demo_cmds = set()
for f in (ROOT / "app/src/commands").glob("*.rs"):
    demo_cmds.update(n for n in re.findall(r'name\s*=\s*"([^"]+)"', f.read_text())
                     if re.fullmatch(r'[a-z][a-z0-9:_-]*', n))

L = ["# Suprnova feature map: command line", "",
     f"Source: binaries built from the repository at {REV}; every entry is read from the "
     "binary's own `--help` output, and each command's declaration is cited.", "",
     "A checked box means the documentation for that item has been remediated against the source.", ""]
counts = {}
body = []
APP_ENUM = [(n, f"framework/src/app/mod.rs:{ln}") for n, ln in enum_commands("framework/src/app/mod.rs", "Commands")]
CLI_ENUM = [(n, f"suprnova-cli/src/main.rs:{ln}") for n, ln in enum_commands("suprnova-cli/src/main.rs", "Commands")]
for binary, title in (("suprnova", "`suprnova` developer CLI (suprnova-cli)"),
                      ("app", "App runner (`suprnova::Application`; the project binary)"),
                      ("console", "Console binary (framework-registered commands)")):
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
    body += ["", f"## {title}", ""]
    n = 0
    rejected = []
    for name, at in cands:
        key = f"{binary} {name}"
        sub = run([str(BIN / binary), name, "--help"])
        sec = sections(sub)
        if "Usage:" not in sub and "USAGE" not in sub.upper():
            rejected.append(name)
            continue
        desc = ""
        first = [l for l in sub.splitlines() if l.strip()]
        if first and not first[0].startswith("Usage"):
            desc = first[0].strip()
        hidden = binary == "console" and name not in listed
        body.append(f"- [{box(key)}] command `{key}` · {at}" +
                    (" (hidden from `help`; dispatchable)" if hidden else ""))
        if desc:
            body.append(f"  - {desc}")
        for kind in ("Arguments", "Options"):
            for arg, adesc in entries(sec.get(kind, [])):
                if arg.startswith("-h, --help") or arg.startswith("-V, --version"):
                    continue
                body.append(f"  - {kind[:-1].lower()} `{arg}`" + (f": {adesc}" if adesc else ""))
        n += 1
    counts[binary] = n
    if rejected:
        body += ["", "Declared in source but not accepted by the built binary: " + ", ".join(f"`{r}`" for r in rejected)]
    if binary == "console":
        body += ["", f"Excluded as demo-app commands (`app/src/commands`): {', '.join(f'`{c}`' for c in sorted(demo_cmds))}"]

L += ["## Counts", ""] + [f"- `{b}`: {n} commands" for b, n in counts.items()] + body
OUT.write_text("\n".join(L) + "\n")
print(json.dumps({"counts": counts, "checked_preserved": len(checked)}))
