local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local graph = require("testkit.graph")
local t = fkst.test
local core = require("core")
local h = require("tests.devloop_base_helpers")
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local repo = "owner/repo"
local issue_number = 42
local updated_at = "2026-06-03T01:02:03Z"
local title = "Add retry backoff to failed widget sync"
local body = "Implement exponential backoff for widget sync retries."
local proposal_id = base_ids.proposal_id(repo, issue_number)
local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"

local function source_ref()
  return entity_lib.issue_source_ref(repo, issue_number)
end

local function initial_event()
  return {
    queue = "github-proxy.github_entity_changed",
    payload = {
      schema = "github-proxy.v1",
      type = "issue",
      repo = repo,
      number = issue_number,
      title = title,
      updated_at = updated_at,
      state = "OPEN",
      dedup_key = "owner/repo#issue/42@2026-06-03T01:02:03Z",
      source_ref = source_ref(),
    },
    source_ref = {
      kind = "external",
      reference = source_ref().ref,
    },
  }
end

local function mock_env()
  for _ = 1, 32 do
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_WRITE"), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(devloop_base.read_env_command("FKST_GITHUB_CLAIM_MODE"), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 12 do
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop-intake-default-full-run-graph/runtime",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_issue_reads()
  entity_read_mocks.mock_issue_read_with_defaults(t, {}, {}, {
    repo = repo,
    number = issue_number,
    title = title,
    body = body,
    updated_at = updated_at,
    state = "OPEN",
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
    times = 4,
  })
  entity_read_mocks.mock_issue_view_selector(t, {
    repo = repo,
    number = issue_number,
    title = title,
    body = body,
    updated_at = updated_at,
    state = "OPEN",
    labels = {},
    comments = {},
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
  }, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author", 4)
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
    }),
  }, 3)
end

local function mock_context_bundle()
  local ok = { stdout = "", stderr = "", exit_code = 0 }
  for _ = 1, 6 do
    t.mock_command("test -d", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("test -e", { stdout = "", stderr = "", exit_code = 1 })
  end
  for _ = 1, 2 do
    t.mock_command("install -d -m 0755", ok)
    t.mock_command("mktemp -d", {
      stdout = "/tmp/fkst-packages-test/github-devloop-intake-default-full-run-graph/runtime/context/.bundle-tmp.full\n",
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
      }),
    })
    entity_read_mocks.mock_issue_board_digest_list_raw(t, repo, { stdout = "[]\n" })
    entity_read_mocks.mock_issue_list_raw_command(t, core.gh_issue_list_recent_closed_cmd(repo, 30), { stdout = "[]\n" })
    t.mock_command("gh pr list", { stdout = "[]\n", stderr = "", exit_code = 0 })
  end
  for _ = 1, 16 do
    t.mock_command("touch ", ok)
    t.mock_command("printf %s '", ok)
    t.mock_command(" > ", ok)
    t.mock_command("test -r", ok)
    t.mock_command("wc -c < ", { stdout = "1\n", stderr = "", exit_code = 0 })
  end
  for _ = 1, 2 do
    t.mock_command("python3 -c", ok)
  end
end

local function mock_codex()
  for _ = 1, 4 do
    t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  end
  t.mock_command("codex exec", {
    stdout = "⟦FKST:INTAKE⟧ enable\n⟦FKST:CLASS⟧ standard\n⟦FKST:REASON⟧ run_graph full chain smoke.",
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, 3 do
    t.mock_command("codex exec", {
      stdout = verdict_label .. " approve\n" .. reply_label .. " full chain smoke approves.",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function require_raise_from_step(trace, step_index, queue, predicate)
  for _, raised in ipairs(((trace.steps or {})[step_index] or {}).raises or {}) do
    if raised.queue == queue and (predicate == nil or predicate(raised)) then
      return raised
    end
  end
  error("missing raised queue=" .. tostring(queue) .. " from step_index=" .. tostring(step_index), 2)
end

return {
  test_run_graph_full_intake_execution_chain_reaches_consensus_proposal = function()
    mock_env()
    mock_issue_reads()
    mock_context_bundle()
    mock_codex()

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 12 }))
    graph.assert_covers(trace, {
      "github-proxy.github_entity_changed -> github-devloop-intake.admission",
      "github-devloop-intake.devloop_intake_candidate -> github-devloop-intake-default.intake_judge",
      "github-devloop.devloop_execute_request -> github-devloop.execute_start",
    })

    local _, admission_index = graph.require_delivery(trace, {
      queue = "github-proxy.github_entity_changed",
      consumer = "github-devloop-intake.admission",
    })
    local candidate = require_raise_from_step(trace, admission_index, "github-devloop-intake.devloop_intake_candidate", function(raised)
      local payload = raised.payload or {}
      return payload.schema == "github-devloop.intake-candidate.v1"
        and payload.proposal_id == proposal_id
        and payload.source_ref ~= nil
        and payload.source_ref.ref == source_ref().ref
    end)
    t.eq(candidate.payload.proposal_id, proposal_id)
    t.eq(candidate.payload.source_ref.ref, source_ref().ref)

    local _, judge_index = graph.require_delivery(trace, {
      queue = "github-devloop-intake.devloop_intake_candidate",
      consumer = "github-devloop-intake-default.intake_judge",
    })
    t.is_true(judge_index > admission_index)
    local execution_request = require_raise_from_step(trace, judge_index, "github-devloop.devloop_execute_request", function(raised)
      local payload = raised.payload or {}
      return payload.schema == "github-devloop.execution-request.v1"
        and payload.proposal_id == proposal_id
        and payload.source_ref ~= nil
        and payload.source_ref.ref == source_ref().ref
    end)
    t.eq(execution_request.payload.proposal_id, proposal_id)
    t.eq(execution_request.payload.source_ref.ref, source_ref().ref)

    local execute_start, execute_index = graph.require_delivery(trace, {
      queue = "github-devloop.devloop_execute_request",
      consumer = "github-devloop.execute_start",
    })
    t.eq(execute_start.exit_code, 0)
    t.is_true(execute_index > judge_index)

    local thinking_comment = require_raise_from_step(trace, execute_index, "github-proxy.github_issue_comment_request")
    t.is_true(thinking_comment.payload.body:find(core.state_marker(proposal_id, "thinking", execution_request.payload.dedup_key), 1, true) ~= nil)
    local thinking_label = require_raise_from_step(trace, execute_index, "github-proxy.github_issue_label_request")
    t.eq(thinking_label.payload.add_labels[1], "fkst-dev:thinking")

    local proposal = require_raise_from_step(trace, execute_index, "consensus.proposal", function(raised)
      local payload = raised.payload or {}
      return payload.schema == "consensus.proposal.v1"
        and payload.proposal_id == proposal_id
        and payload.source_ref ~= nil
        and payload.source_ref.ref == source_ref().ref
    end)
    t.eq(proposal.payload.proposal_id, proposal_id)
    t.eq(proposal.payload.dedup_key, execution_request.payload.dedup_key)
    t.eq(proposal.payload.effect_version, execution_request.payload.dedup_key)
    t.eq(proposal.payload.source_ref.ref, source_ref().ref)
    t.eq(proposal.payload.intake_hand_off.proposal_id, proposal_id)
    t.eq(proposal.payload.intake_hand_off.dedup_key, execution_request.payload.dedup_key)
    t.eq(proposal.payload.intake_hand_off.source_ref.ref, source_ref().ref)
  end,
}
