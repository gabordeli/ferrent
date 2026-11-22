"""Tests for the clippy → Sonar generic issue converter."""

import json
import pathlib

from clippy_to_sonar import convert, main


def diagnostic(level: str, code: str | None, file_name: str, line: int) -> dict:
    message = {
        "level": level,
        "message": f"{level} from clippy",
        "spans": [{"file_name": file_name, "line_start": line, "column_start": 3}],
    }
    if code:
        message["code"] = {"code": code}
    return {"reason": "compiler-message", "message": message}


def write(tmp_path: pathlib.Path, records: list[dict]) -> pathlib.Path:
    source = tmp_path / "clippy.json"
    source.write_text("\n".join(json.dumps(record) for record in records) + "\n")
    return source


def read(target: pathlib.Path) -> dict:
    return json.loads(target.read_text())


def test_keeps_rules_and_issues(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "main.rs").write_text("fn main() {}\n")
    source = write(
        tmp_path,
        [
            diagnostic("warning", "clippy::needless_return", "main.rs", 1),
            diagnostic("warning", "clippy::needless_return", "main.rs", 1),
        ],
    )
    target = tmp_path / "issues.json"

    assert convert(source, target) == (1, 1)
    report = read(target)
    assert report["issues"][0]["ruleId"] == "clippy::needless_return"
    assert report["issues"][0]["primaryLocation"]["textRange"]["startLine"] == 1
    assert report["rules"][0]["engineId"] == "clippy"
    assert report["rules"][0]["impacts"][0]["severity"] == "MEDIUM"


def test_error_raises_the_rule_severity(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "main.rs").write_text("fn main() {}\n")
    source = write(
        tmp_path,
        [
            diagnostic("warning", "clippy::unwrap_used", "main.rs", 1),
            diagnostic("error", "clippy::unwrap_used", "main.rs", 2),
        ],
    )
    target = tmp_path / "issues.json"

    convert(source, target)
    report = read(target)
    assert len(report["issues"]) == 2
    assert report["rules"][0]["impacts"][0]["severity"] == "HIGH"


def test_skips_records_without_a_usable_span(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    source = write(
        tmp_path,
        [
            {"reason": "build-finished"},
            {"reason": "compiler-message", "message": {"level": "warning", "message": "no span"}},
            diagnostic("note", "clippy::x", "missing.rs", 4),
            diagnostic("warning", None, "missing.rs", 4),
        ],
    )
    target = tmp_path / "issues.json"

    assert convert(source, target) == (0, 0)
    assert read(target) == {"rules": [], "issues": []}


def test_falls_back_to_the_first_span(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "main.rs").write_text("fn main() {}\n")
    record = diagnostic("warning", "clippy::x", "main.rs", 3)
    record["message"]["spans"][0]["is_primary"] = False
    source = write(tmp_path, [record])
    target = tmp_path / "issues.json"

    assert convert(source, target) == (1, 1)
    assert read(target)["issues"][0]["primaryLocation"]["textRange"]["startLine"] == 3


def test_main_writes_the_report(tmp_path, monkeypatch, capsys):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "main.rs").write_text("fn main() {}\n")
    source = write(tmp_path, [diagnostic("warning", "clippy::x", "main.rs", 1)])
    target = tmp_path / "issues.json"

    assert main([str(source), str(target)]) == 0
    assert "1 rules" in capsys.readouterr().out
    assert read(target)["issues"]


def test_usage_error(tmp_path, capsys):
    assert main([]) == 2
    assert "usage" in capsys.readouterr().err
