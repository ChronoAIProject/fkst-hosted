local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

-- Reproduction + regression for WIP admission-cap starvation (#635): a PR-bound
-- active-WIP holder whose pr-link base branch is not this instance's integration
-- branch (e.g. a PR stranded on a retired integration branch after a topology
-- migration) must NOT permanently consume a MAX_INFLIGHT slot, or it deadlocks the
-- cap and starves all new work. Holders on the managed base, and holders with no PR
-- yet, must still count.

local REPO = "owner/repo"
local INTEGRATION = "integration/dev"

local function render_comment(body)
  return string.format(
    '{"body":"%s","author":{"login":"fkst-test-bot"},"createdAt":"2026-06-03T01:00:00Z"}',
    tostring(body or ""):gsub("\\", "\\\\"):gsub('"', '\\"'):gsub("\n", "\\n")
  )
end

local function mock_env(max_inflight)
  t.mock_command('printf %s "$FKST_DEVLOOP_MAX_INFLIGHT"', {
    stdout = tostring(max_inflight), stderr = "", exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', {
    stdout = "dev", stderr = "", exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', {
    stdout = INTEGRATION, stderr = "", exit_code = 0,
  })
end

local function mock_wip_list(numbers)
  local items = {}
  for _, number in ipairs(numbers or {}) do
    table.insert(items, string.format('{"number":%d}', number))
  end
  t.mock_command(core.gh_issue_list_wip_cmd(REPO), {
    stdout = "[" .. table.concat(items, ",") .. "]\n", stderr = "", exit_code = 0,
  })
end

-- markers may be nil to model an implementing holder with no pr-link yet
local function mock_wip_state(issue_number, state_name, base_branch)
  local proposal_id = core.proposal_id(REPO, issue_number)
  local version = "ready/consensus-github-devloop/issue/owner/repo/" .. tostring(issue_number) .. "/intake/1/loop/1"
  local comments = { render_comment(core.state_marker(proposal_id, state_name, version)) }
  if base_branch ~= nil then
    local branch = "devloop/issue/owner/repo/" .. tostring(issue_number) .. "/work"
    table.insert(comments, render_comment(core.pr_link_marker(proposal_id, issue_number + 500, branch, version, base_branch)))
  end
  t.mock_command(core.gh_issue_view_state_cmd(REPO, issue_number), {
    stdout = string.format(
      '{"title":"Issue","state":"OPEN","labels":[{"name":"fkst-dev:enabled"}],"comments":[%s],"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
      table.concat(comments, ",")
    ),
    stderr = "", exit_code = 0,
  })
end

return {
  -- The reproduction: a pr-open holder whose PR base is not this instance's
  -- integration branch is excluded from the cap, so a fresh start is admitted.
  -- Before the fix this returned (false, "wip-cap-reached", 1, 1).
  test_unmanaged_base_holder_is_excluded_from_cap = function()
    mock_env(1)
    mock_wip_list({ 51 })
    mock_wip_state(51, "pr-open", "integration")

    local allowed, reason, count, max = core.wip_capacity_allows_start(REPO, 42)
    t.eq(allowed, true)
    t.eq(reason, "wip-cap-available")
    t.eq(count, 0)
    t.eq(max, 1)
  end,

  -- Regression: a holder on the managed integration branch still counts, so the cap
  -- still provides real backpressure (no over-admission).
  test_managed_base_holder_still_counts = function()
    mock_env(1)
    mock_wip_list({ 51 })
    mock_wip_state(51, "pr-open", INTEGRATION)

    local allowed, reason, count = core.wip_capacity_allows_start(REPO, 42)
    t.eq(allowed, false)
    t.eq(reason, "wip-cap-reached")
    t.eq(count, 1)
  end,

  -- Regression: an active holder with no PR yet (implementing, no pr-link) cannot have
  -- its base inspected and must still count -- it is a live local implementation.
  test_holder_without_pr_link_still_counts = function()
    mock_env(1)
    mock_wip_list({ 51 })
    mock_wip_state(51, "implementing", nil)

    local allowed, reason, count = core.wip_capacity_allows_start(REPO, 42)
    t.eq(allowed, false)
    t.eq(reason, "wip-cap-reached")
    t.eq(count, 1)
  end,
}
