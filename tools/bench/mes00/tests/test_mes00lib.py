import json
import sys
import tempfile
from pathlib import Path

import pytest

import mes00lib as lib
import pgoptions_values as pgvalues


def test_percentile_interpolates_like_numpy():
    values = [1.0, 2.0, 3.0, 4.0]
    assert lib.percentile(values, 0) == 1.0
    assert lib.percentile(values, 50) == 2.5
    assert lib.percentile(values, 100) == 4.0
    assert lib.percentile(values, 25) == 1.75


def test_percentile_single_value_and_errors():
    assert lib.percentile([7.0], 99) == 7.0
    with pytest.raises(lib.Mes00Error):
        lib.percentile([], 50)
    with pytest.raises(lib.Mes00Error):
        lib.percentile([1.0], 101)


def test_summarize_reports_count_and_p99():
    summary = lib.summarize(range(1, 101))
    assert summary["n"] == 100
    assert summary["min"] == 1.0
    assert summary["max"] == 100.0
    assert summary["mean"] == 50.5
    assert summary["p50"] == 50.5
    assert summary["p99"] == 99.01


def test_prod_pgoptions_guard_requires_read_only(monkeypatch):
    monkeypatch.delenv("PROD_PGOPTIONS", raising=False)
    with pytest.raises(lib.Mes00Error, match="PROD_PGOPTIONS"):
        lib.require_prod_pgoptions()

    with pytest.raises(lib.Mes00Error, match="default_transaction_read_only=on"):
        lib.require_prod_pgoptions({"PROD_PGOPTIONS": pgvalues.NOT_READ_ONLY})

    value = pgvalues.VALID
    assert lib.require_prod_pgoptions({"PROD_PGOPTIONS": value}) == value


def test_prod_pgoptions_rejects_a_later_read_write_setting():
    with pytest.raises(lib.Mes00Error, match="default_transaction_read_only=on"):
        lib.require_prod_pgoptions({"PROD_PGOPTIONS": pgvalues.SEQUENTIAL_OFF})

    with pytest.raises(lib.Mes00Error, match="default_transaction_read_only=on"):
        lib.require_prod_pgoptions({"PROD_PGOPTIONS": pgvalues.SPACED_OFF})

    with pytest.raises(lib.Mes00Error, match="default_transaction_read_only=on"):
        lib.require_prod_pgoptions({"PROD_PGOPTIONS": pgvalues.ATTACHED_OFF})


def test_prod_pgoptions_returns_value_without_echoing_it():
    secret = pgvalues.VALID_WITH_APP_NAME
    assert lib.require_prod_pgoptions({"PROD_PGOPTIONS": secret}) == secret


def test_check_output_path_accepts_tmp_and_rejects_elsewhere():
    accepted = lib.check_output_path("/tmp/mes00-test.json")
    assert accepted.is_relative_to(Path("/tmp").resolve())

    with pytest.raises(lib.Mes00Error):
        lib.check_output_path("relative.json")
    with pytest.raises(lib.Mes00Error):
        lib.check_output_path("/etc/mes00-test.json")
    with pytest.raises(lib.Mes00Error):
        lib.check_output_path("/tmp/../etc/mes00-test.json")


def test_check_output_path_honours_forbidden_roots():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    target = root / "out.json"
    with pytest.raises(lib.Mes00Error, match="protected"):
        lib.check_output_path(target, allow_roots=[root], forbidden_roots=[root])


def test_check_output_path_requires_existing_allowed_root():
    with pytest.raises(lib.Mes00Error, match="existing directory"):
        lib.check_output_path(
            "/tmp/mes00.json", allow_roots=["/tmp/does-not-exist-mes00"]
        )


def test_find_repo_root_finds_checkout_from_a_nested_script():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    (root / ".git").mkdir()
    nested = root / "tools" / "bench" / "mes00"
    nested.mkdir(parents=True)
    assert lib.find_repo_root(nested / "script.py") == root.resolve()


def test_find_repo_root_returns_none_when_copied_to_tmp():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    nested = root / "mes00"
    nested.mkdir()
    assert lib.find_repo_root(nested / "script.py") is None


def test_resolve_project_dir_refuses_empty_relative_and_missing(monkeypatch):
    monkeypatch.delenv("PROD_REPO", raising=False)
    with pytest.raises(lib.Mes00Error, match="required"):
        lib.resolve_project_dir(None)
    with pytest.raises(lib.Mes00Error, match="required"):
        lib.resolve_project_dir("")
    with pytest.raises(lib.Mes00Error, match="absolute"):
        lib.resolve_project_dir("relative/path")
    with pytest.raises(lib.Mes00Error, match="existing"):
        lib.resolve_project_dir("/tmp/does-not-exist-mes00-project")


def test_resolve_project_dir_accepts_env(monkeypatch):
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    monkeypatch.setenv("PROD_REPO", str(root))
    assert lib.resolve_project_dir(None) == root


def test_write_and_read_json_roundtrip():
    root = Path(tempfile.mkdtemp(dir="/tmp"))
    target = root / "nested" / "data.json"
    lib.write_json(target, {"a": 1, "b": [2]})
    assert lib.read_json(target) == {"a": 1, "b": [2]}
    assert not list(target.parent.glob("*.tmp"))


def test_render_command_keeps_each_argument_whole():
    template = ["echo", "{query}", "{corpus}"]
    rendered = lib.render_command(template, {"query": "a; rm -rf /tmp/x", "corpus": "c"})
    assert rendered == ["echo", "a; rm -rf /tmp/x", "c"]


def test_render_command_rejects_unknown_placeholder():
    with pytest.raises(lib.Mes00Error, match="unknown"):
        lib.render_command(["echo", "{unknown}"], {})


def test_command_from_json_validates_shape():
    assert lib.command_from_json('["a", "b"]') == ["a", "b"]
    with pytest.raises(lib.Mes00Error):
        lib.command_from_json("{}")
    with pytest.raises(lib.Mes00Error):
        lib.command_from_json('["a", 1]')


def test_run_json_command_parses_surrounding_logs():
    script = (
        "import json,sys;"
        "print('log line');"
        "print(json.dumps({'ids': ['a']}))"
    )
    assert lib.run_json_command(
        [sys.executable, "-c", script], label="test"
    ) == {"ids": ["a"]}


def test_run_json_command_reports_failure_without_writing_output():
    script = "import sys; print('boom', file=sys.stderr); sys.exit(3)"
    with pytest.raises(lib.Mes00Error, match="exit code 3"):
        lib.run_json_command([sys.executable, "-c", script], label="test")


def test_child_env_exposes_pgoptions():
    script = "import json,os; print(json.dumps({'pgoptions': os.environ['PGOPTIONS']}))"
    value = pgvalues.VALID
    assert lib.run_json_command(
        [sys.executable, "-c", script],
        label="test",
        env=lib.child_env(value, base={}),
    ) == {"pgoptions": value}
