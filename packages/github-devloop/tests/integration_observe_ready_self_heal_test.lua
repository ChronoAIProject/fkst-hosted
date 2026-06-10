local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local issue = h.issue
local reached = h.reached
local opts = h.opts
local source_ref = h.source_ref
local run_observe = h.run_observe
local run_implement = h.run_implement
local mock_issue_state = h.mock_issue_state
local mock_issue_implement_raw = h.mock_issue_implement_raw
local count_calls = h.count_calls
local find_raise = h.find_raise
local render_comment = h.render_comment
local json_string = h.json_string

local function has_value(values, expected)
  for _, value in ipairs(values or {}) do
    if value == expected then
      return true
    end
  end
  return false
end

local function mock_linked_pr_state(comments, state, exit_code)
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered_comments, render_comment(comment))
  end
  local stderr = ""
  if exit_code ~= nil and exit_code ~= 0 then
    stderr = "pr view failed"
  end
  t.mock_command("--json headRefName,headRefOid,baseRefName,state,updatedAt,comments", {
    stdout = string.format(
      '{"headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"%s","updatedAt":"2026-06-03T02:03:04Z","comments":[%s]}\n',
      json_string(state or "OPEN"),
      table.concat(rendered_comments, ",")
    ),
    stderr = stderr,
    exit_code = exit_code or 0,
  })
end

return {
  test_observe_issue_reraises_ready_for_poll_self_heal = function()
    local event = reached()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:ready" }, "OPEN", {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:ready" } }), opts("observe-issue-ready-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local ready_raise = find_raise(result.raises, "devloop_ready")
    t.eq(ready_raise.payload.schema, "github-devloop.ready.v1")
    t.eq(ready_raise.payload.proposal_id, event.proposal_id)
    t.eq(ready_raise.payload.source_ref.ref, "owner/repo#issue/42")
    t.eq(ready_raise.payload.dedup_key, core.build_devloop_ready_payload({
      proposal_id = event.proposal_id,
      dedup_key = event.dedup_key,
      source_ref = event.source_ref,
    }).dedup_key)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_ready_self_heal_does_not_duplicate_after_implementing = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    local branch = core.implement_branch("owner/repo", 42, ready_payload.dedup_key)
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:implementing" }, "OPEN", {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
      core.state_marker(event.proposal_id, "implementing", ready_payload.dedup_key),
    })

    local observed = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:implementing" } }), opts("observe-issue-ready-self-heal-advanced"))
    t.eq(observed.exit_code, 0)
    t.eq(find_raise(observed.raises, "devloop_ready"), nil)
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json body"), 0)

    mock_issue_implement_raw({ "fkst-dev:implementing" }, {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
      core.state_marker(event.proposal_id, "implementing", ready_payload.dedup_key),
      core.implementing_marker(event.proposal_id, ready_payload.dedup_key, branch, "abc123", "dev", "def456"),
    })
    local implemented = run_implement(ready_payload, opts("implement-ready-self-heal-advanced"))
    t.eq(implemented.exit_code, 0)
    t.eq(#implemented.raises, 0)
  end,

  test_observe_issue_uses_pr_local_current_state_over_issue_pr_open = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    local issue_comments = {
      core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key),
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", issue_comments)
    mock_linked_pr_state({
      core.state_marker(event.proposal_id, "reviewing", ready_payload.dedup_key),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" } }), opts("observe-issue-pr-local-reviewing"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:reviewing")
    t.is_true(has_value(label_raise.payload.remove_labels, "fkst-dev:pr-open"))
    t.eq(count_calls("--json labels,state"), 1)
    t.eq(count_calls("--json headRefName,headRefOid,baseRefName,state,updatedAt,comments"), 1)
  end,

  test_observe_issue_missing_reviewing_label_does_not_change_pr_local_state = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    mock_issue_state({ "fkst-dev:enabled" }, "OPEN", {
      core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key),
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    })
    mock_linked_pr_state({
      core.state_marker(event.proposal_id, "reviewing", ready_payload.dedup_key),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled" } }), opts("observe-issue-pr-local-reviewing-no-label"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:reviewing")
  end,

  test_observe_issue_linked_pr_fetch_failure_fails_closed = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", {
      core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key),
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    })
    mock_linked_pr_state({}, "OPEN", 1)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" } }), opts("observe-issue-pr-local-fetch-failure"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,
}
