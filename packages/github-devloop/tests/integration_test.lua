local t = fkst.test
local core = require("core")

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function runtime_root(name)
  return "/tmp/fkst-packages-test/github-devloop/" .. tostring(now()) .. "/" .. nonce() .. "/" .. name
end

local function opts(name)
  return {
    env = {
      FKST_RUNTIME_ROOT = runtime_root(name),
    },
  }
end

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#issue/42",
  }
end

local function issue(extra)
  local value = {
    schema = "github-proxy.v1",
    type = "issue",
    repo = "owner/repo",
    number = 42,
    title = "Implement decision recorder",
    url = "https://github.example/owner/repo/issues/42",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    labels = { "fkst-dev:enabled" },
    dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
    source_ref = source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function reached(extra)
  local value = {
    schema = "consensus.consensus_reached.v1",
    proposal_id = "github-devloop/issue/owner/repo/42",
    decision = "approve",
    body = "All angles approve.",
    dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    source_ref = source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function unresolved(extra)
  local value = {
    schema = "consensus.consensus_unresolved.v1",
    proposal_id = "github-devloop/issue/owner/repo/42",
    dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    source_ref = source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function run_observe(payload, run_opts)
  return t.run_department("departments/observe_issue/main.lua", {
    queue = "github-proxy.github_entity_changed",
    payload = payload,
  }, run_opts)
end

local function run_result(payload, run_opts)
  return t.run_department("departments/consensus_result/main.lua", {
    queue = "consensus.consensus_reached",
    payload = payload,
  }, run_opts)
end

local function run_loop(payload, run_opts)
  return t.run_department("departments/loop/main.lua", {
    queue = "consensus.consensus_unresolved",
    payload = payload,
  }, run_opts)
end

local function json_string(value)
  return tostring(value)
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
end

local function mock_issue_state(labels, state)
  local rendered_labels = {}
  for _, label in ipairs(labels or { "fkst-dev:enabled" }) do
    table.insert(rendered_labels, string.format('{"name":"%s"}', json_string(label)))
  end
  t.mock_command("--json labels,state", {
    stdout = string.format('{"state":"%s","labels":[%s]}\n', json_string(state or "OPEN"), table.concat(rendered_labels, ",")),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_body(body)
  t.mock_command("--json body", {
    stdout = string.format('{"body":"%s"}\n', json_string(body or "Issue body")),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_result(labels, comments)
  local rendered_labels = {}
  for _, label in ipairs(labels or { "fkst-dev:thinking" }) do
    table.insert(rendered_labels, string.format('{"name":"%s"}', json_string(label)))
  end
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered_comments, string.format('{"body":"%s"}', json_string(comment)))
  end
  t.mock_command("--json labels,comments", {
    stdout = string.format('{"labels":[%s],"comments":[%s]}\n', table.concat(rendered_labels, ","), table.concat(rendered_comments, ",")),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_loop(labels, comments, extra)
  local rendered_labels = {}
  for _, label in ipairs(labels or { "fkst-dev:thinking" }) do
    table.insert(rendered_labels, string.format('{"name":"%s"}', json_string(label)))
  end
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered_comments, string.format('{"body":"%s"}', json_string(comment)))
  end
  local fields = extra or {}
  t.mock_command("--json title,body,updatedAt,labels,comments,state", {
    stdout = string.format(
      '{"title":"%s","body":"%s","updatedAt":"%s","state":"%s","labels":[%s],"comments":[%s]}\n',
      json_string(fields.title or "Implement decision recorder"),
      json_string(fields.body or "Body from GitHub"),
      json_string(fields.updated_at or "2026-06-03T01:02:03Z"),
      json_string(fields.state or "OPEN"),
      table.concat(rendered_labels, ","),
      table.concat(rendered_comments, ",")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_view_failure(json_selector, stderr)
  t.mock_command(json_selector, {
    stdout = "",
    stderr = stderr or "forced issue view failure",
    exit_code = 1,
  })
end

local function count_calls(needle)
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find(needle, 1, true) ~= nil then
      count = count + 1
    end
  end
  return count
end

return {
  test_observe_opt_in_issue_raises_proposal_and_thinking_label = function()
    mock_issue_state({ "fkst-dev:enabled" })
    mock_issue_body("Body from GitHub")

    local result = run_observe(issue(), opts("observe-opt-in"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "consensus.proposal")
    t.eq(result.raises[1].payload.schema, "consensus.proposal.v1")
    t.eq(result.raises[1].payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(result.raises[1].payload.body, "Body from GitHub")
    t.eq(result.raises[1].payload.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z")
    t.eq(result.raises[1].payload.source_ref.ref, "owner/repo#issue/42")

    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[2].payload.schema, "github-proxy.label.v1")
    t.eq(result.raises[2].payload.add_labels[1], "fkst-dev:thinking")
    t.eq(result.raises[2].payload.issue_number, 42)
    t.eq(count_calls("gh issue view"), 2)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 1)
  end,

  test_observe_skips_not_opt_in_and_already_stateful = function()
    mock_issue_state({ "bug" })
    local not_opted = run_observe(issue({ labels = { "bug" } }), opts("observe-no-label"))
    t.eq(not_opted.exit_code, 0)
    t.eq(#not_opted.raises, 0)

    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" })
    local thinking = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:thinking" } }), opts("observe-thinking"))
    t.eq(thinking.exit_code, 0)
    t.eq(#thinking.raises, 0)
    t.eq(count_calls("gh issue view"), 2)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_re_derives_labels_and_skips_stale_enabled_payload = function()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:ready" })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled" } }), opts("observe-stale-payload"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_uses_current_github_state_not_payload_state = function()
    mock_issue_state({ "fkst-dev:enabled" }, "OPEN")
    mock_issue_body("Body from GitHub")

    local result = run_observe(issue({ state = "CLOSED" }), opts("observe-stale-state"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
  end,

  test_observe_issue_state_view_failure_errors_for_retry = function()
    mock_issue_view_failure("--json labels,state", "forced state failure")

    local result = run_observe(issue(), opts("observe-state-view-failure"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_body_view_failure_errors_for_retry = function()
    mock_issue_state({ "fkst-dev:enabled" })
    mock_issue_view_failure("--json body", "forced body failure")

    local result = run_observe(issue(), opts("observe-body-view-failure"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 1)
  end,

  test_observe_re_raises_until_thinking_label_is_on_issue = function()
    local run_opts = opts("observe-idempotent")
    mock_issue_state({ "fkst-dev:enabled" })
    mock_issue_body("Body from GitHub")

    local first = run_observe(issue(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 2)

    mock_issue_state({ "fkst-dev:enabled" })
    mock_issue_body("Body from GitHub")
    local second = run_observe(issue(), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 2)

    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" })
    local thinking = run_observe(issue(), run_opts)
    t.eq(thinking.exit_code, 0)
    t.eq(#thinking.raises, 0)
    t.eq(count_calls("--json labels,state"), 3)
    t.eq(count_calls("--json body"), 2)
  end,

  test_consensus_result_approve_raises_ready_label_and_comment = function()
    mock_issue_result({ "fkst-dev:thinking" })
    local result = run_result(reached(), opts("result-approve"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[1].payload.add_labels[1], "fkst-dev:ready")
    t.eq(result.raises[1].payload.remove_labels[1], "fkst-dev:thinking")
    t.eq(result.raises[1].payload.remove_labels[3], "fkst-dev:stuck")
    t.eq(result.raises[1].payload.issue_number, "42")

    t.eq(result.raises[2].queue, "github-proxy.github_issue_comment_request")
    t.eq(result.raises[2].payload.issue_number, "42")
    t.is_true(result.raises[2].payload.body:find("github-devloop decision: approve", 1, true) ~= nil)
    t.is_true(result.raises[2].payload.body:find('decision="approve"', 1, true) ~= nil)
  end,

  test_consensus_result_reject_raises_blocked = function()
    mock_issue_result({ "fkst-dev:thinking" })
    local result = run_result(reached({ decision = "reject" }), opts("result-reject"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].payload.add_labels[1], "fkst-dev:blocked")
    t.eq(result.raises[1].payload.remove_labels[1], "fkst-dev:thinking")
    t.is_true(result.raises[2].payload.body:find('decision="reject"', 1, true) ~= nil)
  end,

  test_consensus_result_reject_self_heals_opposite_ready_and_skips_completed_marker = function()
    mock_issue_result({ "fkst-dev:thinking", "fkst-dev:ready" })

    local stale_ready = run_result(reached({ decision = "reject" }), opts("result-reject-stale-ready"))
    t.eq(stale_ready.exit_code, 0)
    t.eq(#stale_ready.raises, 2)
    t.eq(stale_ready.raises[1].queue, "github-proxy.github_issue_label_request")
    t.eq(stale_ready.raises[1].payload.add_labels[1], "fkst-dev:blocked")
    t.eq(stale_ready.raises[1].payload.remove_labels[1], "fkst-dev:thinking")
    t.eq(stale_ready.raises[1].payload.remove_labels[2], "fkst-dev:ready")
    t.eq(stale_ready.raises[2].queue, "github-proxy.github_issue_comment_request")

    local completed = reached({ decision = "reject" })
    local marker = core.result_marker(completed.proposal_id, completed.decision, completed.dedup_key)
    mock_issue_result({ "fkst-dev:blocked" }, { marker })

    local complete = run_result(completed, opts("result-reject-complete"))
    t.eq(complete.exit_code, 0)
    t.eq(#complete.raises, 0)
    t.eq(count_calls("--json labels,comments"), 2)
  end,

  test_consensus_result_skips_foreign_proposal = function()
    local result = run_result(reached({ proposal_id = "autochrono/issue/owner/repo/42" }), opts("result-foreign"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_writes_marker_when_terminal_label_present_without_marker = function()
    mock_issue_result({ "fkst-dev:ready" })

    local result = run_result(reached(), opts("result-terminal-label"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.eq(result.raises[1].payload.issue_number, "42")
    t.is_true(result.raises[1].payload.body:find("github-devloop decision: approve", 1, true) ~= nil)
    t.eq(count_calls("--json labels,comments"), 1)
  end,

  test_consensus_result_removes_thinking_when_terminal_label_present = function()
    mock_issue_result({ "fkst-dev:ready", "fkst-dev:thinking" })

    local result = run_result(reached(), opts("result-terminal-plus-thinking"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[1].payload.add_labels[1], "fkst-dev:ready")
    t.eq(result.raises[1].payload.remove_labels[1], "fkst-dev:thinking")
    t.eq(result.raises[2].queue, "github-proxy.github_issue_comment_request")
  end,

  test_consensus_result_removes_stuck_when_late_reached_arrives = function()
    mock_issue_result({ "fkst-dev:stuck" })

    local result = run_result(reached(), opts("result-late-after-stuck"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[1].payload.add_labels[1], "fkst-dev:ready")
    t.eq(result.raises[1].payload.remove_labels[3], "fkst-dev:stuck")
    t.eq(result.raises[2].queue, "github-proxy.github_issue_comment_request")
  end,

  test_consensus_result_raises_label_when_result_marker_present_without_terminal_label = function()
    local current = reached()
    local marker = core.result_marker(current.proposal_id, current.decision, current.dedup_key)
    mock_issue_result({ "fkst-dev:thinking" }, { marker })

    local result = run_result(current, opts("result-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[1].payload.add_labels[1], "fkst-dev:ready")
  end,

  test_consensus_result_skips_when_terminal_label_and_result_marker_present = function()
    local current = reached()
    local marker = core.result_marker(current.proposal_id, current.decision, current.dedup_key)
    mock_issue_result({ "fkst-dev:ready" }, { marker })

    local result = run_result(current, opts("result-complete"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_stale_opposite_marker_does_not_suppress_current_reject = function()
    local current = reached({ decision = "reject" })
    local stale_marker = core.result_marker(current.proposal_id, "approve", current.dedup_key)
    mock_issue_result({ "fkst-dev:ready" }, { stale_marker })

    local result = run_result(current, opts("result-stale-opposite-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[1].payload.add_labels[1], "fkst-dev:blocked")
    t.eq(result.raises[1].payload.remove_labels[2], "fkst-dev:ready")
    t.eq(result.raises[2].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[2].payload.body:find('decision="reject"', 1, true) ~= nil)
    t.is_true(result.raises[2].payload.body:find(core.result_marker(current.proposal_id, current.decision, current.dedup_key), 1, true) ~= nil)
  end,

  test_consensus_result_older_same_direction_marker_does_not_suppress_current_version = function()
    local current = reached({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/v2",
    })
    local older_marker = core.result_marker(current.proposal_id, "approve", "consensus:github-devloop/issue/owner/repo/42/v1")
    mock_issue_result({ "fkst-dev:ready" }, { older_marker })

    local result = run_result(current, opts("result-older-same-direction-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find(core.result_marker(current.proposal_id, current.decision, current.dedup_key), 1, true) ~= nil)
    t.is_true(result.raises[1].payload.dedup_key:find("/v2", 1, true) ~= nil)
  end,

  test_consensus_result_view_failure_errors_for_retry = function()
    mock_issue_view_failure("--json labels,comments", "forced result failure")

    local result = run_result(reached(), opts("result-view-failure"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json labels,comments"), 1)
  end,

  test_consensus_result_rejects_malformed_proposal_id_before_gh_view = function()
    local result = run_result(reached({
      proposal_id = "github-devloop/issue/owner/repo/../../42",
      dedup_key = "github-devloop/issue/owner/repo/../../42/result",
    }), opts("result-malformed-proposal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh issue view"), 0)
  end,

  test_consensus_result_re_raises_until_github_has_terminal_fact = function()
    local run_opts = opts("result-idempotent")
    mock_issue_result({ "fkst-dev:thinking" })

    local first = run_result(reached(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 2)

    mock_issue_result({ "fkst-dev:thinking" })
    local second = run_result(reached({ body = "Different body." }), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 2)
  end,

  test_loop_unresolved_reraises_proposal_and_loop_marker_under_budget = function()
    mock_issue_loop({ "fkst-dev:thinking" })

    local result = run_loop(unresolved(), opts("loop-under-budget"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "consensus.proposal")
    t.eq(result.raises[1].payload.schema, "consensus.proposal.v1")
    t.eq(result.raises[1].payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(result.raises[1].payload.body, "Body from GitHub")
    t.eq(result.raises[1].payload.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/1")
    t.eq(result.raises[1].payload.source_ref.ref, "owner/repo#issue/42")

    t.eq(result.raises[2].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[2].payload.body:find(
      core.loop_marker("github-devloop/issue/owner/repo/42", 1, unresolved().dedup_key),
      1,
      true
    ) ~= nil)
    t.is_true(result.raises[2].payload.dedup_key:find("/comment/loop/1/", 1, true) ~= nil)
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 1)
  end,

  test_loop_reaching_budget_raises_stuck_label_and_marker_without_proposal = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.loop_marker(event.proposal_id, 1, "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"),
      core.loop_marker(event.proposal_id, 2, "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/1"),
    })

    local result = run_loop(event, opts("loop-budget"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find(core.stuck_marker(event.proposal_id, 3, event.dedup_key), 1, true) ~= nil)
    t.is_true(result.raises[1].payload.dedup_key:find("/comment/stuck/3/", 1, true) ~= nil)

    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[2].payload.add_labels[1], "fkst-dev:stuck")
    t.eq(result.raises[2].payload.remove_labels[1], "fkst-dev:thinking")
  end,

  test_loop_uses_unresolved_dedup_loop_suffix_when_github_markers_lag = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    })
    mock_issue_loop({ "fkst-dev:thinking" })

    local result = run_loop(event, opts("loop-dedup-suffix-counts-marker-lag"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find(core.stuck_marker(event.proposal_id, 3, event.dedup_key), 1, true) ~= nil)
    t.is_true(result.raises[1].payload.dedup_key:find("/comment/stuck/3/", 1, true) ~= nil)
    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[2].payload.add_labels[1], "fkst-dev:stuck")
    t.eq(result.raises[2].payload.remove_labels[1], "fkst-dev:thinking")
  end,

  test_loop_github_markers_ahead_of_event_still_bound_round = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/v2",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.loop_marker(event.proposal_id, 1, "consensus:github-devloop/issue/owner/repo/42/base"),
      core.loop_marker(event.proposal_id, 2, "consensus:github-devloop/issue/owner/repo/42/v1"),
    })

    local result = run_loop(event, opts("loop-markers-bound-event"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find(core.stuck_marker(event.proposal_id, 3, event.dedup_key), 1, true) ~= nil)
    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[2].payload.add_labels[1], "fkst-dev:stuck")
    t.eq(result.raises[2].payload.remove_labels[1], "fkst-dev:thinking")
  end,

  test_loop_skips_foreign_proposal = function()
    local result = run_loop(unresolved({ proposal_id = "autochrono/issue/owner/repo/42" }), opts("loop-foreign"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh issue view"), 0)
  end,

  test_loop_skips_already_terminal_issue = function()
    mock_issue_loop({ "fkst-dev:ready" })

    local result = run_loop(unresolved(), opts("loop-terminal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 1)
  end,

  test_loop_retries_until_state_label_is_visible = function()
    mock_issue_loop({ "fkst-dev:enabled" })

    local pending = run_loop(unresolved(), opts("loop-state-label-pending"))
    t.eq(pending.exit_code, 1)
    t.eq(#pending.raises, 0)

    mock_issue_loop({ "fkst-dev:ready" })
    local ready = run_loop(unresolved(), opts("loop-state-label-ready"))
    t.eq(ready.exit_code, 0)
    t.eq(#ready.raises, 0)

    mock_issue_loop({ "fkst-dev:thinking" })
    local thinking = run_loop(unresolved(), opts("loop-state-label-thinking"))
    t.eq(thinking.exit_code, 0)
    t.eq(#thinking.raises, 2)
    t.eq(thinking.raises[1].queue, "consensus.proposal")
    t.eq(thinking.raises[2].queue, "github-proxy.github_issue_comment_request")
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 3)
  end,

  test_loop_skips_decision_terminal_even_when_thinking_lingers = function()
    mock_issue_loop({ "fkst-dev:thinking", "fkst-dev:ready" })

    local ready = run_loop(unresolved(), opts("loop-terminal-plus-thinking"))
    t.eq(ready.exit_code, 0)
    t.eq(#ready.raises, 0)
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 1)

    local stuck_event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    })
    mock_issue_loop({ "fkst-dev:thinking", "fkst-dev:stuck" }, {
      core.stuck_marker(stuck_event.proposal_id, 3, stuck_event.dedup_key),
    })

    local stuck = run_loop(stuck_event, opts("loop-stuck-plus-thinking-self-heal"))
    t.eq(stuck.exit_code, 0)
    t.eq(#stuck.raises, 1)
    t.eq(stuck.raises[1].queue, "github-proxy.github_issue_label_request")
    t.eq(stuck.raises[1].payload.add_labels[1], "fkst-dev:stuck")
    t.eq(stuck.raises[1].payload.remove_labels[1], "fkst-dev:thinking")
  end,

  test_loop_issue_view_failure_errors_for_retry = function()
    mock_issue_view_failure("--json title,body,updatedAt,labels,comments,state", "forced loop failure")

    local result = run_loop(unresolved(), opts("loop-view-failure"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
    t.eq(count_calls("--json title,body,updatedAt,labels,comments,state"), 1)
  end,

  test_loop_duplicate_same_round_unresolved_does_not_advance_budget = function()
    local event = unresolved()
    mock_issue_loop({ "fkst-dev:thinking" }, { core.loop_marker(event.proposal_id, 1, event.dedup_key) })

    local result = run_loop(event, opts("loop-duplicate-same-round"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_new_round_unresolved_advances_by_version = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/1",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.loop_marker(
        event.proposal_id,
        1,
        "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
      ),
    })

    local result = run_loop(event, opts("loop-new-version-advances"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "consensus.proposal")
    t.eq(result.raises[1].payload.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2")
    t.eq(result.raises[2].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[2].payload.body:find(core.loop_marker(event.proposal_id, 2, event.dedup_key), 1, true) ~= nil)
  end,

  test_loop_duplicate_new_round_unresolved_skips_when_next_marker_exists = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/1",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.loop_marker(event.proposal_id, 1, "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"),
      core.loop_marker(event.proposal_id, 2, event.dedup_key),
    })

    local result = run_loop(event, opts("loop-new-version-duplicate"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_stuck_marker_idempotency_skips_repeat = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    })
    mock_issue_loop({ "fkst-dev:stuck" }, { core.stuck_marker(event.proposal_id, 3, event.dedup_key) })

    local result = run_loop(event, opts("loop-idempotent-stuck-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_older_stuck_marker_does_not_suppress_current_version = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/v2",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.loop_marker(event.proposal_id, 1, "consensus:github-devloop/issue/owner/repo/42/base"),
      core.loop_marker(event.proposal_id, 2, "consensus:github-devloop/issue/owner/repo/42/v1"),
      core.loop_marker(event.proposal_id, 3, "consensus:github-devloop/issue/owner/repo/42/v1/loop/2"),
      core.stuck_marker(event.proposal_id, 3, "consensus:github-devloop/issue/owner/repo/42/v1"),
    })

    local result = run_loop(event, opts("loop-older-stuck-marker"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find(core.stuck_marker(event.proposal_id, 3, event.dedup_key), 1, true) ~= nil)
    t.is_true(result.raises[1].payload.dedup_key:find("/comment/stuck/3", 1, true) ~= nil)
    t.eq(result.raises[2].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[2].payload.add_labels[1], "fkst-dev:stuck")
    t.eq(result.raises[2].payload.remove_labels[1], "fkst-dev:thinking")
  end,

  test_loop_stuck_marker_self_heals_label_transition = function()
    local event = unresolved({
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/2",
    })
    mock_issue_loop({ "fkst-dev:thinking" }, { core.stuck_marker(event.proposal_id, 3, event.dedup_key) })

    local result = run_loop(event, opts("loop-stuck-marker-self-heal-label"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_label_request")
    t.eq(result.raises[1].payload.add_labels[1], "fkst-dev:stuck")
    t.eq(result.raises[1].payload.remove_labels[1], "fkst-dev:thinking")
  end,
}
