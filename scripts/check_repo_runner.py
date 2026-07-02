#!/usr/bin/env python3
"""Orchestrate generic and fkst-packages-specific repository ratchets."""

from __future__ import annotations

import check_repo_config
import check_repo_content_truncation
import check_repo_coverage
import check_repo_devloop_godlib
import check_repo_devloop_decouple
import check_repo_devloop_installer
import check_repo_hidden_state
import check_repo_intake_default_surface
import check_repo_intake_routing
import check_repo_integration_coverage
import check_repo_lower_injected_m
import check_repo_monotone_gate
import check_repo_namespaced_queue
import check_repo_producer_liveness
import check_repo_saga_head
import check_repo_saga_split


def check_content_truncation(c, root, violations, allowlist_dir=None, enforce_base=True) -> None:
    sources = {}
    for package_root in c.package_roots(root):
        sources.update(check_repo_content_truncation.package_lua_sources(root, package_root, c.read_text, c.rel))
    current = check_repo_content_truncation.sites(
        sources
    )
    allowlist = check_repo_content_truncation.load_allowlist(
        c.allowlist_path(root, check_repo_content_truncation.ALLOWLIST, allowlist_dir)
    )
    base_status, base_allowlist = check_repo_content_truncation.allowlist_at_dev_base(root) if enforce_base else ("absent", None)
    if base_status == "unresolved":
        c.add(violations, "G-CONTENT-TRUNCATION", "cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    for message in check_repo_content_truncation.ratchet_messages(current, allowlist, base_allowlist):
        c.add(violations, "G-CONTENT-TRUNCATION", message)


def check_producer_liveness(c, root, violations, allowlist_dir=None, enforce_base=True) -> None:
    package_roots = c.package_roots(root)
    raisers = set().union(*[
        check_repo_producer_liveness.declared_raisers(root, package_root)
        for package_root in package_roots
    ])
    fixture_coverage = {
        package.name: check_repo_producer_liveness.package_test_fixture_coverage(package)
        for package_root in package_roots
        if package_root.exists()
        for package in sorted(package_root.iterdir())
        if package.is_dir()
    }
    coverage = {
        package: set().union(*by_fixture.values()) if by_fixture else set()
        for package, by_fixture in fixture_coverage.items()
    }
    allowlist = check_repo_producer_liveness.load_allowlist(
        c.allowlist_path(root, check_repo_producer_liveness.ALLOWLIST, allowlist_dir)
    )
    base_status, base_allowlist = check_repo_producer_liveness.allowlist_at_dev_base(root) if enforce_base else ("absent", None)
    if base_status == "unresolved":
        c.add(violations, "G-PRODUCER-LIVENESS", "cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    messages = check_repo_producer_liveness.ratchet_messages(
        raisers,
        coverage,
        allowlist,
        base_allowlist,
        fixture_coverage,
        set().union(*[
            check_repo_producer_liveness.declared_liveness_contracts(root, package_root)
            for package_root in package_roots
        ]),
    )
    for message in messages:
        c.add(violations, "G-PRODUCER-LIVENESS", message)


def check_monotone_gate(c, root, violations, allowlist_dir=None, enforce_base=True) -> None:
    package_roots = c.package_roots(root)
    if not enforce_base and not check_repo_monotone_gate.production_sources(root, package_roots):
        return
    current, messages = check_repo_monotone_gate.current_violations(root, package_roots)
    for message in messages:
        c.add(violations, "G-MONOTONE-GATE", message)
    allowlist = check_repo_monotone_gate.load_allowlist(
        c.allowlist_path(root, check_repo_monotone_gate.ALLOWLIST, allowlist_dir)
    )
    base_status, base_allowlist = check_repo_monotone_gate.allowlist_at_dev_base(root) if enforce_base else ("absent", None)
    if base_status == "unresolved":
        c.add(violations, "G-MONOTONE-GATE", "cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    for message in check_repo_monotone_gate.ratchet_messages(current, allowlist, base_allowlist):
        c.add(violations, "G-MONOTONE-GATE", message)


def run_generic(c, config: check_repo_config.CheckRepoConfig, violations: list[str], warnings: list[str]) -> None:
    root = config.project_root
    allowlists = config.allowlist_dir
    enforce_base = config.is_own_repo
    c.check_line_limit(root, violations, warnings); c.check_test_shape(root, violations, warnings)
    c.check_helper_reachability(root, violations); c.check_graphql_connection_guards(root, warnings)
    c.check_rest_pagination_guards(root, warnings); c.check_hidden_text_encoded_literals(root, violations)
    c.check_gh_rate_pool_sizing(root, violations); c.check_error_class_prefixes(root, warnings)
    c.check_persistence_classes(root, violations); c.check_cross_package_require(root, violations)
    for package_root in c.package_roots(root):
        for message in c.check_repo_ingress.scoped_file_watch_ingress_messages(root, package_root, c.read_text, c.rel):
            c.add(violations, "G13", message)
    c.check_no_permission_control(root, violations)
    c.check_gh_git_adapter_ratchet(root, violations, allowlists)
    c.check_shell_out_to_self_ratchet(root, violations, allowlists)
    c.check_code_dedup_ratchet(root, violations, allowlists, enforce_base)
    check_content_truncation(c, root, violations, allowlists, enforce_base)
    for message in check_repo_coverage.repository_messages(root):
        c.add(violations, "G-COVERAGE", message)
    integration_allowlist = None
    integration_exclusions = None
    if config.platform_root is not None:
        integration_allowlist = root / check_repo_integration_coverage.HOST_ALLOWLIST
        integration_exclusions = root / check_repo_integration_coverage.HOST_EXCLUSIONS
    for message in check_repo_integration_coverage.repository_messages(
        root,
        platform_root=config.platform_root,
        allowlist_path=integration_allowlist,
        exclusions_path=integration_exclusions,
    ):
        c.add(violations, "G-INTEGRATION-COVERAGE", message)
    check_producer_liveness(c, root, violations, allowlists, enforce_base)
    for package_root in c.package_roots(root):
        for message in check_repo_namespaced_queue.repository_messages(root, package_root, c.read_text, c.rel, c.strip_lua_comments_and_strings, c.is_unmasked_range):
            c.add(violations, "G-NAMESPACED-QUEUE", message)
    check_monotone_gate(c, root, violations, allowlists, enforce_base)
    c.check_saga_handler_ratchet(root, violations, warnings, allowlists, enforce_base)
    sources = {c.rel(root, path): c.read_text(path) for package_root in c.package_roots(root) for path in sorted(package_root.glob("*/departments/*/main.lua")) if path.is_file()}
    for message in check_repo_saga_head.violations(sources, c.strip_lua_comments_and_strings):
        c.add(violations, "G-SAGA-HEAD", message)


def run_library_b_specific(c, config: check_repo_config.CheckRepoConfig, violations: list[str], warnings: list[str]) -> None:
    root = config.project_root
    c.check_ownership_gate_claim_owner(root, violations)
    c.check_std_dependency_model(root, violations, warnings)
    if (root / ".claude/skills/dogfood-github-devloop/dogfood.sh").exists():
        __import__("check_repo_dogfood_boundary").check(root, violations, c.add)
    for message in check_repo_saga_split.repository_messages(root):
        c.add(violations, "G-SAGA-SPLIT", message)
    for message in check_repo_hidden_state.repository_messages(root, config.allowlist_dir, config.is_own_repo):
        c.add(violations, "G-HIDDEN-STATE", message)
    for message in check_repo_intake_default_surface.repository_messages(root):
        c.add(violations, "G-INTAKE-DEFAULT-SURFACE", message)
    for message in check_repo_intake_routing.repository_messages(root):
        c.add(violations, "G-INTAKE-ROUTING", message)
    for message in check_repo_devloop_godlib.repository_messages(root):
        c.add(violations, "G-DEVLOOP-GODLIB", message)
    for message in check_repo_lower_injected_m.repository_messages(root):
        c.add(violations, "G-LOWER-INJECTED-M", message)
    for message in check_repo_devloop_decouple.repository_messages(root):
        c.add(violations, "G-DEVLOOP-DECOUPLE", message)
    for message in check_repo_devloop_installer.repository_messages(root):
        c.add(violations, "G-DEVLOOP-INSTALLER", message)


def run(c, config: check_repo_config.CheckRepoConfig, violations: list[str], warnings: list[str]) -> None:
    run_generic(c, config, violations, warnings)
    if config.is_own_repo:
        run_library_b_specific(c, config, violations, warnings)
    else:
        print(f"OK: skipped library-B-specific ratchets for external project root: {config.project_root}")
