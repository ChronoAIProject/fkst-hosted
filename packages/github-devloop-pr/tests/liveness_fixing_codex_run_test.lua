local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local requests_review = require("devloop.requests.review")
local convergence_shared = require("devloop.convergence.shared")
local contract_time = require("contract.time")
local transition_version = require("contract.transition_version")
local h = require("tests.devloop_helpers")
local conv_rounds = require("devloop.convergence.rounds")
local conv_attempts = require("devloop.convergence.attempts")
local m_rae = require("devloop.restart_actionable_epoch")
local t = h.t
local core = h.core
local opts = h.opts
local replay_fields = require("devloop.replay_fields")
local fixing = h.fixing
local run_fix = h.run_fix
local mock_issue_fix_for_event = h.mock_issue_fix_for_event
local mock_pr_fix = h.mock_pr_fix
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_existing_fix_worktree = h.mock_existing_fix_worktree
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")

local repo = "owner/repo"
local proposal_id = "github-devloop/issue/owner/repo/42"

local function restart_transition_row(state_name)
  return replay_fields.restart_transition_row(core.restart_transition_table(), state_name)
end

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function json_string(value)
  return tostring(value)
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
end

local function json_value(value)
  if type(value) == "number" then
    return tostring(value)
  end
  if type(value) == "boolean" then
    return value and "true" or "false"
  end
  if value == nil then
    return "null"
  end
  return '"' .. json_string(value) .. '"'
end

local function json_object(record)
  local parts = {}
  for key, value in pairs(record or {}) do
    table.insert(parts, '"' .. json_string(key) .. '":' .. json_value(value))
  end
  table.sort(parts)
  return "{" .. table.concat(parts, ",") .. "}"
end

local function seed_codex_run(run_opts, record)
  local root = run_opts and run_opts.env and run_opts.env.FKST_RUNTIME_LOG_DIR
  if root == nil or root == "" then
    error("github-devloop-pr test: FKST_RUNTIME_LOG_DIR is required to seed codex status")
  end
  local dir = root .. "/codex"
  os.execute("mkdir -p " .. string.format("%q", dir))
  local path = dir .. "/" .. tostring(record.run_id or nonce()) .. ".log"
  local file = assert(io.open(path, "a"))
  file:write("CODEX_STATUS:" .. json_object(record) .. "\n")
  file:close()
  return path
end

local function live_run_timing()
  local started = now() - 60
  return os.date("!%Y-%m-%dT%H:%M:%SZ", started),
    started * 1000,
    (now() + 3600) * 1000
end

local function seed_role_codex_run(run_opts, role, run_proposal_id, dedup_key, extra)
  local started_at, started_at_ms, lease_expires_at_ms = live_run_timing()
  local record = {
    run_id = nonce(),
    role = role,
    dept = role,
    proposal_id = run_proposal_id,
    dedup_key = dedup_key,
    status = "running",
    started_at = started_at,
    started_at_ms = started_at_ms,
    lease_expires_at_ms = lease_expires_at_ms,
    timeout_seconds = 3600,
    log_path = "/tmp/fkst-packages-test/codex.log",
    cmd_line = "codex exec -",
  }
  for key, value in pairs(extra or {}) do
    record[key] = value
  end
  seed_codex_run(run_opts, record)
  return record
end

local function trusted_comment(body, created_at)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = created_at or "2026-06-03T00:00:00Z",
  }
end

local function fixing_state(event, version, created_at)
  return {
    state = "fixing",
    version = version or event.version,
    proposal_id = event.proposal_id,
    marker_created_at = created_at or "2026-06-03T00:00:00Z",
  }
end

local function fixing_comments(event, version)
  return {
    trusted_comment(m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")),
    trusted_comment(core.state_marker(event.proposal_id, "fixing", version or event.version)),
    trusted_comment(m_builders.review_result_marker(core, event.review_proposal_id, event.proposal_id, "reject", event.review_dedup_key, 1, "missing regression guard")),
    trusted_comment(m_builders.merge_gate_marker(core, 
      event.proposal_id,
      event.pr_number,
      event.version,
      event.review_proposal_id,
      event.review_dedup_key,
      event.reviewed_head_sha,
      nil,
      "missing regression guard"
    )),
  }
end

local function review_meta_comments(event, version)
  return {
    trusted_comment(m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")),
    trusted_comment(core.state_marker(event.proposal_id, "review-meta", version or event.version)),
    trusted_comment(m_builders.review_meta_marker(core, event.proposal_id, event.dedup_key)),
    trusted_comment(m_builders.review_result_marker(core, event.review_proposal_id, event.proposal_id, "reject", event.review_dedup_key, 1, "missing regression guard")),
    trusted_comment(conv_rounds.review_converge_round_marker(core,
      event.review_proposal_id,
      event.proposal_id,
      event.version,
      "def456",
      convergence_shared.source_ref_digest(entity_lib.pr_source_ref(repo, event.pr_number)),
      event.n,
      event.review_dedup_key,
      "Need a meta decision.",
      { { angle = "minimal", verdict = "no", digest = "gap" } }
    )),
  }
end

local function timeout_attempt_v2_comment(row, state, comments, round)
  local facts = {
    proposal_id = state.proposal_id,
    current = { comments = comments or {} },
    current_pr = { comments = comments or {}, head_sha = "def456" },
    source_ref = entity_lib.pr_source_ref(repo, 7),
  }
  local eval = m_rae.actionable_epoch_resolve(core, row, state, facts, contract_time.iso_timestamp_epoch_seconds("2026-06-03T03:00:00Z"))
  return trusted_comment(conv_attempts.timeout_attempt_v2_marker(core,
    proposal_id,
    row.from_state,
    row.liveness_class_id,
    eval.generation_key,
    round,
    entity_lib.pr_source_ref(repo, 7)
  ))
end

local function timeout_facts(event, state, comments)
  return {
    proposal_id = event.proposal_id,
    source_ref = entity_lib.pr_source_ref(repo, event.pr_number),
    current = { comments = {} },
    current_pr = {
      comments = comments,
      head_sha = event.reviewed_head_sha,
    },
    link = {
      proposal_id = event.proposal_id,
      pr_number = event.pr_number,
      branch = "devloop-owner-repo-42-01HY",
      impl_version = event.version,
      base_branch = "dev",
    },
    snapshot = {
      comments = comments,
      prs = { { number = event.pr_number, current = { comments = comments, head_sha = event.reviewed_head_sha } } },
      state = state,
    },
    head_sha = event.reviewed_head_sha,
    fresh_current_state = state,
    now_seconds = contract_time.iso_timestamp_epoch_seconds("2026-06-03T03:00:00Z"),
  }
end

local function capture_raises(fn)
  local raised = {}
  local original = core.log_raise
  core.log_raise = function(_, _, queue, payload)
    table.insert(raised, { queue = queue, payload = payload })
  end
  local ok, err = pcall(fn)
  core.log_raise = original
  if not ok then
    error(err)
  end
  return raised
end

local function captured_raise(raised, queue, predicate)
  for _, item in ipairs(raised or {}) do
    if item.queue == queue and (predicate == nil or predicate(item.payload, item)) then
      return item
    end
  end
  return nil
end

local function with_codex_runs(running, fn)
  local original = fkst.codex_runs
  fkst.codex_runs = function()
    return { running = running or {}, recent = {} }
  end
  local ok, err = pcall(fn)
  fkst.codex_runs = original
  if not ok then
    error(err)
  end
end

local function mock_repo_and_empty_issue_list()
  t.mock_command(devloop_base.read_env_command("FKST_GITHUB_REPO"), {
    stdout = repo,
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(core.gh_issue_list_observe_cmd(repo), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_list()
  t.mock_command(core.gh_pr_list_observe_cmd(repo), {
    stdout = '[{"number":7,"state":"open","updated_at":"2026-06-04T01:02:03Z"}]\n',
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_claim()
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = 42,
    labels = { "fkst-dev:enabled", "fkst-dev:fixing" },
    comments = {},
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
    register_all_views = true,
    times = 1,
  })
end

local function mock_pr_state(comments)
  entity_read_mocks.mock_pr_read_forms(t, {
    repo = repo,
    number = 7,
    head = "devloop-owner-repo-42-01HY",
    head_sha = "def456",
    base_branch = "dev",
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    comments = comments,
    labels = {},
    register_all_views = true,
    times = 3,
  })
  entity_read_mocks.mock_pr_view_selector(t, {
    repo = repo,
    number = 7,
    head = "devloop-owner-repo-42-01HY",
    head_sha = "def456",
    base_branch = "dev",
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    comments = comments,
    labels = {},
  }, entity_read_mocks.pr_origin_selector)
end

local function reject_comment(event)
  return requests_review.build_review_result_comment_request(core,
    repo,
    "42",
    event.proposal_id,
    event.version,
    {
      proposal_id = event.review_proposal_id,
      decision = "reject",
      body = "Reject because parser must fail closed.",
      blocking_gap = "missing regression guard",
      dedup_key = event.review_dedup_key,
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    },
    event.source_ref
  ).body
end

local function mock_fix_dispatch_context(event, branch, rejection)
  mock_bot_env()
  mock_write_env("1")
  mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
    core.state_marker(event.proposal_id, "fixing", event.version),
    rejection,
  }, branch, event.version)
  mock_pr_fix({ m_builders.pr_origin_marker(core, event.proposal_id, "42", branch, event.version, "dev") }, branch, event.reviewed_head_sha)
end

local function run_liveness_scan(name, run_opts)
  return t.run_department("departments/liveness_scan/main.lua", {
    queue = "devloop_liveness_tick",
    payload = { schema = "github-devloop.tick.v1" },
    ts = "2026-06-04T01:32:03Z",
  }, run_opts or opts(name or "fixing-codex-run-liveness"))
end

local function run_timeout_reconcile(payload, comments, name)
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = 42,
    labels = { "fkst-dev:enabled", "fkst-dev:fixing" },
    comments = {},
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
    register_all_views = true,
    times = 1,
  })
  entity_read_mocks.mock_pr_read_forms(t, {
    repo = repo,
    number = 7,
    head = "devloop-owner-repo-42-01HY",
    head_sha = "def456",
    base_branch = "dev",
    state = "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    comments = comments,
    labels = {},
    register_all_views = true,
    times = 3,
  })
  return t.run_department("departments/reconcile/main.lua", {
    queue = "devloop_timeout_reconcile",
    payload = payload,
  }, opts(name or "fixing-timeout-reconcile"))
end

return {
  test_fixing_live_codex_run_defers_without_redrive_or_timeout_attempt = function()
    local event = fixing()
    local row = restart_transition_row("fixing")
    local state = fixing_state(event)
    local comments = fixing_comments(event)
    local facts = timeout_facts(event, state, comments)
    with_codex_runs({
      {
        run_id = "fix-live",
        role = "fix",
        proposal_id = event.proposal_id,
        dedup_key = event.version,
        status = "running",
        lease_expires_at_ms = (now() + 3600) * 1000,
      },
    }, function()
      local receiver = core.restart_row_receiver_liveness(row, state, facts, facts.now_seconds)
      t.eq(receiver.action, "defer")
      t.eq(receiver.signal.family, "codex_run:v1")
      local due = core.liveness_timeout_due_with_facts(row, state, facts, facts.now_seconds)
      t.eq(due, false)
      local raised = capture_raises(function()
        local handled = core.maybe_timeout_redrive_from_table("liveness_scan", {
          repo = repo,
          number = event.pr_number,
          source_ref = entity_lib.pr_source_ref(repo, event.pr_number),
        }, state, row, facts)
        t.eq(handled, true)
      end)
      t.eq(captured_raise(raised, "devloop_fixing"), nil)
      t.eq(captured_raise(raised, "devloop_timeout_reconcile"), nil)
      t.eq(captured_raise(raised, "github-proxy.github_pr_comment_request", function(payload)
        return tostring(payload.body or ""):find("fkst:github-devloop:timeout-attempt", 1, true) ~= nil
      end), nil)
    end)
  end,

  test_fixing_no_codex_run_over_budget_escalates_to_blocked_with_why = function()
    local event = fixing()
    local row = restart_transition_row("fixing")
    local state = fixing_state(event, event.version .. "/timeout/fixing/2")
    local comments = fixing_comments(event, state.version)
    table.insert(comments, timeout_attempt_v2_comment(row, state, comments, 1))
    table.insert(comments, timeout_attempt_v2_comment(row, state, comments, 2))
    local facts = timeout_facts(event, state, comments)
    with_codex_runs({}, function()
      local due, age = core.liveness_timeout_due_with_facts(row, state, facts, facts.now_seconds)
      t.eq(due, true)
      t.eq(age, 180)
      local raised = capture_raises(function()
        local handled = core.maybe_timeout_redrive_from_table("liveness_scan", {
          repo = repo,
          number = event.pr_number,
          source_ref = entity_lib.pr_source_ref(repo, event.pr_number),
        }, state, row, facts)
        t.eq(handled, true)
      end)
      local reconcile = captured_raise(raised, "devloop_timeout_reconcile")
      t.is_true(reconcile ~= nil)
      t.eq(reconcile.payload.state, "fixing")
      t.eq(reconcile.payload.issue_version, state.version)
      t.eq(reconcile.payload.round, 3)

      local reconciled = run_timeout_reconcile(reconcile.payload, comments, "fixing-no-codex-run-blocked")
      t.eq(reconciled.exit_code, 0)
      local comment = h.find_raise(reconciled.raises, "github-proxy.github_pr_comment_request")
      t.is_true(comment ~= nil)
      t.is_true(tostring(comment.payload.body or ""):find('state="blocked"', 1, true) ~= nil)
      t.is_true(tostring(comment.payload.body or ""):find("state-output-obligation-timeout", 1, true) ~= nil)
    end)
  end,

  test_liveness_scan_fixing_live_codex_run_drops_redrive = function()
    local event = fixing()
    local run_opts = opts("liveness-scan-fixing-live-codex")
    local comments = fixing_comments(event)
    seed_role_codex_run(run_opts, "fix", event.proposal_id, event.version)
    mock_repo_and_empty_issue_list()
    mock_pr_list()
    mock_issue_claim()
    mock_pr_state(comments)

    local result = run_liveness_scan("liveness-scan-fixing-live-codex", run_opts)
    t.eq(result.exit_code, 0)
    t.eq(h.find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(h.find_raise(result.raises, "devloop_timeout_reconcile"), nil)
    t.eq(h.find_raise(result.raises, "github-proxy.github_pr_comment_request", function(payload)
      return tostring(payload.body or ""):find("fkst:github-devloop:timeout-attempt", 1, true) ~= nil
    end), nil)
  end,

  test_fixing_dispatch_with_live_run_without_completion_markers_skips_redelivery = function()
    local event = fixing()
    local branch = devloop_base.implement_branch(repo, "42", event.version)
    local rejection = reject_comment(event)
    local run_opts = opts("fixing-dispatch-live-run-no-marker", { FKST_GITHUB_WRITE = "1" })
    seed_role_codex_run(run_opts, "fix", event.proposal_id, event.version)
    mock_fix_dispatch_context(event, branch, rejection)
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
    mock_existing_fix_worktree(branch, event.reviewed_head_sha)
    mock_implement_codex(0, "duplicate fix should not spawn")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_fix_dispatch_context(event, branch, rejection)
    mock_git_push(branch)
    mock_pr_fix({ m_builders.pr_origin_marker(core, event.proposal_id, "42", branch, event.version, "dev") }, branch, "feedface")

    local result = run_fix(event, run_opts)
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 0)
    t.eq(count_calls("git worktree add --force -B"), 0)
    t.eq(count_calls("git worktree remove --force"), 0)
    t.eq(count_calls("git worktree prune"), 0)
    t.eq(count_calls("merge --no-edit"), 0)
  end,

  test_fixing_codex_run_match_preserves_fix_suffix = function()
    local event = fixing()
    local row = restart_transition_row("fixing")
    local state = fixing_state(event)
    local facts = timeout_facts(event, state, fixing_comments(event))
    with_codex_runs({
      {
        run_id = "base-only-wrong",
        role = "fix",
        proposal_id = event.proposal_id,
        dedup_key = transition_version.strip_suffixes(event.version),
        status = "running",
      },
    }, function()
      local signal = core.restart_row_liveness_signal(row, state, facts, facts.now_seconds)
      t.eq(signal.live, false)
      t.eq(signal.expected_dedup_key, event.version)
    end)
  end,

  test_review_meta_live_codex_run_defers_without_redrive_or_timeout_attempt = function()
    local event = h.review_meta_event()
    local row = restart_transition_row("review-meta")
    local state = {
      state = "review-meta",
      version = event.version,
      proposal_id = event.proposal_id,
      marker_created_at = "2026-06-03T00:00:00Z",
    }
    local comments = review_meta_comments(event)
    local facts = timeout_facts(event, state, comments)
    with_codex_runs({
      {
        run_id = "review-meta-live",
        role = "review-meta",
        proposal_id = event.proposal_id,
        dedup_key = event.version,
        status = "running",
        lease_expires_at_ms = (now() + 3600) * 1000,
      },
    }, function()
      local receiver = core.restart_row_receiver_liveness(row, state, facts, facts.now_seconds)
      t.eq(receiver.action, "defer")
      t.eq(receiver.signal.family, "codex_run:v1")
      local raised = capture_raises(function()
        local handled = core.maybe_timeout_redrive_from_table("liveness_scan", {
          repo = repo,
          number = event.pr_number,
          source_ref = entity_lib.pr_source_ref(repo, event.pr_number),
        }, state, row, facts)
        t.eq(handled, true)
      end)
      t.eq(captured_raise(raised, "devloop_review_meta"), nil)
      t.eq(captured_raise(raised, "devloop_timeout_reconcile"), nil)
      t.eq(captured_raise(raised, "github-proxy.github_pr_comment_request", function(payload)
        return tostring(payload.body or ""):find("fkst:github-devloop:timeout-attempt", 1, true) ~= nil
      end), nil)
    end)
  end,

  test_review_meta_no_codex_run_over_budget_escalates = function()
    local event = h.review_meta_event()
    local row = restart_transition_row("review-meta")
    local state = {
      state = "review-meta",
      version = event.version .. "/timeout/review-meta/2",
      proposal_id = event.proposal_id,
      marker_created_at = "2026-06-03T00:00:00Z",
    }
    local comments = review_meta_comments(event, state.version)
    table.insert(comments, timeout_attempt_v2_comment(row, state, comments, 1))
    table.insert(comments, timeout_attempt_v2_comment(row, state, comments, 2))
    local facts = timeout_facts(event, state, comments)
    with_codex_runs({}, function()
      local due, age = core.liveness_timeout_due_with_facts(row, state, facts, facts.now_seconds)
      t.eq(due, true)
      t.eq(age, 180)
      local raised = capture_raises(function()
        local handled = core.maybe_timeout_redrive_from_table("liveness_scan", {
          repo = repo,
          number = event.pr_number,
          source_ref = entity_lib.pr_source_ref(repo, event.pr_number),
        }, state, row, facts)
        t.eq(handled, true)
      end)
      local reconcile = captured_raise(raised, "devloop_timeout_reconcile")
      t.is_true(reconcile ~= nil)
      t.eq(reconcile.payload.state, "review-meta")
      t.eq(reconcile.payload.issue_version, state.version)
      t.eq(reconcile.payload.round, 3)
    end)
  end,
}
