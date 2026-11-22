#!/usr/bin/env python3
"""Convert cargo/clippy JSON diagnostics to SonarQube generic issues.

The generic issue format requires the `rules` block since SonarQube 10.x;
without it the server imports the report in a deprecated format.
"""

import json
import pathlib
import sys

source = pathlib.Path(sys.argv[1])
target = pathlib.Path(sys.argv[2])
issues = []
rules = {}
seen = set()

# Clippy has no clean-code taxonomy of its own; every diagnostic is a
# maintainability finding, escalated to HIGH for errors.
SEVERITY = {"warning": "MEDIUM", "error": "HIGH"}
RANK = {severity: rank for rank, severity in enumerate(("INFO", "LOW", "MEDIUM", "HIGH"))}


def rule_for(rule_id: str, severity: str) -> dict:
    rule = rules.get(rule_id)
    if rule is None:
        rule = {
            "id": rule_id,
            "name": rule_id,
            "engineId": "clippy",
            "cleanCodeAttribute": "CONVENTIONAL",
            "impacts": [{"softwareQuality": "MAINTAINABILITY", "severity": severity}],
        }
        rules[rule_id] = rule
        return rule
    # The severity lives on the rule, so a diagnostic that is more severe than
    # the first one seen must raise it instead of being downgraded.
    impact = rule["impacts"][0]
    if RANK[severity] > RANK[impact["severity"]]:
        impact["severity"] = severity
    return rule


for raw in source.read_text().splitlines():
    try:
        record = json.loads(raw)
    except json.JSONDecodeError:
        continue
    if record.get("reason") != "compiler-message":
        continue
    message = record.get("message", {})
    level = message.get("level")
    if level not in SEVERITY:
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
    severity = SEVERITY[level]
    rule_for(code, severity)
    issues.append(
        {
            "ruleId": code,
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

target.write_text(
    json.dumps({"rules": list(rules.values()), "issues": issues}, indent=2) + "\n"
)
print(f"sonar generic issues: {len(issues)} in {len(rules)} rules")
