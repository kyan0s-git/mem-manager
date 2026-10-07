#!/usr/bin/env python3
"""Renders bench JSON (memmanager --bench / MemManager --bench) as Markdown.

    python3 scripts/bench_report.py bench-windows.json bench-macos.json > bench.md
"""
import json
import sys

OS_TITLE = {"windows": "Windows", "macos": "macOS"}


def gib(b: float) -> str:
    return f"{b / (1 << 30):.2f} GiB"


def render(path: str) -> str:
    with open(path, encoding="utf-8") as f:
        d = json.load(f)
    os_name = OS_TITLE.get(d.get("os"), d.get("os", "?"))
    ram = d.get("total_ram", 0)
    extra = f", build {d['build']}" if d.get("build") else ""
    out = [f"### {os_name} (CI runner, {gib(ram)} RAM{extra})", ""]
    out.append("| Bloat | Action | Freed | % of RAM | Cost afterwards |")
    out.append("|---|---|---:|---:|---|")
    for s in d.get("scenarios", []):
        if s.get("skipped"):
            out.append(f"| {s['name']} | {s['action']} | — | — | skipped: {s['skipped']} |")
            continue
        freed = max(0, s.get("freed_bytes", 0))
        out.append(
            f"| {s['name']} | {s['action']} | {gib(freed)} | {max(0.0, s.get('freed_pct_ram', 0)):.1f}% | {s.get('cost', '')} |"
        )
    if d.get("failures"):
        out += ["", "Bench notes: " + "; ".join(d["failures"])]
    return "\n".join(out) + "\n"


def main(paths):
    parts = ["## Measured memory reduction", ""]
    parts.append(
        "Each row creates real bloat on a GitHub-hosted runner, runs the action through the same code path "
        "the app uses, and measures memory freed plus the cost afterwards. Numbers vary by machine and workload."
    )
    parts.append("")
    for p in paths:
        try:
            parts.append(render(p))
        except (OSError, ValueError, KeyError) as e:
            parts.append(f"_Could not read {p}: {e}_\n")
    sys.stdout.write("\n".join(parts))


if __name__ == "__main__":
    main(sys.argv[1:])
