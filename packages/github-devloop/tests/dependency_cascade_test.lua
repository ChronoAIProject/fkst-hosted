local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local repo = "owner/repo"
local proposal_id = "github-devloop/issue/owner/repo/42"
local version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#issue/42",
  }
end

local function json_string(value)
  return tostring(value)
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
end

local function render_comment(body)
  return string.format(
    '{"body":"%s","author":{"login":"fkst-test-bot"},"createdAt":"2026-06-03T01:00:00Z"}',
    json_string(body or "")
  )
end

local function issue_comments_json(comments)
  local rendered = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered, render_comment(comment))
  end
  return table.concat(rendered, ",")
end

local function issue_view_json(labels, comments, state)
  local rendered_labels = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered_labels, string.format('{"name":"%s"}', json_string(label)))
  end
  return string.format(
    '{"title":"Implement dependency cascade","state":"%s","labels":[%s],"comments":[%s]}\n',
    json_string(state or "OPEN"),
    table.concat(rendered_labels, ","),
    issue_comments_json(comments)
  )
end

local function observe_issue_state_json(labels, comments, state)
  local rendered_labels = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered_labels, string.format('{"name":"%s"}', json_string(label)))
  end
  return string.format(
    '{"state":"%s","labels":[%s],"comments":[%s]}\n',
    json_string(state or "OPEN"),
    table.concat(rendered_labels, ","),
    issue_comments_json(comments)
  )
end

local function blocked_by_json(nodes)
  local rendered = {}
  for _, node in ipairs(nodes or {}) do
    table.insert(rendered, string.format(
      '{"number":%s,"state":"%s","repository":{"nameWithOwner":"%s"}}',
      tostring(node.number),
      json_string(node.state or "OPEN"),
      json_string(node.repo or repo)
    ))
  end
  return '{"data":{"repository":{"issue":{"blockedBy":{"nodes":[' .. table.concat(rendered, ",") .. ']}}}}}\n'
end

local function mock_blocked_by(issue_number, nodes)
  t.mock_command(core.gh_blocked_by_cmd(repo, issue_number), {
    stdout = blocked_by_json(nodes),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_blocked_by_failure(issue_number)
  t.mock_command(core.gh_blocked_by_cmd(repo, issue_number), {
    stdout = "",
    stderr = "graphql failed",
    exit_code = 1,
  })
end

local function mock_blocked_by_malformed(issue_number)
  t.mock_command(core.gh_blocked_by_cmd(repo, issue_number), {
    stdout = "{",
    stderr = "",
    exit_code = 0,
  })
end

-- gh succeeds but the blockedBy list is truncated (more blockers than the page
-- returns). An unseen unmet blocker must fail-closed, never read as absent.
local function mock_blocked_by_truncated(issue_number)
  t.mock_command(core.gh_blocked_by_cmd(repo, issue_number), {
    stdout = '{"data":{"repository":{"issue":{"blockedBy":{"totalCount":51,"pageInfo":{"hasNextPage":true},"nodes":[{"number":7,"state":"CLOSED","repository":{"nameWithOwner":"' .. repo .. '"}}]}}}}}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_blocker_issue(issue_number, state_name)
  local comments = {}
  if state_name ~= nil then
    table.insert(comments, core.state_marker(core.proposal_id(repo, issue_number), state_name, "v-" .. tostring(issue_number)))
  end
  t.mock_command(core.gh_issue_view_observe_cmd(repo, issue_number), {
    stdout = '{"state":"OPEN","comments":[' .. issue_comments_json(comments) .. ']}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_blocker_issue_with_pr_link(issue_number, pr_number, state_name)
  local blocker_proposal_id = core.proposal_id(repo, issue_number)
  local branch = "devloop-owner-repo-" .. tostring(issue_number) .. "-01HY"
  local impl_version = "v-" .. tostring(issue_number)
  local comments = {}
  if state_name ~= nil then
    table.insert(comments, core.state_marker(blocker_proposal_id, state_name, impl_version))
  end
  table.insert(comments, core.pr_link_marker(blocker_proposal_id, pr_number, branch, impl_version, "dev"))
  t.mock_command(core.gh_issue_view_observe_cmd(repo, issue_number), {
    stdout = '{"state":"OPEN","comments":[' .. issue_comments_json(comments) .. ']}\n',
    stderr = "",
    exit_code = 0,
  })
  return {
    proposal_id = blocker_proposal_id,
    branch = branch,
    impl_version = impl_version,
    base_branch = "dev",
  }
end

local function mock_blocker_pr(issue_number, pr_number, link, comments)
  local rendered_comments = comments or {
    core.pr_origin_marker(link.proposal_id, issue_number, link.branch, link.impl_version, link.base_branch),
  }
  t.mock_command(core.gh_pr_view_observe_cmd(repo, pr_number), {
    stdout = '{"headRefName":"' .. json_string(link.branch)
      .. '","headRefOid":"abc123","baseRefName":"' .. json_string(link.base_branch)
      .. '","state":"MERGED","comments":[' .. issue_comments_json(rendered_comments) .. ']}\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_blocker_pr_failure(pr_number)
  t.mock_command(core.gh_pr_view_observe_cmd(repo, pr_number), {
    stdout = "",
    stderr = "pr view failed",
    exit_code = 1,
  })
end

local function mock_result_issue(labels, comments)
  t.mock_command(core.gh_issue_view_result_cmd(repo, 42), {
    stdout = issue_view_json(labels or { "fkst-dev:thinking" }, comments or {
      core.state_marker(proposal_id, "thinking", "2026-06-02T00-00-00Z"),
    }),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_observe_issue(labels, comments)
  t.mock_command(core.gh_issue_view_entity_cmd(repo, 42), {
    stdout = issue_view_json(labels or { "fkst-dev:enabled", "fkst-dev:ready" }, comments or {
      core.state_marker(proposal_id, "ready", version),
    }),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_implement_issue(labels, comments)
  t.mock_command(core.gh_issue_view_implement_cmd(repo, 42), {
    stdout = issue_view_json(labels or { "fkst-dev:ready" }, comments or {
      core.state_marker(proposal_id, "ready", h.ready().dedup_key),
    }),
    stderr = "",
    exit_code = 0,
  })
end

local function reached()
  return {
    schema = "consensus.consensus_reached.v1",
    proposal_id = proposal_id,
    decision = "approve",
    body = "Approved.",
    dedup_key = version,
    source_ref = source_ref(),
  }
end

local function run_result()
  return t.run_department("departments/consensus_result/main.lua", {
    queue = "consensus.consensus_reached",
    payload = reached(),
  }, h.opts("dependency-result"))
end

local function run_observe()
  return t.run_department("departments/observe_issue/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = h.issue(),
  }, h.opts("dependency-observe"))
end

local function run_implement()
  return t.run_department("departments/implement/main.lua", {
    queue = "devloop_ready",
    payload = h.ready(),
  }, h.opts("dependency-implement"))
end

local function find_raise(raises, queue, predicate)
  for _, item in ipairs(raises or {}) do
    if item.queue == queue and (predicate == nil or predicate(item.payload)) then
      return item
    end
  end
  return nil
end

local function has_queue(raises, queue)
  return find_raise(raises, queue) ~= nil
end

local function count_queue(raises, queue)
  local count = 0
  for _, item in ipairs(raises or {}) do
    if item.queue == queue then
      count = count + 1
    end
  end
  return count
end

local function has_marker(raises, marker_text)
  return find_raise(raises, "github-proxy.github_issue_comment_request", function(payload)
    return tostring(payload.body or ""):find(marker_text, 1, true) ~= nil
  end) ~= nil
end

return {
  test_dependency_gate_satisfied_without_blockers = function()
    mock_blocked_by(42, {})
    local gate = core.dependency_gate(repo, 42)
    t.eq(gate.ok, true)
    t.eq(gate.kind, "satisfied")
  end,

  test_dependency_markers_are_versioned_and_bounded = function()
    t.eq(
      core.dependency_wait_marker(proposal_id, "v1", { 1, 2, 3 }),
      '<!-- fkst:github-devloop:dependency-wait:v1 proposal="github-devloop/issue/owner/repo/42" version="v1" hold_kind="waiting" reason="waiting-on-dependency" unmet="1,2,3" -->'
    )
    t.eq(
      core.dependency_cycle_marker(proposal_id, "v1"),
      '<!-- fkst:github-devloop:dependency-cycle:v1 proposal="github-devloop/issue/owner/repo/42" version="v1" -->'
    )
    t.eq(
      core.dependency_unresolvable_marker(proposal_id, "v1", { 1, 2, 3 }),
      '<!-- fkst:github-devloop:dependency-unresolvable:v1 proposal="github-devloop/issue/owner/repo/42" version="v1" hold_kind="unresolvable" reason="gh-failed" unmet="1,2,3" -->'
    )
    t.eq(
      core.dependency_release_marker(proposal_id, "v1"),
      '<!-- fkst:github-devloop:dependency-release:v1 proposal="github-devloop/issue/owner/repo/42" version="v1" -->'
    )
  end,

  test_dependency_gate_waiting_for_open_blocker = function()
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, {})
    mock_blocker_issue(7, "ready")
    local gate = core.dependency_gate(repo, 42)
    t.eq(gate.ok, false)
    t.eq(gate.kind, "waiting")
    t.eq(gate.unmet[1], 7)
  end,

  test_dependency_gate_satisfied_for_merged_blocker = function()
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, {})
    mock_blocker_issue(7, "merged")
    local gate = core.dependency_gate(repo, 42)
    t.eq(gate.ok, true)
    t.eq(gate.kind, "satisfied")
  end,

  test_dependency_gate_satisfied_for_pr_stream_merged_blocker = function()
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, {})
    local link = mock_blocker_issue_with_pr_link(7, 8, "pr-open")
    mock_blocker_pr(7, 8, link, {
      core.pr_origin_marker(link.proposal_id, 7, link.branch, link.impl_version, link.base_branch),
      core.state_marker(link.proposal_id, "merged", "merge-version-7"),
      core.merged_marker(link.proposal_id, 8, "merge-version-7", "def456"),
    })
    local gate = core.dependency_gate(repo, 42)
    t.eq(gate.ok, true)
    t.eq(gate.kind, "satisfied")
  end,

  test_dependency_gate_waits_when_linked_pr_has_no_merged_fact = function()
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, {})
    local link = mock_blocker_issue_with_pr_link(7, 8, "pr-open")
    mock_blocker_pr(7, 8, link, {
      core.pr_origin_marker(link.proposal_id, 7, link.branch, link.impl_version, link.base_branch),
      core.state_marker(link.proposal_id, "merge-ready", "merge-version-7"),
    })
    local gate = core.dependency_gate(repo, 42)
    t.eq(gate.ok, false)
    t.eq(gate.kind, "waiting")
    t.eq(gate.unmet[1], 7)
  end,

  test_dependency_gate_pr_stream_fetch_failure_fails_closed = function()
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, {})
    mock_blocker_issue_with_pr_link(7, 8, "pr-open")
    mock_blocker_pr_failure(8)
    local gate = core.dependency_gate(repo, 42)
    t.eq(gate.ok, false)
    t.eq(gate.kind, "unresolvable")
    t.eq(gate.unmet[1], 7)
  end,

  test_dependency_gate_cycle = function()
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, { { number = 42 } })
    local gate = core.dependency_gate(repo, 42)
    t.eq(gate.ok, false)
    t.eq(gate.kind, "cycle")
  end,

  test_dependency_gate_cross_repo_and_failures_unresolvable = function()
    mock_blocked_by(42, { { number = 7, repo = "other/repo" } })
    local cross_repo = core.dependency_gate(repo, 42)
    t.eq(cross_repo.ok, false)
    t.eq(cross_repo.kind, "unresolvable")

    mock_blocked_by_failure(42)
    local failed = core.dependency_gate(repo, 42)
    t.eq(failed.ok, false)
    t.eq(failed.kind, "unresolvable")

    mock_blocked_by_malformed(42)
    local malformed = core.dependency_gate(repo, 42)
    t.eq(malformed.ok, false)
    t.eq(malformed.kind, "unresolvable")
  end,

  test_dependency_gate_truncated_blockedby_fails_closed = function()
    -- 51 blockers exist but the page returns 1 (merged); the unseen 50 must not
    -- be read as absent. The gate must fail-closed, NOT return ok=true.
    mock_blocked_by_truncated(42)
    local gate = core.dependency_gate(repo, 42)
    t.eq(gate.ok, false)
    t.eq(gate.kind, "unresolvable")
  end,

  test_consensus_result_holds_for_unmet_dependency = function()
    mock_result_issue()
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, {})
    mock_blocker_issue(7, "ready")
    local result = run_result()
    t.eq(result.exit_code, 0)
    t.eq(has_queue(result.raises, "devloop_ready"), false)
    t.is_true(has_marker(result.raises, "fkst:github-devloop:dependency-wait:v1"))
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request", function(payload)
      return h.has_value(payload.add_labels, "fkst-dev:blocked-on-dependency")
    end)
    t.is_true(label ~= nil)
  end,

  test_consensus_result_raises_ready_for_satisfied_dependency = function()
    mock_result_issue()
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, {})
    mock_blocker_issue(7, "merged")
    local result = run_result()
    t.eq(result.exit_code, 0)
    t.is_true(has_queue(result.raises, "devloop_ready"))
  end,

  test_observe_issue_ready_holds_then_cascades_when_satisfied = function()
    mock_observe_issue()
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, {})
    mock_blocker_issue(7, "ready")
    local held = run_observe()
    t.eq(held.exit_code, 0)
    t.eq(has_queue(held.raises, "devloop_ready"), false)
    t.is_true(has_marker(held.raises, "fkst:github-devloop:dependency-wait:v1"))

    mock_observe_issue(
      { "fkst-dev:enabled", "fkst-dev:ready", "fkst-dev:blocked-on-dependency" },
      {
        core.state_marker(proposal_id, "ready", version),
        "github-devloop dependency hold: waiting\n\nReason: waiting-on-dependency\n\n"
          .. core.dependency_wait_marker(proposal_id, version, { 7 }),
      }
    )
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, {})
    mock_blocker_issue(7, "merged")
    local cascaded = run_observe()
    t.eq(cascaded.exit_code, 0)
    t.is_true(has_queue(cascaded.raises, "devloop_ready"))
    t.is_true(has_marker(cascaded.raises, "fkst:github-devloop:dependency-release:v1"))
    local clear = find_raise(cascaded.raises, "github-proxy.github_issue_label_request", function(payload)
      return h.has_value(payload.remove_labels, "fkst-dev:blocked-on-dependency")
    end)
    t.is_true(clear ~= nil)
  end,

  test_observe_issue_existing_hold_still_waiting_does_not_refresh = function()
    mock_observe_issue(
      { "fkst-dev:enabled", "fkst-dev:ready", "fkst-dev:blocked-on-dependency" },
      {
        core.state_marker(proposal_id, "ready", version),
        "github-devloop dependency hold: waiting\n\nReason: waiting-on-dependency\n\n"
          .. core.dependency_wait_marker(proposal_id, version, { 7 }),
      }
    )
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, {})
    mock_blocker_issue(7, "ready")
    local result = run_observe()
    t.eq(result.exit_code, 0)
    t.eq(count_queue(result.raises, "github-proxy.github_issue_comment_request"), 0)
    t.eq(count_queue(result.raises, "github-proxy.github_issue_label_request"), 0)
    t.eq(has_queue(result.raises, "devloop_ready"), false)
  end,

  test_cycle_holds_with_cycle_marker = function()
    mock_result_issue()
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, { { number = 42 } })
    local result = run_result()
    t.eq(result.exit_code, 0)
    t.eq(has_queue(result.raises, "devloop_ready"), false)
    t.is_true(has_marker(result.raises, "fkst:github-devloop:dependency-cycle:v1"))
  end,

  test_unresolvable_holds_fail_closed = function()
    mock_result_issue()
    mock_blocked_by_malformed(42)
    local result = run_result()
    t.eq(result.exit_code, 0)
    t.eq(has_queue(result.raises, "devloop_ready"), false)
    t.is_true(has_marker(result.raises, "fkst:github-devloop:dependency-unresolvable:v1"))
  end,

  test_dependency_hold_fact_reads_marker_semantics_not_prose = function()
    local gh_failed = core.dependency_hold_fact({
      core.state_marker(proposal_id, "ready", version),
      "localized prose and arbitrary reason noise\n\n"
        .. core.dependency_unresolvable_marker(proposal_id, version, { 42 }, "unresolvable", "gh-failed"),
    }, proposal_id)
    t.eq(gh_failed.marker_kind, "dependency-unresolvable")
    t.eq(gh_failed.hold_kind, "unresolvable")
    t.eq(gh_failed.reason, "gh-failed")

    local old_gh_failed = core.dependency_hold_fact({
      core.state_marker(proposal_id, "ready", version),
      "github-devloop dependency hold: unresolvable\n\nReason: gh-failed\n\n"
        .. core.dependency_wait_marker(proposal_id, version, { 42 }),
    }, proposal_id)
    t.eq(old_gh_failed.marker_kind, "dependency-wait")
    t.eq(old_gh_failed.hold_kind, "waiting")
    t.eq(old_gh_failed.reason, "waiting-on-dependency")

    local attr_gh_failed = core.dependency_hold_fact({
      core.state_marker(proposal_id, "ready", version),
      "localized prose and arbitrary reason noise\n\n"
        .. core.dependency_wait_marker(proposal_id, version, { 42 }, "unresolvable", "gh-failed"),
    }, proposal_id)
    t.eq(attr_gh_failed.marker_kind, "dependency-wait")
    t.eq(attr_gh_failed.hold_kind, "unresolvable")
    t.eq(attr_gh_failed.reason, "gh-failed")

    local cycle = core.dependency_hold_fact({
      core.state_marker(proposal_id, "ready", version),
      "localized prose and arbitrary reason noise\n\n"
        .. core.dependency_cycle_marker(proposal_id, version),
    }, proposal_id)
    t.eq(cycle.marker_kind, "dependency-cycle")
    t.eq(cycle.reason, "dependency-cycle")
  end,

  test_gh_failed_hold_rechecks_and_releases_on_next_poll = function()
    mock_observe_issue()
    mock_blocked_by_failure(42)
    local held = run_observe()
    t.eq(held.exit_code, 0)
    t.eq(has_queue(held.raises, "devloop_ready"), false)
    t.is_true(has_marker(held.raises, 'hold_kind="unresolvable"'))
    t.is_true(has_marker(held.raises, 'reason="gh-failed"'))

    mock_observe_issue(
      { "fkst-dev:enabled", "fkst-dev:ready", "fkst-dev:blocked-on-dependency" },
      {
        core.state_marker(proposal_id, "ready", version),
        "github-devloop dependency hold: unresolvable\n\nReason: gh-failed\n\n"
          .. core.dependency_unresolvable_marker(proposal_id, version, { 42 }),
      }
    )
    mock_blocked_by(42, {})
    local released = run_observe()
    t.eq(released.exit_code, 0)
    t.is_true(has_queue(released.raises, "devloop_ready"))
    t.eq(find_raise(released.raises, "devloop_ready").payload.dedup_key, core._dedup_key({ "ready", version }))
    t.is_true(has_marker(released.raises, "fkst:github-devloop:dependency-release:v1"))
    local clear = find_raise(released.raises, "github-proxy.github_issue_label_request", function(payload)
      return h.has_value(payload.remove_labels, "fkst-dev:blocked-on-dependency")
    end)
    t.is_true(clear ~= nil)
  end,

  test_old_gh_failed_wait_hold_rechecks_and_releases_on_next_poll = function()
    mock_observe_issue(
      { "fkst-dev:enabled", "fkst-dev:ready", "fkst-dev:blocked-on-dependency" },
      {
        core.state_marker(proposal_id, "ready", version),
        "github-devloop dependency hold: unresolvable\n\nReason: gh-failed\n\n"
          .. core.dependency_wait_marker(proposal_id, version, { 42 }),
      }
    )
    mock_blocked_by(42, {})
    local released = run_observe()
    t.eq(released.exit_code, 0)
    t.is_true(has_queue(released.raises, "devloop_ready"))
    t.eq(find_raise(released.raises, "devloop_ready").payload.dedup_key, core._dedup_key({ "ready", version }))
    t.is_true(has_marker(released.raises, "fkst:github-devloop:dependency-release:v1"))
  end,

  test_non_hold_state_clears_stale_dependency_label = function()
    mock_observe_issue(
      { "fkst-dev:enabled", "fkst-dev:implementing", "fkst-dev:blocked-on-dependency" },
      {
        core.state_marker(proposal_id, "implementing", "ready-consensus-github-devloop-issue-owner-repo-42-2026-06-03T01-02-03Z"),
        "github-devloop dependency hold: waiting\n\nReason: waiting-on-dependency\n\n"
          .. core.dependency_wait_marker(proposal_id, version, { 7 }),
      }
    )
    local result = run_observe()
    t.eq(result.exit_code, 0)
    local clear = find_raise(result.raises, "github-proxy.github_issue_label_request", function(payload)
      return h.has_value(payload.remove_labels, "fkst-dev:blocked-on-dependency")
    end)
    t.is_true(clear ~= nil)
    t.eq(has_queue(result.raises, "devloop_ready"), false)
  end,

  test_implement_backstop_returns_without_implementing = function()
    mock_blocked_by(42, { { number = 7 } })
    mock_blocked_by(7, {})
    mock_blocker_issue(7, "ready")
    mock_implement_issue()
    local result = run_implement()
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_no_blockers_unaffected = function()
    mock_result_issue()
    mock_blocked_by(42, {})
    local result = run_result()
    t.eq(result.exit_code, 0)
    t.is_true(has_queue(result.raises, "devloop_ready"))
  end,
}
