"""List keyword literals the proc-macro parsers match on, with context.

Output is raw evidence for hand review: file, line, enclosing fn, keyword,
and the source line. Test modules are stripped first.
"""
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from common import strip_tests  # noqa: E402

ROOT = Path(sys.argv[1])
PAT = re.compile(r'is_ident\("([a-z_]+)"\)|^\s*((?:"[a-z_0-9]+"\s*\|\s*)*"[a-z_0-9]+")\s*(?:if [^=]*)?=>|== "([a-z_]+)"|matches!\([^,]+,\s*((?:"[a-z_]+"\s*\|\s*)*"[a-z_]+")')
FN = re.compile(r'\bfn\s+([a-z_][a-z0-9_]*)')

rows = []
for f in sorted((ROOT / "suprnova-macros/src").rglob("*.rs")):
    text = strip_tests(f.read_text())
    fn = "?"
    for ln, line in enumerate(text.splitlines(), 1):
        m = FN.search(line)
        if m:
            fn = m.group(1)
        for m in PAT.finditer(line):
            raw = next(g for g in m.groups() if g)
            for kw in re.findall(r'[a-z_][a-z_0-9]*', raw):
                rows.append({"file": str(f.relative_to(ROOT)), "line": ln, "fn": fn,
                             "keyword": kw, "src": line.strip()[:140]})
json.dump(rows, sys.stdout, indent=0)
