#!/usr/bin/env python3
"""Convert cargo/clippy JSON diagnostics to SonarQube generic issues."""

import json
import pathlib
import sys

source = pathlib.Path(sys.argv[1])
target = pathlib.Path(sys.argv[2])
issues = []
seen = set()

for raw in source.read_text().splitlines():
    try:
        record = json.loads(raw)
    except json.JSONDecodeError:
        continue
    if record.get("reason") != "compiler-message":
        continue
    message = record.get("message", {})
    if message.get("level") not in ("warning", "error"):
        continue
    spans = message.get("spans", [])
    span = next((span for span in spans if span.get("is_primary")), None)
    if span is None and spans:
        span = spans[0]
    if not span:
        continue
    file_name = span.get("file_name")
    line = span.get("line_start")
    if not file_name or not line or not pathlib.Path(file_name).exists():
        continue
    code = (message.get("code") or {}).get("code") or "clippy"
    text = message.get("message", "clippy diagnostic")
    key = (file_name, line, code, text)
    if key in seen:
        continue
    seen.add(key)
    issues.append(
        {
            "engineId": "clippy",
            "ruleId": code,
            "severity": "CRITICAL" if message.get("level") == "error" else "MAJOR",
            "type": "CODE_SMELL",
            "primaryLocation": {
                "message": text,
                "filePath": file_name,
                "textRange": {
                    "startLine": line,
                    "startColumn": span.get("column_start", 1),
                },
            },
        }
    )

target.write_text(json.dumps({"issues": issues}, indent=2) + "\n")
print(f"sonar generic issues: {len(issues)}")
