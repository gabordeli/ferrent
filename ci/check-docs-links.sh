#!/usr/bin/env bash
# Checks that every relative markdown link points at an existing file and that
# the anchors used across the docs exist. Runs in the fast gate.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

python3 - <<'PY'
import pathlib
import re
import sys

ROOT = pathlib.Path(".").resolve()
LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
SKIP_DIRS = {"target", "node_modules", ".git", "dist", "coverage", ".playwright-mcp"}

def docs():
    for path in ROOT.rglob("*.md"):
        if any(part in SKIP_DIRS for part in path.parts):
            continue
        yield path

def anchors(path):
    text = path.read_text(encoding="utf-8", errors="replace")
    found = set()
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("#"):
            title = line.lstrip("#").strip().lower()
            slug = re.sub(r"[^\w\s-]", "", title).strip().replace(" ", "-")
            found.add(slug)
    return found

problems = []
cache = {}
for path in docs():
    text = path.read_text(encoding="utf-8", errors="replace")
    for target in LINK.findall(text):
        if target.startswith(("http://", "https://", "mailto:", "tel:")):
            continue
        file_part, _, anchor = target.partition("#")
        if not file_part:
            resolved = path
        else:
            resolved = (path.parent / file_part).resolve()
        if not resolved.exists():
            problems.append(f"{path.relative_to(ROOT)}: missing {target}")
            continue
        if anchor and resolved.suffix == ".md":
            known = cache.setdefault(resolved, anchors(resolved))
            if anchor not in known:
                problems.append(f"{path.relative_to(ROOT)}: missing anchor {target}")

if problems:
    print("\n".join(problems))
    sys.exit(1)
print("docs links: ok")
PY
