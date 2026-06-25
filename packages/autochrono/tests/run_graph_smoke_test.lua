local graph = require("testkit.graph")
local t = fkst.test

local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"

local function issue(issue_number)
  issue_number = issue_number or 42
  return {
    schema = "autochrono.issue.v1",
    repo = "owner/repo",
    issue_number = issue_number,
    title = "Bridge issue",
    url = "https://github.example/owner/repo/issues/" .. tostring(issue_number),
    state = "OPEN",
    updated_at = "2026-06-03T01:02:" .. tostring(issue_number) .. "Z",
    source_ref = {
      kind = "external",
      ref = "owner/repo#issue/" .. tostring(issue_number),
    },
    dedup_key = "owner/repo#issue#" .. tostring(issue_number) .. "@2026-06-03T01:02:" .. tostring(issue_number) .. "Z",
  }
end

local function mock_consensus_approval()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/autochrono-run-graph/runtime",
    stderr = "",
    exit_code = 0,
  })
  for _, angle in ipairs({ "minimal", "structural", "delete" }) do
    t.mock_command("mkdir -p", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("codex exec", {
      stdout = verdict_label .. " approve\n" .. reply_label .. " " .. angle .. " approves.\n",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function initial_event(issue_number)
  return {
    queue = "issue",
    payload = issue(issue_number),
    source_ref = {
      kind = "external",
      reference = "owner/repo#issue/" .. tostring(issue_number or 42),
    },
  }
end

local function run_smoke(issue_number)
  mock_consensus_approval()
  local trace = graph.require_quiescent(graph.run(initial_event(issue_number), { max_steps = 8 }))
  graph.assert_covers(trace, {
    "autochrono.issue -> autochrono.propose",
    "consensus.proposal -> consensus.decide",
    "consensus.consensus_reached -> autochrono.reply",
  })
  return trace
end

return {
  test_run_graph_drives_autochrono_issue_through_consensus_to_reply = function()
    local trace = run_smoke(42)

    graph.require_delivery(trace, { queue = "autochrono.issue", consumer = "autochrono.propose" })
    graph.require_delivery(trace, { queue = "consensus.proposal", consumer = "consensus.decide" })
    graph.require_delivery(trace, { queue = "consensus.consensus_reached", consumer = "autochrono.reply" })
    graph.require_raise(trace, "autochrono.reply", function(raised)
      return raised.payload.schema == "autochrono.reply.v1"
        and raised.payload.repo == "owner/repo"
        and raised.payload.issue_number == "42"
        and raised.payload.body:find("minimal approves.", 1, true) ~= nil
        and raised.payload.source_ref.ref == "owner/repo#issue/42"
    end)
  end,

  test_run_graph_autochrono_trace_is_deterministic = function()
    local first = run_smoke(43)
    local second = run_smoke(44)

    t.eq(graph.signature_without_payload_identity(first), graph.signature_without_payload_identity(second))
  end,
}
