"""Shared helpers: a small Rust lexer for brace matching and test stripping."""
import re


def _match_brace(text: str, i: int) -> int:
    """Index of the brace closing text[i] == '{', skipping strings, chars, comments."""
    depth, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            i = text.find("\n", i)
            i = n if i == -1 else i
            continue
        if text.startswith("/*", i):
            i = text.find("*/", i + 2)
            i = n if i == -1 else i + 2
            continue
        m = re.match(r'b?r(#*)"', text[i:i + 12])
        if m:
            close = '"' + m.group(1)
            j = text.find(close, i + m.end())
            i = n if j == -1 else j + len(close)
            continue
        if c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            i = j + 1
            continue
        if c == "'":
            m = re.match(r"'(\\.[^']*|[^\\'])'", text[i:i + 12])
            if m:
                i += m.end()
                continue
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return n - 1


def strip_tests(text: str) -> str:
    """Blank out `#[cfg(test)]` modules so test code is ignored."""
    out = list(text)
    for m in re.finditer(r'#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{', text):
        close = _match_brace(text, m.end() - 1)
        for k in range(m.start(), close + 1):
            if out[k] != "\n":
                out[k] = " "
    return "".join(out)
