"""Tests for the G-DEVLOOP-GODLIB shrink-only coupling ratchet."""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_repo_devloop_godlib as m  # noqa: E402


def test_baseline_passes_at_current_counts() -> None:
    current = {"install_defs": 51, "m_writes": 875, "package_core_installs": 188, "wildcard_exports": 1}
    baseline = dict(current)
    assert list(m.ratchet_messages(current, baseline)) == [], "current == baseline must pass (shrink-only)"


def test_shrink_passes() -> None:
    baseline = {"install_defs": 51, "m_writes": 875, "package_core_installs": 188, "wildcard_exports": 1}
    shrunk = {"install_defs": 40, "m_writes": 700, "package_core_installs": 150, "wildcard_exports": 1}
    assert list(m.ratchet_messages(shrunk, baseline)) == [], "shrinking below baseline must pass"


def test_growth_fails() -> None:
    baseline = {"install_defs": 51, "m_writes": 875, "package_core_installs": 188, "wildcard_exports": 1}
    grown = {"install_defs": 52, "m_writes": 875, "package_core_installs": 188, "wildcard_exports": 1}
    msgs = list(m.ratchet_messages(grown, baseline))
    assert any("install_defs 52 > baseline 51" in x for x in msgs), "a new install(M) must fail the ratchet"


def test_wildcard_growth_fails() -> None:
    baseline = {"install_defs": 0, "m_writes": 0, "package_core_installs": 0, "wildcard_exports": 0}
    grown = {"install_defs": 0, "m_writes": 0, "package_core_installs": 0, "wildcard_exports": 1}
    assert any("wildcard_exports 1 > baseline 0" in x for x in m.ratchet_messages(grown, baseline))


def test_missing_baseline_reports() -> None:
    msgs = list(m.ratchet_messages({"install_defs": 1, "m_writes": 0, "package_core_installs": 0, "wildcard_exports": 0}, None))
    assert msgs and "missing baseline" in msgs[0]


def test_live_repo_at_or_below_baseline() -> None:
    root = Path(__file__).resolve().parents[1]
    current = m.measure(root)
    baseline = m.load_baseline(root)
    assert baseline is not None, "committed baseline must exist"
    msgs = list(m.ratchet_messages(current, baseline))
    assert msgs == [], f"live repo must be at/below baseline (shrink-only); got: {msgs}"


def test_replayer_does_not_read_package_replayers_from_ambient_m() -> None:
    root = Path(__file__).resolve().parents[1]
    text = (root / "libraries" / "devloop" / "replayer.lua").read_text(encoding="utf-8")
    forbidden = [
        "M.replay_dependency_wait_state",
        "M.replay_ready_state",
        "M.replay_awaiting_pr_state",
        "M.install_pr_review_replayers",
    ]
    hits = [token for token in forbidden if token in text]
    assert hits == [], f"devloop.replayer must use package-provided registry, not ambient M: {hits}"


if __name__ == "__main__":
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]
    for fn in fns:
        fn()
        print(f"ok {fn.__name__}")
    print(f"PASS {len(fns)} tests")
