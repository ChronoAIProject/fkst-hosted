local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts

local function mock_repo_env(repo)
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = repo or "owner/repo",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_probe_proof(value)
  t.mock_command('printf %s "$FKST_DEVLOOP_INTAKE_PROBE_PROOF"', {
    stdout = value or "event-fast-path-insufficient",
    stderr = "",
    exit_code = 0,
  })
end

local function json_string(value)
  return h.json_string(value)
end

local function labels_json(labels)
  local rendered = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered, string.format('{"name":"%s"}', json_string(label)))
  end
  return table.concat(rendered, ",")
end

local function comments_json(comments)
  local rendered = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered, h.render_comment(comment))
  end
  return table.concat(rendered, ",")
end

local function issue_probe_json(issues)
  local rendered = {}
  for _, issue in ipairs(issues or {}) do
    table.insert(rendered, string.format(
      '{"number":%d,"title":"%s","created_at":"%s","updated_at":"%s","labels":[%s],"assignees":[%s]}',
      issue.number,
      json_string(issue.title or "Issue"),
      json_string(issue.created_at or "2026-06-03T01:00:00Z"),
      json_string(issue.updated_at or "2026-06-03T01:02:03Z"),
      labels_json(issue.labels or {}),
      issue.assignees_json or '{"login":"fkst-test-bot"}'
    ))
  end
  return "[" .. table.concat(rendered, ",") .. "]"
end

local function mock_probe_issue_list(issues, since)
  t.mock_command(core.gh_issue_list_intake_probe_cmd("owner/repo", 5, since), {
    stdout = issue_probe_json(issues) .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function count_rendered_calls(needle)
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    if tostring(call.rendered or ""):find(needle, 1, true) ~= nil then
      count = count + 1
    end
  end
  return count
end

local function run_probe(run_opts)
  return t.run_department("departments/intake_probe/main.lua", {
    queue = "devloop_intake_probe_tick",
    payload = { schema = "github-devloop.intake-probe-tick.v1" },
  }, run_opts)
end

local function seed_probe_cursor(value, run_opts)
  local result = t.run_department("tests/cache_seed_helpers.lua", {
    queue = "cache_seed",
    payload = {
      key = "github-devloop/intake-probe/created-cursor",
      value = value,
    },
  }, run_opts)
  t.eq(result.exit_code, 0)
end

return {
  test_probe_is_gated_without_event_fast_path_insufficiency_proof = function()
    h.mock_bot_env()
    mock_probe_proof("")

    local result = run_probe(opts("intake-probe-gated-without-proof"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_rendered_calls("repos/owner/repo/issues?state=open"), 0)
  end,

  test_probe_rejects_unknown_proof_value = function()
    h.mock_bot_env()
    mock_probe_proof("temporary-poll")

    local result = run_probe(opts("intake-probe-invalid-proof"))
    t.eq(result.exit_code, 1)
    t.eq(count_rendered_calls("repos/owner/repo/issues?state=open"), 0)
  end,

  test_probe_raises_recent_new_issue_candidate = function()
    h.mock_bot_env()
    mock_probe_proof()
    mock_repo_env()
    mock_probe_issue_list({
      { number = 42, created_at = "2026-06-03T01:01:00Z", updated_at = "2026-06-03T01:02:03Z", labels = {} },
    })

    local result = run_probe(opts("intake-probe-new-issue"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "devloop_intake_candidate")
    t.eq(result.raises[1].payload.issue_number, "42")
    t.eq(result.raises[1].payload.effect_id, core.intake_decision_dedup_key("github-devloop/issue/owner/repo/42", {
      title = "Issue",
      body = "",
    }))
    t.is_true(result.raises[1].payload.dedup_key:find("intake%-candidate/github%-devloop/issue/owner/repo/42", 1, false) ~= nil)
    t.eq(result.raises[1].payload.source_ref.ref, "owner/repo#issue/42")
    t.eq(count_rendered_calls("--json labels,comments,state,assignees"), 0)
  end,

  test_probe_skips_known_devloop_state_label_without_issue_view = function()
    h.mock_bot_env()
    mock_probe_proof()
    mock_repo_env()
    mock_probe_issue_list({
      { number = 42, created_at = "2026-06-03T01:01:00Z", labels = { "fkst-dev:enabled" } },
    })

    local result = run_probe(opts("intake-probe-existing-state-label"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_rendered_calls("--json labels,comments,state,assignees"), 0)
  end,

  test_probe_cursor_uses_since_and_same_second_issue_number_tiebreak = function()
    local run_opts = opts("intake-probe-same-second-cursor")
    seed_probe_cursor("2026-06-03T01:01:00Z\t42", run_opts)
    h.mock_bot_env()
    mock_probe_proof()
    mock_repo_env()
    mock_probe_issue_list({
      { number = 43, created_at = "2026-06-03T01:01:00Z", updated_at = "2026-06-03T01:02:04Z", labels = {} },
      { number = 42, created_at = "2026-06-03T01:01:00Z", updated_at = "2026-06-03T01:02:03Z", labels = {} },
    }, "2026-06-03T01:00:59Z")

    local result = run_probe(run_opts)
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].payload.issue_number, "43")
  end,

  test_probe_full_window_rechecks_until_full_sweep_backstop = function()
    h.mock_bot_env()
    mock_probe_proof()
    mock_repo_env()
    mock_probe_issue_list({
      { number = 50, created_at = "2026-06-03T01:05:00Z", labels = { "fkst-dev:enabled" } },
      { number = 49, created_at = "2026-06-03T01:04:00Z", labels = { "fkst-dev:enabled" } },
      { number = 48, created_at = "2026-06-03T01:03:00Z", labels = { "fkst-dev:enabled" } },
      { number = 47, created_at = "2026-06-03T01:02:00Z", labels = { "fkst-dev:enabled" } },
      { number = 46, created_at = "2026-06-03T01:01:00Z", labels = { "fkst-dev:enabled" } },
    })

    local result = run_probe(opts("intake-probe-full-window"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_rendered_calls("--json labels,comments,state,assignees"), 0)
  end,
}
