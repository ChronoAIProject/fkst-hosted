local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local old_dashboard_body_cap = 12000

local function mock_dashboard_env()
  for _ = 1, 4 do
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function large_mermaid(line_count)
  local lines = { "flowchart LR" }
  for index = 1, line_count do
    table.insert(lines, "  node_" .. tostring(index) .. " --> node_" .. tostring(index + 1))
  end
  return table.concat(lines, "\n")
end

local function trusted_comment(body, created_at, id)
  return {
    id = id,
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at or "2026-06-03T01:00:00Z",
  }
end

local function untrusted_comment(body, created_at, id)
  return {
    id = id,
    body = body,
    author_login = "mallory",
    created_at = created_at or "2026-06-03T01:00:00Z",
  }
end

local function autonomy_record(fields)
  local record = {
    proposal_id = fields.proposal_id,
    repo = "owner/repo",
    issue_number = fields.issue_number or "42",
    pr_number = fields.pr_number or "7",
    version = fields.version,
    head_sha = fields.head_sha or "def456",
    task_class = fields.task_class,
    human_touch_count = 0,
    rounds = fields.rounds or 1,
    retry_count = fields.retry_count or 0,
    codex_calls = fields.codex_calls,
    gates = fields.gates,
  }
  return record
end

local function assert_dashboard_marker_outside_fences(body)
  local marker_start = body:find("<!-- fkst:dashboard:v1", 1, true)
  t.is_true(marker_start ~= nil)

  local search_from = 1
  local last_close = nil
  while true do
    local opening = body:find("```mermaid", search_from, true)
    if opening == nil then
      break
    end
    local closing = body:find("\n```", opening + #"```mermaid", true)
    t.is_true(closing ~= nil)
    t.is_true(closing < marker_start)
    t.eq(body:sub(opening, closing):find("<!--", 1, true), nil)
    last_close = closing
    search_from = closing + #"\n```"
  end

  if last_close ~= nil then
    t.is_true(last_close < marker_start)
  end
end

return {
  test_avm_scoreboard_aggregates_by_task_level_without_total = function()
    local rows = core.aggregate_avm_scoreboard({
      {
        proposal_id = "github-devloop/issue/owner/repo/1",
        pr_number = 11,
        version = "v1",
        head_sha = "abc123",
        task_class = "L1",
        valid_autonomous_merge = "true",
        avm_rate_numerator = 1,
        avm_rate_denominator = 2,
        codex_calls = 6,
        rounds = 3,
        gates = { no_revert_reopen = "pass" },
        false_consensus = false,
      },
      {
        proposal_id = "github-devloop/issue/owner/repo/1",
        pr_number = 11,
        version = "v1",
        head_sha = "abc123",
        task_class = "L1",
        valid_autonomous_merge = "true",
        avm_rate_numerator = 1,
        avm_rate_denominator = 2,
        codex_calls = 6,
        rounds = 3,
        gates = { no_revert_reopen = "pass" },
        false_consensus = false,
      },
      {
        proposal_id = "github-devloop/issue/owner/repo/2",
        pr_number = 12,
        version = "v2",
        head_sha = "def456",
        task_class = "unknown",
        valid_autonomous_merge = "false",
        rounds = 4,
        gates = { no_revert_reopen = "fail" },
      },
    })
    local by_level = {}
    for _, row in ipairs(rows) do
      by_level[row.level] = row
    end

    t.eq(by_level.L1.merges, 1)
    t.eq(by_level.L1.avm_numerator, 1)
    t.eq(by_level.L1.avm_denominator, 2)
    t.eq(by_level.L1.false_consensus_numerator, 0)
    t.eq(by_level.L1.false_consensus_denominator, 1)
    t.eq(by_level.unclassified.merges, 1)
    t.eq(by_level.unclassified.avm_denominator, 1)
    t.eq(by_level.unclassified.revert_numerator, 1)
    t.eq(by_level.unclassified.false_consensus_numerator, 0)
    t.eq(by_level.unclassified.false_consensus_denominator, 0)
    t.eq(core.render_avm_scoreboard_bucket(by_level.L1):find("TOTAL", 1, true), nil)
  end,

  test_dashboard_renders_avm_scoreboard_from_trusted_ledger_markers = function()
    mock_dashboard_env()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local first_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local second_version = first_version .. "/reimplement/2"
    local head_sha = "def456"
    local record = autonomy_record({
      proposal_id = proposal_id,
      version = second_version,
      head_sha = head_sha,
      task_class = "L4",
      rounds = 2,
      codex_calls = 8,
      gates = {
        human_touch = "pass",
        pre_merge_ci = "pass",
        evidence_manifest = "pass",
        post_merge_probe = "pass",
        no_revert_reopen = "fail",
        cost_budget = "pass",
      },
    })
    local comments = {
      trusted_comment(core.implement_attempt_marker(proposal_id, first_version, 1, "100"), "2026-06-03T01:00:00Z", 1001),
      trusted_comment(core.state_marker(proposal_id, "blocked", first_version), "2026-06-03T01:10:00Z", 1002),
      trusted_comment(core.implement_attempt_marker(proposal_id, second_version, 2, "200"), "2026-06-03T01:20:00Z", 1003),
      trusted_comment(core.merged_marker(proposal_id, "7", second_version, head_sha, record), "2026-06-03T01:30:00Z", 1004),
      untrusted_comment(core.merged_marker(proposal_id, "7", second_version, head_sha, record), "2026-06-03T01:31:00Z", 1005),
    }
    local dashboard = core.render_observability_dashboard({
      entities = {
        {
          proposal_id = proposal_id,
          issue_number = 42,
          pr_number = 7,
          title = "Security API recovery change",
          state = { state = "merged", version = second_version },
          parent_issue = { comments = comments },
          pr = { comments = comments },
        },
        {
          proposal_id = "github-devloop/issue/owner/repo/43",
          issue_number = 43,
          pr_number = 8,
          title = "Unclassified change",
          autonomy_results = {
            {
              proposal_id = "github-devloop/issue/owner/repo/43",
              pr_number = 8,
              version = "v3",
              head_sha = "fedcba",
              task_class = "unknown",
              valid_autonomous_merge = "pending",
              rounds = 1,
              gates = { no_revert_reopen = "pending" },
            },
          },
        },
      },
      counts = { merged = 1 },
      stalls = {},
      topology_mermaid = "",
      now_seconds = 1770000000,
    })

    t.is_true(dashboard.body:find("## AVM scoreboard by task level", 1, true) ~= nil)
    t.is_true(dashboard.body:find(
      "- L4 merges=1 AVM-rate=0/2 (0%) cost-per-AVM=n/a revert-rate=1/1 (100%) median-rounds=2 false-consensus-rate=n/a",
      1,
      true
    ) ~= nil)
    t.is_true(dashboard.body:find("- unclassified merges=1 AVM-rate=0/1 (0%) cost-per-AVM=unknown", 1, true) ~= nil)
    t.eq(dashboard.body:find("TOTAL", 1, true), nil)
  end,

  test_dashboard_renders_large_topology_without_old_cap_cutting_mermaid = function()
    mock_dashboard_env()
    local mermaid = large_mermaid(900)
    t.is_true(#mermaid > old_dashboard_body_cap)

    local dashboard = core.render_observability_dashboard({
      entities = {},
      counts = {},
      stalls = {},
      topology_mermaid = mermaid,
      now_seconds = 1770000000,
    })

    t.is_true(#dashboard.body > old_dashboard_body_cap)
    t.is_true(dashboard.body:find("node_900 --> node_901", 1, true) ~= nil)
    t.is_true(dashboard.body:find("## Board by state", 1, true) ~= nil)
    t.is_true(dashboard.body:find("## Ready", 1, true) ~= nil)
    t.is_true(dashboard.body:find("## Blocked", 1, true) ~= nil)
    t.is_true(dashboard.body:find("## Stall suspects", 1, true) ~= nil)
    t.is_true(dashboard.body:find("## Footer", 1, true) ~= nil)
    assert_dashboard_marker_outside_fences(dashboard.body)
  end,

  test_dashboard_forced_cap_drops_whole_sections_without_open_fence = function()
    mock_dashboard_env()
    local forced_cap = 2500
    local dashboard = core.render_observability_dashboard({
      entities = {},
      counts = {},
      stalls = {},
      topology_mermaid = large_mermaid(900),
      now_seconds = 1770000000,
      max_body_len = forced_cap,
    })

    t.is_true(#dashboard.body <= forced_cap)
    t.eq(dashboard.body:find("node_900 --> node_901", 1, true), nil)
    assert_dashboard_marker_outside_fences(dashboard.body)
  end,
}
