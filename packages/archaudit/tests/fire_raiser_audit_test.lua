local helper = require("tests.fire_raiser_helpers")
local t = fkst.test

return {
  test_fire_raiser_audit_poll_idle_and_due_produces_issue_create_request = function()
    local root = helper.setup_workspace("produce", helper.fire_raiser_child([[
  test_full_produce = function()
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_production_github("[]", "[]")
    mock_codex_findings("[]", 0)

    local trace = t.fire_raiser("audit_poll")
    t.eq(trace.source_ref.kind, "cron")
    t.eq(trace.source_payload.raiser, "archaudit.audit_poll")
    t.eq(trace.routed_to[1], "archaudit.audit")
    if trace.consumer_result.status ~= "accepted" then
      error(trace.consumer_result.message or "fire_raiser consumer failed")
    end
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 1)
    t.eq(trace.raised[1].queue, "github-proxy.github_issue_create_request")
    t.eq(trace.raised[1].payload.schema, "github-proxy.issue-create.v1")
    t.eq(trace.raised[1].payload.repo, "owner/repo")
    t.eq(trace.raised[1].payload.title, "Archaudit: audit completed with zero findings")
    t.is_true(trace.raised[1].payload.body:find("Audit trigger: stale", 1, true) ~= nil)
    t.is_true(trace.raised[1].payload.body:find('fkst:archaudit:audit-run:v1 reason="stale"', 1, true) ~= nil)
  end,
]]))
    local output = helper.run_child(root)
    t.is_true(output:find("1 passed, 0 failed", 1, true) ~= nil, output)
  end,

  test_fire_raiser_audit_poll_recent_audit_issue_skips_without_issue_create_request = function()
    local root = helper.setup_workspace("skip", helper.fire_raiser_child([[
  test_recent_audit_skip = function()
    local recent = '[{"number":77,"title":"Archaudit: packages/archaudit/core.lua:1 SRP","state":"OPEN","body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"stale\\" -->","createdAt":"2026-06-19T00:30:00Z","author":{"login":"fkst-test-bot"},"url":"https://github.com/owner/repo/issues/77"}]'
    mock_env("owner/repo", "3")
    mock_idle_observe()
    mock_production_github(recent, "[]")

    local trace = t.fire_raiser("audit_poll")
    t.eq(trace.source_payload.raiser, "archaudit.audit_poll")
    t.eq(trace.routed_to[1], "archaudit.audit")
    if trace.consumer_result.status ~= "accepted" then
      error(trace.consumer_result.message or "fire_raiser consumer failed")
    end
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 0)
  end,
]]))
    local output = helper.run_child(root)
    t.is_true(output:find("1 passed, 0 failed", 1, true) ~= nil, output)
  end,

  test_fire_raiser_audit_poll_busy_overdue_produces_issue_create_request = function()
    local root = helper.setup_workspace("busy-overdue", helper.fire_raiser_child([[
  test_busy_overdue_terminal_fire = function()
    local past_force_at_but_before_raw_deadline = '[{"number":77,"title":"Archaudit: audit completed with zero findings","state":"OPEN","body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"stale\\" -->","createdAt":"2026-06-19T02:15:00Z","author":{"login":"fkst-test-bot"},"url":"https://github.com/owner/repo/issues/77"}]'
    mock_env("owner/repo", "3")
    mock_busy_observe_at(1781917260000)
    mock_production_github(past_force_at_but_before_raw_deadline, "[]")
    mock_codex_findings("[]", 0)

    local trace = t.fire_raiser("audit_poll")
    t.eq(trace.source_payload.raiser, "archaudit.audit_poll")
    t.eq(trace.routed_to[1], "archaudit.audit")
    if trace.consumer_result.status ~= "accepted" then
      error(trace.consumer_result.message or "fire_raiser consumer failed")
    end
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 1)
    t.eq(trace.raised[1].queue, "github-proxy.github_issue_create_request")
    t.eq(trace.raised[1].payload.title, "Archaudit: audit completed with zero findings")
    t.is_true(trace.raised[1].payload.body:find("Audit trigger: stale", 1, true) ~= nil)
  end,
]]))
    local output = helper.run_child(root)
    t.is_true(output:find("1 passed, 0 failed", 1, true) ~= nil, output)
  end,

  test_fire_raiser_audit_poll_busy_not_overdue_skips_issue_create_request = function()
    local root = helper.setup_workspace("busy-not-overdue", helper.fire_raiser_child([[
  test_busy_not_overdue_skips = function()
    local just_before_force_at = '[{"number":77,"title":"Archaudit: audit completed with zero findings","state":"OPEN","body":"<!-- fkst:archaudit:audit-run:v1 reason=\\"stale\\" -->","createdAt":"2026-06-19T02:17:00Z","author":{"login":"fkst-test-bot"},"url":"https://github.com/owner/repo/issues/77"}]'
    mock_env("owner/repo", "3")
    mock_busy_observe_at(1781917260000)
    mock_production_github(just_before_force_at, "[]")

    local trace = t.fire_raiser("audit_poll")
    t.eq(trace.source_payload.raiser, "archaudit.audit_poll")
    t.eq(trace.routed_to[1], "archaudit.audit")
    if trace.consumer_result.status ~= "accepted" then
      error(trace.consumer_result.message or "fire_raiser consumer failed")
    end
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 0)
  end,
]]))
    local output = helper.run_child(root)
    t.is_true(output:find("1 passed, 0 failed", 1, true) ~= nil, output)
  end,
}
