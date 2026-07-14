local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local graph = require("testkit.graph")
local t = fkst.test
local core = require("core")
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local m_builders = require("devloop.markers.builders")
local m_state = require("devloop.state")
local gh_argv = require("testkit.gh_argv_mock")

gh_argv.install(t, core)

local repo = "owner/repo"
local issue_number = 42
local updated_at = "2026-06-03T01:02:03Z"
local title = "Recover intake after terminal DLQ"
local body = "Exercise safe replay after the intake judge exhausted retries."
local owner = "fkst-test-bot"
local proposal_id = base_ids.proposal_id(repo, issue_number)
local target_queue = "github-devloop-intake.devloop_intake_candidate"
local target_dept = "github-devloop-intake-default.intake_judge"
local system_path = "/usr/bin:/bin"

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\"'\"'") .. "'"
end

local function command_output(command)
  local handle = assert(io.popen(command .. " 2>&1"))
  local output = handle:read("*a")
  local ok = handle:close()
  return output, ok ~= false and ok ~= nil
end

local function read_command(command)
  local output, ok = command_output(command)
  if not ok then
    error("intake replay run_graph fixture command failed: " .. tostring(command) .. "\n" .. tostring(output))
  end
  return output
end

local function repo_root()
  return (read_command("pwd"):gsub("%s+$", ""))
end

local function framework_bin()
  local bin = os.getenv("BIN") or ""
  if bin == "" then
    error("intake replay run_graph fixture requires BIN")
  end
  return bin
end

local function run_nested_acceptance()
  local root = repo_root()
  local nested_root = (read_command("mktemp -d " .. shell_quote("/tmp/fkst-intake-replay.XXXXXX")):gsub("%s+$", ""))
  local script = table.concat({
    "cd " .. shell_quote(root),
    "eval \"$(BIN=" .. shell_quote(framework_bin()) .. " /bin/bash scripts/composed_test_graph_roots.sh graph github-devloop-intake-default)\"",
    table.concat({
      "BIN=" .. shell_quote(framework_bin()),
      "PATH=" .. shell_quote(system_path),
      "FKST_INTAKE_REPLAY_NESTED=1",
      "FKST_RETRY_DEFAULT_MAX_ATTEMPTS=1",
      "FKST_RETRY_DEFAULT_BASE=1s",
      "FKST_RETRY_DEFAULT_CAP=1s",
      "FKST_RUNTIME_ROOT=" .. shell_quote(nested_root .. "/runtime"),
      "FKST_DURABLE_ROOT=" .. shell_quote(nested_root .. "/durable"),
      shell_quote(framework_bin()),
      "test",
      "--project-root",
      "\"$test_project_root\"",
      "\"${test_pkg_args[@]}\"",
    }, " "),
  }, " && ")
  local command = "/bin/bash -lc " .. shell_quote(script)
  local ok, err = pcall(function()
    read_command(command)
  end)
  read_command("rm -rf " .. shell_quote(nested_root))
  if not ok then
    error(err)
  end
end

local function source_ref()
  return entity_lib.issue_source_ref(repo, issue_number)
end

local function source_row()
  return {
    kind = "external",
    reference = source_ref().ref,
  }
end

local function mock_base_env(times)
  for _ = 1, times or 120 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_REPO"), { stdout = repo, stderr = "", exit_code = 0 })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), { stdout = owner, stderr = "", exit_code = 0 })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_CLAIM_MODE"), { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command(devloop_base.read_env_command("FKST_DEVLOOP_FORK_GRACE_HOURS"), { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_PROXY_POLL_LABEL_PREFIX"', { stdout = "fkst-dev:,fkst-class:", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_GITHUB_PROXY_REPLAY_BUDGET"', { stdout = "1", stderr = "", exit_code = 0 })
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop-intake-replay/runtime",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function assignees_json(assignees)
  local parts = {}
  for _, login in ipairs(assignees or {}) do
    table.insert(parts, '{"login":"' .. tostring(login) .. '"}')
  end
  return "[" .. table.concat(parts, ",") .. "]"
end

local function mock_proxy_poll_lists(assignees)
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/issues?state=open&per_page=100'", {
    stdout = '[[{"number":42,"title":"'
      .. title
      .. '","html_url":"https://github.example/owner/repo/issues/42","updated_at":"'
      .. updated_at
      .. '","state":"open","labels":[{"name":"bug"}],"assignees":'
      .. assignees_json(assignees)
      .. "}]]\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh api --paginate --slurp 'repos/owner/repo/pulls?state=open&per_page=100'", {
    stdout = "[[]]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function seed_proxy_cache()
  cache_set("github-proxy/issue/" .. tostring(repo) .. "/" .. tostring(issue_number), updated_at)
end

local function mock_issue_view(assignees, comments, times)
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = issue_number,
    title = title,
    body = body,
    updated_at = updated_at,
    state = "OPEN",
    labels = { "bug" },
    comments = comments or {},
    assignees = assignees,
    author_login = owner,
  }, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author", times or 1)
end

local function mock_claim_verify()
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = issue_number,
    assignees = { owner },
    author_login = owner,
  }, "assignees,author")
end

local function mock_claim_write()
  t.mock_command("gh issue edit '42' --repo 'owner/repo' --add-assignee 'fkst-test-bot'", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_devloop_observe_issue_read()
  entity_read_mocks.mock_issue_read_with_defaults(t, { "bug" }, {}, {
    repo = repo,
    number = issue_number,
    title = title,
    body = body,
    updated_at = updated_at,
    state = "OPEN",
    assignees = { owner },
    author_login = owner,
    times = 2,
  })
end

local function mock_context_bundle()
  local ok = { stdout = "", stderr = "", exit_code = 0 }
  for _ = 1, 6 do
    t.mock_command("test -d", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("test -e", { stdout = "", stderr = "", exit_code = 1 })
  end
  t.mock_command("install -d -m 0755", ok)
  t.mock_command("mktemp -d", {
    stdout = "/tmp/fkst-packages-test/github-devloop-intake-replay/runtime/context/.bundle-tmp.replay\n",
    stderr = "",
    exit_code = 0,
  })
  entity_read_mocks.mock_issue_view_raw_selector(t, {
    repo = repo,
    number = issue_number,
  }, "title,body,updatedAt,labels,comments,state", {
    stdout = entity_read_mocks.issue_view_stdout({
      repo = repo,
      number = issue_number,
      title = title,
      body = body,
      updated_at = updated_at,
      state = "OPEN",
      labels = {},
      comments = {},
      author_login = owner,
    }),
  })
  entity_read_mocks.mock_issue_board_digest_list_raw(t, repo, { stdout = "[]\n" })
  entity_read_mocks.mock_issue_list_raw_command(t, core.gh_issue_list_recent_closed_cmd(repo, 30), { stdout = "[]\n" })
  t.mock_command("gh pr list", { stdout = "[]\n", stderr = "", exit_code = 0 })
  for _ = 1, 10 do
    t.mock_command("touch ", ok)
    t.mock_command("printf %s '", ok)
    t.mock_command(" > ", ok)
    t.mock_command("test -r", ok)
    t.mock_command("wc -c < ", { stdout = "1\n", stderr = "", exit_code = 0 })
  end
  t.mock_command("python3 -c", ok)
end

local function mock_codex_failure()
  for _ = 1, 4 do
    t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  end
  t.mock_command("codex exec", {
    stdout = "",
    stderr = "deterministic intake failure",
    exit_code = 1,
  })
end

local function find_raise_from_step(step, queue)
  for _, raised in ipairs(step.raises or {}) do
    if raised.queue == queue then
      return raised
    end
  end
  return nil
end

local function observe_snapshot(deliveries, dead_letters)
  return {
    schema_version = 1,
    generated_at_ms = 1781830860000,
    source = {
      durable_root = "/tmp/fkst-durable",
      database = "/tmp/fkst-durable/delivery.redb",
      read_semantics = "single read transaction",
      history_semantics = "delivery queue snapshot only",
    },
    limits = { max_deliveries = 10000, max_dead_letters = 10000 },
    truncated = { deliveries = false, dead_letters = false },
    queues = json.decode("[]"),
    deliveries = deliveries or json.decode("[]"),
    dead_letters = dead_letters or json.decode("[]"),
  }
end

local function terminal_row(delivery_id, attempts)
  return {
    delivery_id = delivery_id,
    queue = target_queue,
    dept = target_dept,
    source = source_row(),
    attempts = attempts or 1,
    permanent = true,
    replayable = false,
    dead_at_ms = 1781830861000,
  }
end

local function live_row()
  return {
    delivery_id = "opaque-live-delivery",
    queue = target_queue,
    dept = target_dept,
    source = source_row(),
    status = "retrying",
  }
end

local function expect_no_candidate(trace)
  local raised = graph.find_raise(trace, target_queue)
  t.is_nil(raised)
end

local function issue_refetch_call_count()
  local count = 0
  local fields = "--json title,body,createdAt,updatedAt,labels,comments,state,assignees,author"
  for _, call in ipairs(t.command_calls()) do
    local rendered = tostring(call.rendered or call.command or call.cmd or call)
    if rendered:find("gh issue view", 1, true) ~= nil
      and rendered:find(fields, 1, true) ~= nil then
      count = count + 1
    end
  end
  return count
end

local function run_poll(max_steps)
  return t.run_graph("github-proxy.github_poll", { max_steps = max_steps or 12 })
end

local function assert_observed_admission(trace)
  graph.assert_covers(trace, {
    "github-proxy.github_poll_tick -> github-proxy.github_poll",
    "github-proxy.github_issue_observed -> github-devloop-intake.admission",
  })
end

return os.getenv("FKST_INTAKE_REPLAY_NESTED") == "1" and {
  test_run_graph_replays_intake_after_terminal_dlq_only_when_inactive = function()
    mock_base_env()

    mock_proxy_poll_lists()
    mock_issue_view({}, {})
    mock_claim_write()
    mock_claim_verify()
    mock_issue_view({ owner }, {})
    mock_devloop_observe_issue_read()
    mock_context_bundle()
    mock_codex_failure()
    local first = run_poll(16)

    graph.assert_covers(first, {
      "github-proxy.github_poll_tick -> github-proxy.github_poll",
      "github-proxy.github_entity_changed -> github-devloop-intake.admission",
      "github-devloop-intake.devloop_intake_candidate -> github-devloop-intake-default.intake_judge",
    })
    t.eq(first.status, "quiescent")
    t.eq(first.final.dead_letters, 1)

    local candidate, admission_step = graph.require_raise(first, target_queue)
    t.eq(candidate.payload.proposal_id, proposal_id)
    t.eq(candidate.payload.source_ref.ref, source_ref().ref)
    local judge_step = graph.require_delivery(first, {
      queue = target_queue,
      consumer = target_dept,
    })
    t.eq(judge_step.status, "error")
    t.eq(judge_step.exit_code, 1)
    t.is_true(type(judge_step.delivery_id) == "string" and judge_step.delivery_id ~= "")
    t.is_true(admission_step.queue == "github-proxy.github_entity_changed")

    t.mock_observe(observe_snapshot({ live_row() }, nil))
    seed_proxy_cache()
    mock_proxy_poll_lists({ owner })
    local refetches_before_live_blocked = issue_refetch_call_count()
    local live_blocked = run_poll(8)
    t.eq(live_blocked.status, "quiescent")
    t.eq(live_blocked.final.dead_letters, 0)
    assert_observed_admission(live_blocked)
    expect_no_candidate(live_blocked)
    t.eq(issue_refetch_call_count(), refetches_before_live_blocked)

    local terminal = terminal_row(judge_step.delivery_id, 1)
    t.mock_observe(observe_snapshot(nil, { terminal }))
    seed_proxy_cache()
    mock_proxy_poll_lists({ owner })
    mock_issue_view({ owner }, {})
    mock_issue_view({ owner }, {})
    mock_context_bundle()
    mock_codex_failure()
    local replayed = run_poll(16)
    t.eq(replayed.status, "quiescent")
    assert_observed_admission(replayed)
    graph.assert_covers(replayed, {
      "github-devloop-intake.devloop_intake_candidate -> github-devloop-intake-default.intake_judge",
    })
    local replay_candidate = graph.require_raise(replayed, target_queue)
    local expected_key = base_ids.dedup_key({
      "intake-replay",
      proposal_id,
      judge_step.delivery_id,
      "1",
    })
    t.eq(replay_candidate.payload.dedup_key, expected_key)
    t.eq(replay_candidate.payload.source_ref.ref, source_ref().ref)
    local replay_judge = graph.require_delivery(replayed, {
      queue = target_queue,
      consumer = target_dept,
    })
    t.eq(replay_judge.status, "error")
    t.is_true(replay_judge.delivery_id ~= judge_step.delivery_id)

    t.mock_observe(observe_snapshot(nil, { terminal }))
    seed_proxy_cache()
    mock_proxy_poll_lists({ owner })
    mock_issue_view({ owner }, {})
    local repeated = run_poll(8)
    t.eq(repeated.status, "quiescent")
    assert_observed_admission(repeated)
    expect_no_candidate(repeated)

    t.mock_observe(observe_snapshot(nil, { terminal }))
    seed_proxy_cache()
    mock_proxy_poll_lists({ owner })
    mock_issue_view({ owner }, {
      m_builders.intake_decision_marker(proposal_id, "decline", "intake/github-devloop/issue/owner/repo/42/v1", "standard"),
    })
    local progressed = run_poll(8)
    t.eq(progressed.status, "quiescent")
    assert_observed_admission(progressed)
    expect_no_candidate(progressed)

    t.mock_observe(observe_snapshot(nil, { terminal }))
    seed_proxy_cache()
    mock_proxy_poll_lists({ owner })
    mock_issue_view({ owner }, {
      m_state.state_marker(proposal_id, "thinking", "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"),
    })
    local state_progressed = run_poll(8)
    t.eq(state_progressed.status, "quiescent")
    assert_observed_admission(state_progressed)
    expect_no_candidate(state_progressed)
  end,
} or {
  test_run_graph_intake_replay_after_terminal_dlq_nested = function()
    run_nested_acceptance()
  end,
}
