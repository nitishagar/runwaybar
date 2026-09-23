#!/usr/bin/env python3
"""WCAG 2.2 contrast arithmetic over the site's token pairs (invariant #16).

Fails (exit 1) if any TEXT pair < 4.5:1 or any UI pair < 3:1.
Pairs are declared here explicitly so a token change cannot silently regress.
"""
import re
import sys
from pathlib import Path

TOKENS = {}


def lum(hexcolor: str) -> float:
    h = hexcolor.lstrip("#")
    r, g, b = (int(h[i : i + 2], 16) / 255 for i in (0, 2, 4))
    def f(c):
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
    r, g, b = f(r), f(g), f(b)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def ratio(a: str, b: str) -> float:
    la, lb = lum(a), lum(b)
    lo, hi = min(la, lb), max(la, lb)
    return (hi + 0.05) / (lo + 0.05)


def load_tokens(path: str, theme: str):
    src = Path(path).read_text()
    block = ""
    if theme == "dark":
        m = re.search(r":root\s*\{([^}]*)\}", src)
        block = m.group(1)
    else:
        m = re.search(r'html\[data-theme="light"\]\s*\{([^}]*)\}', src)
        block = m.group(1)
    for name, value in re.findall(r"(--[\w-]+)\s*:\s*([^;]+);", block):
        color = value.strip().split()[0]
        if re.fullmatch(r"#[0-9a-fA-F]{6}", color):
            TOKENS[(theme, name)] = color


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    css = root / "site" / "assets" / "tokens.css"
    load_tokens(css, "dark")
    load_tokens(css, "light")

    text_pairs = [
        ("--text", "--bg-canvas"),
        ("--text-base", "--bg-canvas"),
        ("--muted-base", "--bg-canvas"),
        ("--accent", "--bg-canvas"),
        ("--text", "--panel-base"),
        ("--accent", "--bg-deep"),
        ("--terminal-fg", "--terminal-bg"),
    ]
    ui_pairs = [
        ("--line-base", "--bg-canvas"),
        ("--thread-blue", "--bg-canvas"),
    ]

    failures = 0
    for theme in ("dark", "light"):
        for fg, bg in text_pairs:
            r = ratio(TOKENS[(theme, fg)], TOKENS[(theme, bg)])
            ok = r >= 4.5
            print(f"{theme:5} TEXT  {fg} on {bg}: {r:.2f}:1 {'PASS' if ok else 'FAIL'}")
            failures += 0 if ok else 1
        for fg, bg in ui_pairs:
            r = ratio(TOKENS[(theme, fg)], TOKENS[(theme, bg)])
            ok = r >= 3.0
            print(f"{theme:5} UI   {fg} on {bg}: {r:.2f}:1 {'PASS' if ok else 'FAIL'}")
            failures += 0 if ok else 1

    if failures:
        print(f"\n{failures} contrast failures")
        return 1
    print("\nall contrast pairs pass")
    return 0


if __name__ == "__main__":
    sys.exit(main())
