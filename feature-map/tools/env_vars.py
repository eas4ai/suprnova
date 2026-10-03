"""Extract environment variables the Suprnova crates read, from source.

Candidates are every SCREAMING_SNAKE string literal in non-test source.
A candidate is confirmed when it reaches an env-reading call: it sits on
the same logical statement as a callee whose name mentions env/var, or it is
the value of a constant that does. Code the CLI scaffolds counts too, and so
do `${VAR}` interpolations in the docker templates it writes. Every rejected
candidate is reported, so nothing is dropped silently.
"""
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from common import _match_brace, strip_tests  # noqa: E402

ROOT = Path(sys.argv[1])
CRATES = {
    "suprnova": ROOT / "framework/src",
    "suprnova-live": ROOT / "crates/suprnova-live/src",
    "suprnova-magnetar": ROOT / "crates/suprnova-magnetar/src",
    "suprnova-payments-stripe": ROOT / "crates/suprnova-payments-stripe/src",
    "suprnova-payments-paddle": ROOT / "crates/suprnova-payments-paddle/src",
    "suprnova-payments-nowpayments": ROOT / "crates/suprnova-payments-nowpayments/src",
    "suprnova-web-push": ROOT / "crates/suprnova-web-push/src",
    "suprnova-cli": ROOT / "suprnova-cli/src",
    "suprnova-macros": ROOT / "suprnova-macros/src",
}
LIT = re.compile(r'"([A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+)"')
CONST = re.compile(r'\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&(?:\'static\s+)?str\s*=\s*"([^"]+)"')
CALLEE = re.compile(r'\b([A-Za-z_][A-Za-z0-9_]*)\s*(?:::<[^>]*>)?\s*\(')
ENVISH = re.compile(r'(?i)(env|^var$|^var_os$)')
NOT_ENV = re.compile(r'(?i)envelope|environment_name')


def statements(text: str):
    """Yield (line_no, statement_text) splitting on ';' '{' '}' boundaries."""
    buf, start = [], 1
    line = 1
    for ch in text:
        if not buf:
            start = line
        buf.append(ch)
        if ch == "\n":
            line += 1
        if ch in ";{}":
            yield start, "".join(buf)
            buf = []
    if buf:
        yield start, "".join(buf)


def base_reader(name: str) -> bool:
    return bool(ENVISH.search(name)) and not NOT_ENV.search(name)


def envish_callees(stmt: str, readers=frozenset()) -> bool:
    for name in CALLEE.findall(stmt):
        if name in readers or base_reader(name):
            return True
    return False


FN_DEF = re.compile(r'\bfn\s+([a-z_][a-z0-9_]*)\s*(?:<[^>{]*>)?\s*\(')
CLOSURE = re.compile(r'\blet\s+([a-z_][a-z0-9_]*)\s*=\s*(?:move\s+)?\|')
FN_PARAM = re.compile(r'\b([a-z_][a-z0-9_]*)\s*:\s*(?:&(?:\'\w+\s+)?(?:dyn|impl)|impl|&?\s*[A-Z])\s*Fn(?:Mut|Once)?\s*\(\s*&(?:\'\w+\s+)?str')
GENERIC_FN = re.compile(r'\b([A-Z])\s*:\s*Fn(?:Mut|Once)?\s*\(\s*&(?:\'\w+\s+)?str')


def file_readers(text: str) -> set:
    """Names in this file that lead to an env read (fixed point)."""
    readers = set(m.group(1) for m in FN_PARAM.finditer(text))
    for g in GENERIC_FN.finditer(text):
        for pm in re.finditer(r'\b([a-z_][a-z0-9_]*)\s*:\s*&?' + g.group(1) + r'\b', text):
            readers.add(pm.group(1))
    bodies = []
    for m in FN_DEF.finditer(text):
        brace = text.find("{", m.end())
        semi = text.find(";", m.end())
        if brace == -1 or (semi != -1 and semi < brace):
            continue
        bodies.append((m.group(1), text[m.end():_match_brace(text, brace) + 1]))
    for m in CLOSURE.finditer(text):
        end = text.find(";", m.end())
        bodies.append((m.group(1), text[m.end():end if end != -1 else len(text)]))
    changed = True
    while changed:
        changed = False
        for name, body in bodies:
            if name not in readers and envish_callees(body, frozenset(readers)):
                readers.add(name)
                changed = True
    return readers


def is_test_path(p: Path) -> bool:
    s = str(p)
    return "/tests/" in s or s.endswith("_tests.rs") or s.endswith("/tests.rs")


found = {}      # name -> set((crate, file, line))
rejected = {}   # name -> set((crate, file, line))
consts = {}     # (crate, const_name) -> value

files = []
for crate, src in CRATES.items():
    for f in sorted(src.rglob("*.rs")):
        if is_test_path(f):
            continue
        files.append((crate, f, strip_tests(f.read_text())))
# Code that `suprnova new` generates reads env vars too; users configure those the same way.
for f in sorted((ROOT / "suprnova-cli/src/templates").rglob("*.rs.tpl")):
    files.append(("suprnova-cli (scaffold)", f, strip_tests(f.read_text())))

for crate, f, text in files:
    for m in CONST.finditer(text):
        consts[(crate, m.group(1))] = m.group(2)

const_hits = set()
for crate, f, text in files:
    rel = f.relative_to(ROOT)
    readers = frozenset(file_readers(text))
    for line, stmt in statements(text):
        env_stmt = envish_callees(stmt, readers)
        for m in LIT.finditer(stmt):
            name = m.group(1)
            ln = line + stmt[: m.start()].count("\n")
            (found if env_stmt else rejected).setdefault(name, set()).add((crate, str(rel), ln))
        if env_stmt:
            for ident in re.findall(r'\b([A-Z][A-Z0-9_]+)\b', stmt):
                if (crate, ident) in consts:
                    const_hits.add((crate, ident))

# Constants whose value reaches an env call through their identifier.
for crate, f, text in files:
    rel = f.relative_to(ROOT)
    for m in CONST.finditer(text):
        key = (crate, m.group(1))
        value = m.group(2)
        if key in const_hits and LIT.fullmatch(f'"{value}"'):
            ln = text[: m.start()].count("\n") + 1
            found.setdefault(value, set()).add((crate, str(rel), ln))

# Generated compose files read `${VAR}` / `${VAR:-default}` from the shell or .env.
COMPOSE_VAR = re.compile(r'\$\{([A-Z][A-Z0-9_]*)(?::?-[^}]*)?\}')
for f in sorted((ROOT / "suprnova-cli/src/templates").rglob("*.tpl")):
    if f.name.endswith(".rs.tpl"):
        continue
    for ln, text in enumerate(f.read_text().splitlines(), 1):
        for m in COMPOSE_VAR.finditer(text):
            found.setdefault(m.group(1), set()).add(("suprnova-cli (docker)", str(f.relative_to(ROOT)), ln))

for name in list(rejected):
    if name in found:
        del rejected[name]

BUILD_TIME = re.compile(r'^(CARGO_|OUT_DIR$|RUSTC|TARGET$|PROFILE$|HOST$)')
build_time = {k: v for k, v in list(found.items()) + list(rejected.items()) if BUILD_TIME.match(k)}
for k in build_time:
    found.pop(k, None)
    rejected.pop(k, None)

json.dump(
    {
        "env_vars": {k: sorted(v) for k, v in sorted(found.items())},
        "build_time": {k: sorted(v) for k, v in sorted(build_time.items())},
        "rejected_candidates": {k: sorted(v) for k, v in sorted(rejected.items())},
    },
    sys.stdout,
    indent=1,
)
