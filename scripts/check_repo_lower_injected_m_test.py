"""Tests for the G-LOWER-INJECTED-M shrink-only coupling ratchet."""
from __future__ import annotations

import json
import sys
import tempfile
import textwrap
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_repo_lower_injected_m as m  # noqa: E402


def lower_repo_current(library: str, source: str) -> dict:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        base = root / "libraries" / library
        base.mkdir(parents=True)
        (base / "installed.lua").write_text(textwrap.dedent(source), encoding="utf-8")
        return m.measure(root)


def lower_repo_messages(library: str, source: str, baseline_reads: int = 0) -> list[str]:
    current = lower_repo_current(library, source)
    baseline = {
        "workflow_injected_m_reads": 0,
        "workflow_injected_m_unique_symbols": 0,
        "forge_injected_m_reads": 0,
        "forge_injected_m_unique_symbols": 0,
        "total_injected_m_reads": 0,
        "total_injected_m_unique_symbols": 0,
    }
    baseline[f"{library}_injected_m_reads"] = baseline_reads
    baseline["total_injected_m_reads"] = baseline_reads
    manifest = {
        "workflow": {},
        "forge": {},
        library: {
            symbol: {"route": "typed_port", "owner": "test"}
            for symbol in current["symbols"][library]
        },
    }
    return list(m.ratchet_messages(current, baseline, manifest, m.current_symbols(current)))


def lower_repo_symbols(library: str, source: str) -> list[str]:
    return lower_repo_current(library, source)["symbols"][library]


def test_baseline_passes_at_current_counts() -> None:
    current = {
        "workflow_injected_m_reads": 10,
        "workflow_injected_m_unique_symbols": 4,
        "forge_injected_m_reads": 20,
        "forge_injected_m_unique_symbols": 8,
        "total_injected_m_reads": 30,
        "total_injected_m_unique_symbols": 12,
    }
    assert list(m.ratchet_messages(current, dict(current), {"workflow": {}, "forge": {}}, {})) == []


def test_growth_fails() -> None:
    baseline = {
        "workflow_injected_m_reads": 10,
        "workflow_injected_m_unique_symbols": 4,
        "forge_injected_m_reads": 20,
        "forge_injected_m_unique_symbols": 8,
        "total_injected_m_reads": 30,
        "total_injected_m_unique_symbols": 12,
    }
    grown = dict(baseline)
    grown["workflow_injected_m_reads"] = 11
    grown["total_injected_m_reads"] = 31

    msgs = list(m.ratchet_messages(grown, baseline, {"workflow": {}, "forge": {}}, {}))

    assert any("workflow_injected_m_reads 11 > baseline 10" in msg for msg in msgs)
    assert any("total_injected_m_reads 31 > baseline 30" in msg for msg in msgs)


def test_shrink_passes() -> None:
    baseline = {
        "workflow_injected_m_reads": 10,
        "workflow_injected_m_unique_symbols": 4,
        "forge_injected_m_reads": 20,
        "forge_injected_m_unique_symbols": 8,
        "total_injected_m_reads": 30,
        "total_injected_m_unique_symbols": 12,
    }
    shrunk = {
        "workflow_injected_m_reads": 9,
        "workflow_injected_m_unique_symbols": 3,
        "forge_injected_m_reads": 18,
        "forge_injected_m_unique_symbols": 7,
        "total_injected_m_reads": 27,
        "total_injected_m_unique_symbols": 10,
    }
    assert list(m.ratchet_messages(shrunk, baseline, {"workflow": {}, "forge": {}}, {})) == []


def test_missing_baseline_reports() -> None:
    msgs = list(m.ratchet_messages({}, None, {}, {}))
    assert msgs and "missing baseline" in msgs[0]


def test_scanner_finds_only_injected_m_reads() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        workflow = root / "libraries" / "workflow"
        workflow.mkdir(parents=True)
        (workflow / "local_module.lua").write_text(
            textwrap.dedent(
                """\
                local M = {}
                function M.local_api()
                  return M.local_helper()
                end
                return M
                """
            ),
            encoding="utf-8",
        )
        (workflow / "installed.lua").write_text(
            textwrap.dedent(
                """\
                local S = {}
                function S.install(M, resolved)
                  -- M.comment_call()
                  local literal = "M.string_call()"
                  function M.exported_api()
                    return M.live_call(M.indexed_value)
                  end
                end
                return S
                """
            ),
            encoding="utf-8",
        )

        current = m.measure(root)

    assert current["workflow_injected_m_reads"] == 3
    assert current["workflow_injected_m_unique_symbols"] == 3
    assert current["symbols"]["workflow"] == ["exported_api", "indexed_value", "live_call"]


def test_workflow_comparison_access_fails_growth_ratchet() -> None:
    msgs = lower_repo_messages(
        "workflow",
        """\
        local S = {}
        function S.install(M, resolved)
          if M.new_injected_hook == x then
            return true
          end
        end
        return S
        """,
    )

    assert any("workflow_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert any("total_injected_m_reads 1 > baseline 0" in msg for msg in msgs)


def test_non_install_m_parameter_helper_access_fails_growth_ratchet() -> None:
    source = """\
        local S = {}
        local function helper(M)
          return M.new_hook()
        end
        return S
        """
    msgs = lower_repo_messages("workflow", source)

    assert any("workflow_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert any("total_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert lower_repo_symbols("workflow", source) == ["new_hook"]


def test_module_table_m_without_parameter_is_not_injected_m() -> None:
    current = lower_repo_current(
        "workflow",
        """\
        local M = {}
        function M.local_api()
          return M.local_helper()
        end
        return M
        """,
    )

    assert current["workflow_injected_m_reads"] == 0
    assert current["symbols"]["workflow"] == []


def test_method_declaration_name_is_not_counted_as_injected_m() -> None:
    current = lower_repo_current(
        "workflow",
        """\
        local M = {}
        function M.uses_injected(M)
          if M.aaa then
            return 1
          elseif M.bbb then
            return 2
          end
        end
        return M
        """,
    )

    assert current["workflow_injected_m_reads"] == 2
    assert current["symbols"]["workflow"] == ["aaa", "bbb"]


def test_same_line_body_after_m_parameter_signature_is_counted() -> None:
    current = lower_repo_current(
        "workflow",
        """\
        local S = {}
        function S.f(M) return M.x end
        return S
        """,
    )

    assert current["workflow_injected_m_reads"] == 1
    assert current["symbols"]["workflow"] == ["x"]


def test_elseif_in_injected_m_function_does_not_leak_into_following_module_table() -> None:
    current = lower_repo_current(
        "workflow",
        """\
        local S = {}
        function S.install(M, value)
          if value == "a" then
            return M.first_hook()
          elseif value == "b" then
            return M.second_hook()
          else
            return M.third_hook()
          end
        end

        local M = {}
        function M.module_own()
          return M.local_helper()
        end
        return S
        """,
    )

    assert current["workflow_injected_m_reads"] == 3
    assert current["symbols"]["workflow"] == ["first_hook", "second_hook", "third_hook"]


def test_forge_bare_field_access_fails_growth_ratchet() -> None:
    msgs = lower_repo_messages(
        "forge",
        """\
        local S = {}
        function S.install(M)
          local v = M.new_injected_hook
          return v
        end
        return S
        """,
    )

    assert any("forge_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert any("total_injected_m_reads 1 > baseline 0" in msg for msg in msgs)


def test_workflow_bracket_string_access_fails_growth_ratchet() -> None:
    source = """\
        local S = {}
        function S.install(M)
          local v = M["new_hook"]
          return v
        end
        return S
        """
    msgs = lower_repo_messages("workflow", source)

    assert any("workflow_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert any("total_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert lower_repo_symbols("workflow", source) == ["new_hook"]


def test_workflow_single_quote_bracket_string_access_fails_growth_ratchet() -> None:
    source = """\
        local S = {}
        function S.install(M)
          local v = M['new_hook']
          return v
        end
        return S
        """
    msgs = lower_repo_messages("workflow", source)

    assert any("workflow_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert any("total_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert lower_repo_symbols("workflow", source) == ["new_hook"]


def test_forge_dynamic_bracket_access_fails_growth_ratchet() -> None:
    source = """\
        local S = {}
        function S.install(M)
          local dyn_key = "new_hook"
          local v = M[dyn_key]
          return v
        end
        return S
        """
    msgs = lower_repo_messages("forge", source)

    assert any("forge_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert any("total_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert lower_repo_symbols("forge", source) == ["<dynamic>"]


def test_call_access_fails_growth_ratchet() -> None:
    msgs = lower_repo_messages(
        "workflow",
        """\
        local S = {}
        function S.install(M)
          return M.new_injected_hook()
        end
        return S
        """,
    )

    assert any("workflow_injected_m_reads 1 > baseline 0" in msg for msg in msgs)


def test_method_access_fails_growth_ratchet() -> None:
    msgs = lower_repo_messages(
        "workflow",
        """\
        local S = {}
        function S.install(M)
          return M:new_injected_hook()
        end
        return S
        """,
    )

    assert any("workflow_injected_m_reads 1 > baseline 0" in msg for msg in msgs)


def test_rebind_rhs_access_fails_growth_ratchet() -> None:
    source = """\
        local S = {}
        function S.install(M)
          local M = M.new_hook
          return M
        end
        return S
        """
    msgs = lower_repo_messages("workflow", source)

    assert any("workflow_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert any("total_injected_m_reads 1 > baseline 0" in msg for msg in msgs)
    assert lower_repo_symbols("workflow", source) == ["new_hook"]


def test_install_signature_comment_string_and_rebind_are_not_counted() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        workflow = root / "libraries" / "workflow"
        workflow.mkdir(parents=True)
        (workflow / "installed.lua").write_text(
            textwrap.dedent(
                """\
                local S = {}
                function S.install(M, resolved)
                  -- M.comment_hook
                  -- M["comment_hook"]
                  local literal = "M.string_hook"
                  local bracket_literal = "M['string_hook']"
                  local block = [[ M.long_string_hook ]]
                  local bracket_block = [[ M["long_string_hook"] ]]
                  local M = { rebound_hook = true }
                  local _M = { ignored_hook = true }
                end
                return S
                """
            ),
            encoding="utf-8",
        )

        current = m.measure(root)

    assert current["workflow_injected_m_reads"] == 0
    assert current["symbols"]["workflow"] == []


def test_unmanifested_symbol_fails() -> None:
    current = {
        "workflow_injected_m_reads": 1,
        "workflow_injected_m_unique_symbols": 1,
        "forge_injected_m_reads": 0,
        "forge_injected_m_unique_symbols": 0,
        "total_injected_m_reads": 1,
        "total_injected_m_unique_symbols": 1,
        "symbols": {"workflow": ["new_hook"], "forge": []},
    }
    baseline = {
        "workflow_injected_m_reads": 1,
        "workflow_injected_m_unique_symbols": 1,
        "forge_injected_m_reads": 0,
        "forge_injected_m_unique_symbols": 0,
        "total_injected_m_reads": 1,
        "total_injected_m_unique_symbols": 1,
    }
    msgs = list(m.ratchet_messages(current, baseline, {"workflow": {}, "forge": {}}, m.current_symbols(current)))
    assert any("workflow M.new_hook has no declared route" in msg for msg in msgs)


def test_repository_noops_when_lower_libraries_absent() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        assert list(m.repository_messages(Path(tmp))) == []


def test_live_repo_at_or_below_baseline() -> None:
    root = Path(__file__).resolve().parents[1]
    current = m.measure(root)
    baseline, manifest = m.load_inventory(root)
    assert baseline is not None, "committed baseline must exist"
    msgs = list(m.ratchet_messages(current, baseline, manifest, m.current_symbols(current)))
    assert msgs == [], f"live repo must be at/below baseline and fully routed; got: {msgs}"


def test_inventory_json_is_loadable() -> None:
    root = Path(__file__).resolve().parents[1]
    path = root / m.INVENTORY
    assert isinstance(json.loads(path.read_text(encoding="utf-8")), dict)


if __name__ == "__main__":
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]
    for fn in fns:
        fn()
        print(f"ok {fn.__name__}")
    print(f"PASS {len(fns)} tests")
