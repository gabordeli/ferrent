#!/usr/bin/env python3
"""Convert cargo/clippy JSON diagnostics to SonarQube generic issues.

The generic issue format requires the `rules` block since SonarQube 10.x;
without it the server imports the report in a deprecated format.
"""

import json
import pathlib
import sys

# Clippy has no clean-code taxonomy of its own; every diagnostic is a
# maintainability finding, escalated to HIGH for errors.
SEVERITY = {"warning": "MEDIUM", "error": "HIGH"}
RANK = {severity: rank for rank, severity in enumerate(("INFO", "LOW", "MEDIUM", "HIGH"))}


def rule_for(rules: dict, rule_id: str, severity: str) -> dict:
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


def _span_of(message: dict) -> dict | None:
    spans = message.get("spans", [])
    span = next((span for span in spans if span.get("is_primary")), None)
    if span is None and spans:
        span = spans[0]
    return span


def _diagnostic_of(record: dict) -> tuple[str, str, str, dict] | None:
    """Return (rule id, level, text, span) for a usable clippy diagnostic."""
    if record.get("reason") != "compiler-message":
        return None
    message = record.get("message", {})
    level = message.get("level")
    if level not in SEVERITY:
        return None
    span = _span_of(message)
    if not span:
        return None
    file_name = span.get("file_name")
    line = span.get("line_start")
    if not file_name or not line or not pathlib.Path(file_name).exists():
        return None
    code = (message.get("code") or {}).get("code") or "clippy"
    return code, level, message.get("message", "clippy diagnostic"), span


def convert(source: pathlib.Path, target: pathlib.Path) -> tuple[int, int]:
    """Write the Sonar report and return (issue count, rule count)."""
    issues = []
    rules = {}
    seen = set()

    for raw in source.read_text().splitlines():
        try:
            record = json.loads(raw)
        except json.JSONDecodeError:
            continue
        diagnostic = _diagnostic_of(record)
        if diagnostic is None:
            continue
        code, level, text, span = diagnostic
        file_name = span["file_name"]
        line = span["line_start"]
        key = (file_name, line, code, text)
        if key in seen:
            continue
        seen.add(key)
        rule_for(rules, code, SEVERITY[level])
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
    return len(issues), len(rules)


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else argv
    if len(args) != 2:
        print("usage: clippy_to_sonar.py <clippy.json> <sonar-issues.json>", file=sys.stderr)
        return 2
    issues, rules = convert(pathlib.Path(args[0]), pathlib.Path(args[1]))
    print(f"sonar generic issues: {issues} in {rules} rules")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
