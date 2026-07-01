local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local operator_commands = require("devloop.operator_commands")
local opts = h.opts
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local function mock_repo_env(repo)
  t.mock_command('printf %s "$FKST_DEVLOOP_UPSTREAM_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_INTEGRATION_BRANCH"', { stdout = "dev", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_DEVLOOP_ROLLUP_MERGE"', { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = repo or "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', { stdout = "", stderr = "", exit_code = 0 })
end

local function source_ref(number)
  return entity_lib.issue_source_ref("owner/repo", number or 42)
end

local function event(fields)
  local f = fields or {}
  local number = f.number or 42
  local updated_at = f.updated_at or "2026-06-03T01:02:03Z"
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = "owner/repo",
      number = number,
      title = f.title or "Issue",
      state = f.state or "OPEN",
      labels = f.labels or {},
      updated_at = updated_at,
      dedup_key = "owner/repo#issue#" .. tostring(number) .. "@" .. tostring(updated_at),
      source_ref = source_ref(number),
    },
    source_ref = source_ref(number),
  }
end

local function mock_issue(fields)
  local f = fields or {}
  entity_read_mocks.mock_issue_view_selector(t, {
    number = f.number or 42,
    title = f.title or "Issue",
    body = f.body or "",
    updated_at = f.updated_at or "2026-06-03T01:02:03Z",
    state = f.state or "OPEN",
    labels = f.labels or {},
    comments = f.comments or {},
    assignees = f.assignees or { "fkst-test-bot" },
    author_login = f.author_login or "fkst-test-bot",
  }, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author")
end

local function trusted_reintake_command(id)
  return {
    id = id or "IC_reintake_1",
    body = "fkst: reintake",
    author_login = devloop_base.trusted_bot_login(),
    created_at = "2026-06-04T03:00:00Z",
  }
end

local function run_admission(run_opts)
  return t.run_department("departments/admission/main.lua", event(), run_opts)
end

local function assert_queues(raises, expected)
  t.eq(#raises, #expected)
  for index, queue in ipairs(expected) do
    t.eq(raises[index].queue, queue)
  end
end

local function assert_source_ref(payload, ref)
  t.eq(payload.source_ref.kind, "external")
  t.eq(payload.source_ref.ref, ref or "owner/repo#issue/42")
end

local function assert_issue_claim(payload)
  t.eq(payload.claim.owner, "fkst-test-bot")
  assert_source_ref(payload.claim, "owner/repo#issue/42")
end

local function assert_common_issue_request(payload, schema, dedup_key)
  t.eq(payload.schema, schema)
  t.eq(payload.repo, "owner/repo")
  t.eq(tostring(payload.issue_number), "42")
  t.eq(payload.dedup_key, dedup_key)
  assert_source_ref(payload)
  assert_issue_claim(payload)
end

local function assert_no_codex_or_issue_edit()
  t.eq(h.count_calls("codex exec"), 0)
  t.eq(h.count_calls("gh issue edit"), 0)
end

local function assert_admission_candidate_delivery_key(payload)
  local prefix = "intake-candidate/"
    .. tostring(payload.proposal_id)
    .. "/"
    .. tostring(payload.effect_id)
    .. "/"
  t.is_true(tostring(payload.dedup_key or ""):sub(1, #prefix) == prefix)
  local delivery_version = tostring(payload.dedup_key):sub(#prefix + 1)
  t.is_true(delivery_version:match("^%d+$") ~= nil)
  t.eq(payload.dedup_key, core.intake_candidate_delivery_dedup_key(
    payload.proposal_id,
    payload.effect_id,
    delivery_version
  ))
end

return {
  test_golden_admission_open_unmanaged_raises_candidate = function()
    h.mock_bot_env()
    mock_repo_env()
    mock_issue({ number = 42, labels = {}, title = "Issue", body = "" })

    local result = run_admission(opts("golden-admission-open-unmanaged"))

    t.eq(result.exit_code, 0)
    assert_queues(result.raises, { "devloop_intake_candidate" })
    local payload = result.raises[1].payload
    t.eq(payload.schema, "github-devloop.intake-candidate.v1")
    t.eq(payload.repo, "owner/repo")
    t.eq(payload.issue_number, "42")
    t.eq(payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(payload.effect_id, devloop_base.intake_decision_dedup_key(payload.proposal_id, { title = "Issue", body = "" }))
    assert_admission_candidate_delivery_key(payload)
    assert_source_ref(payload)
  end,

  test_golden_admission_refuses_reintake_without_existing_intake = function()
    local command = trusted_reintake_command("IC_reintake_no_marker")
    local command_fact = operator_commands.operator_command_fact(core, { command }, "reintake")
    h.mock_bot_env()
    mock_repo_env()
    mock_issue({ number = 42, labels = {}, comments = { command } })

    local result = run_admission(opts("golden-admission-reintake-refusal"))

    t.eq(result.exit_code, 0)
    assert_queues(result.raises, { "github-proxy.github_issue_comment_request" })
    local request = result.raises[1].payload
    assert_common_issue_request(request, "github-proxy.v1", base_ids.dedup_key({
      "operator-command",
      "comment",
      command_fact.key,
      "refused",
      "reintake requires an existing intake decision",
    }))
    t.is_true(request.body:find("github-devloop operator command refused: reintake requires an existing intake decision", 1, true) ~= nil)
    t.is_true(request.body:find('command="reintake"', 1, true) ~= nil)
    t.is_true(request.body:find('outcome="refused"', 1, true) ~= nil)
  end,

  test_golden_admission_claim_skip_known_state_hold_and_foreign_assignee = function()
    local cases = {
      { name = "known-state", view = { labels = { "fkst-dev:thinking" } } },
      { name = "hold", view = { labels = { "fkst-dev:hold" } } },
      { name = "foreign-assignee", view = { labels = {}, assignees = { "other-bot" } } },
    }
    for _, case in ipairs(cases) do
      h.mock_bot_env()
      mock_repo_env()
      mock_issue(case.view)

      local result = run_admission(opts("golden-admission-skip-" .. case.name))

      t.eq(result.exit_code, 0)
      t.eq(#result.raises, 0)
      assert_no_codex_or_issue_edit()
    end
  end,
}
