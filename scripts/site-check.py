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
        self.sections = []
        self.demo_rows = 0
        self.tiles = 0
        self.provider_cards = 0

    def handle_starttag(self, tag, attrs):
        d = dict(attrs)
        for attr in ("href", "src"):
            v = d.get(attr)
            if v and not v.startswith(("http", "#", "mailto:")):
                self.refs.append(v)
        if "id" in d:
            self.ids.add(d["id"])
        if tag == "section":
            self.sections.append(d.get("id"))
        if tag == "div":
            cls = d.get("class", "")
            if cls == "demo-row":
                self.demo_rows += 1
            elif cls == "card tile":
                self.tiles += 1
            elif cls == "card" and self.sections and self.sections[-1] == "providers":
                self.provider_cards += 1
        if tag not in self.VOID:
            self.stack.append(tag)

    def handle_endtag(self, tag):
        if tag == "section" and self.sections:
            self.sections.pop()
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
    # provider-count self-consistency: the demo rows, tiles, provider cards,
    # proof strip, section heading, and aria labels must all agree on N.
    n = w.demo_rows
    if w.tiles != n:
        FAILURES.append(f"tiles {w.tiles} != demo rows {n}")
    if w.provider_cards != n:
        FAILURES.append(f"provider cards {w.provider_cards} != demo rows {n}")
    words = {4: "four", 5: "five", 6: "six", 7: "seven"}
    if n not in words:
        FAILURES.append(f"provider count {n} outside the word map; extend it")
    else:
        word = words[n]
        proof = re.findall(r"<span>(\d+) providers</span>", html)
        if proof != [str(n)]:
            FAILURES.append(f"proof strip claims {proof}, want {[str(n)]}")
        heads = re.findall(r"<h2>(\w+) providers,", html)
        if [h.lower() for h in heads] != [word]:
            FAILURES.append(f"providers h2 claims {heads}, want {word}")
        for label in re.findall(r'aria-label="([^"]*providers[^"]*)"', html):
            if word not in label.lower().split(" providers")[0].split():
                FAILURES.append(f"aria label disagrees on count: {label!r}")
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
