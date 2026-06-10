local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core

local function opts(name, write_mode)
  return {
    env = {
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/github-devloop/" .. tostring(now()) .. "/" .. tostring(name),
      FKST_GITHUB_WRITE = write_mode or "1",
    },
  }
end

local function event(extra)
  local payload = core.rollup_ready_payload("owner/repo", "dev", "integration/dev", 9, "def456")
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return payload
end

local function run_merge(payload, run_opts)
  return t.run_department("departments/rollup_merge/main.lua", {
    queue = "devloop_rollup_ready",
    payload = payload,
  }, run_opts or opts("rollup-merge"))
end

local function mock_write_mode(value)
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = value or "1", stderr = "", exit_code = 0 })
end

local function mock_pr(head_sha, base, rollup_state, rollup_conclusion, mergeable, merge_state, state, merged_at)
  t.mock_command("--json headRefName,headRefOid,baseRefName,state,isDraft,mergedAt,comments,headRepository,headRepositoryOwner,isCrossRepository,mergeable,mergeStateStatus,statusCheckRollup", {
    stdout = string.format(
      '{"headRefName":"integration/dev","headRefOid":"%s","baseRefName":"%s","state":"%s","isDraft":false,"mergedAt":"%s","comments":[],"headRepository":{"nameWithOwner":"owner/repo"},"isCrossRepository":false,"mergeable":"%s","mergeStateStatus":"%s","statusCheckRollup":[{"name":"ci","state":"%s","conclusion":"%s"}]}\n',
      h.json_string(head_sha or "def456"),
      h.json_string(base or "dev"),
      h.json_string(state or "OPEN"),
      h.json_string(merged_at or ""),
      h.json_string(mergeable or "MERGEABLE"),
      h.json_string(merge_state or "CLEAN"),
      h.json_string(rollup_state or "COMPLETED"),
      h.json_string(rollup_conclusion or "SUCCESS")
    ),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_merge_command()
  t.mock_command("gh pr merge '9' --repo 'owner/repo' --merge --match-head-commit 'def456'", {
    stdout = "merged\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_successful_merge()
  mock_write_mode("1")
  mock_pr()
  mock_pr()
  mock_merge_command()
  mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "MERGEABLE", "CLEAN", "MERGED", "2026-06-03T02:03:04Z")
end

return {
  test_rollup_merge_green_mergeable_identity_match_merges = function()
    mock_successful_merge()
    local result = run_merge(event(), opts("rollup-merge-success", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 1)
    t.is_true(h.has_call("--match-head-commit 'def456'"))
  end,

  test_rollup_merge_red_or_pending_ci_never_merges = function()
    mock_write_mode("1")
    mock_pr("def456", "dev", "COMPLETED", "FAILURE")
    local red = run_merge(event(), opts("rollup-merge-red", "1"))
    t.eq(red.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)

    mock_write_mode("1")
    mock_pr("def456", "dev", "IN_PROGRESS", "")
    local pending = run_merge(event(), opts("rollup-merge-pending", "1"))
    t.eq(pending.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_neutral_ci_does_not_merge = function()
    mock_write_mode("1")
    mock_pr("def456", "dev", "COMPLETED", "NEUTRAL")
    local result = run_merge(event(), opts("rollup-merge-neutral", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_unmergeable_never_merges = function()
    mock_write_mode("1")
    mock_pr("def456", "dev", "COMPLETED", "SUCCESS", "CONFLICTING", "DIRTY")
    local result = run_merge(event(), opts("rollup-merge-unmergeable", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_head_mismatch_never_merges = function()
    mock_write_mode("1")
    mock_pr("aaaa1111")
    local result = run_merge(event(), opts("rollup-merge-head-mismatch", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_base_mismatch_never_merges = function()
    mock_write_mode("1")
    mock_pr("def456", "main")
    local result = run_merge(event(), opts("rollup-merge-base-mismatch", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
  end,

  test_rollup_merge_does_not_require_issue_review_markers = function()
    mock_successful_merge()
    local result = run_merge(event(), opts("rollup-merge-no-markers", "1"))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh issue view"), 0)
    t.eq(h.count_calls("gh issue comment"), 0)
    t.eq(h.count_calls("gh pr merge"), 1)
  end,

  test_rollup_merge_dry_run_never_merges = function()
    mock_write_mode("")
    local result = run_merge(event(), opts("rollup-merge-dry-run", ""))
    t.eq(result.exit_code, 0)
    t.eq(h.count_calls("gh pr merge"), 0)
    t.eq(h.count_calls("gh pr view"), 0)
  end,
}
