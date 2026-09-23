#!/usr/bin/env python3
"""Static site checks: every referenced asset resolves, HTML is well-formed,
noscript fallback exists, payload is within budget."""
import re
import sys
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "site"
FAILURES = []


class Walker(HTMLParser):
    VOID = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"}

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.stack = []
        self.refs = []
        self.ids = set()

    def handle_starttag(self, tag, attrs):
        d = dict(attrs)
        for attr in ("href", "src"):
            v = d.get(attr)
            if v and not v.startswith(("http", "#", "mailto:")):
                self.refs.append(v)
        if "id" in d:
            self.ids.add(d["id"])
        if tag not in self.VOID:
            self.stack.append(tag)

    def handle_endtag(self, tag):
        if not self.stack:
            FAILURES.append(f"unmatched closing </{tag}>")
            return
        if self.stack[-1] == tag:
            self.stack.pop()
        else:
            FAILURES.append(f"mismatched </{tag}> (open: {self.stack[-1]})")


def main() -> int:
    html = (ROOT / "index.html").read_text()
    w = Walker()
    w.feed(html)
    if w.stack:
        FAILURES.append(f"unclosed tags: {w.stack}")
    for ref in w.refs:
        if not (ROOT / ref).exists():
            FAILURES.append(f"missing asset: {ref}")
    # internal anchors
    for target in set(re.findall(r'href="#([\w-]+)"', html)):
        if target and target not in w.ids:
            FAILURES.append(f"broken anchor: #{target}")
    if "<noscript>" not in html:
        FAILURES.append("no noscript fallback for the tabs")
    # payload budget
    total = sum(p.stat().st_size for p in ROOT.rglob("*") if p.is_file())
    print(f"site payload: {total} bytes across {sum(1 for p in ROOT.rglob('*') if p.is_file())} files")
    if total > 61440:
        FAILURES.append(f"payload {total} exceeds 61440 bytes")
    if FAILURES:
        for f in FAILURES:
            print("FAIL:", f)
        return 1
    print("all site checks pass")
    return 0


if __name__ == "__main__":
    sys.exit(main())
