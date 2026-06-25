local graph = require("testkit.graph")
local t = fkst.test

local repo = "owner/repo"
local issue_number = 1492
local proposal_id = "autochrono/issue/owner/repo/" .. tostring(issue_number)

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#issue/" .. tostring(issue_number),
  }
end

local function initial_event()
  return {
    queue = "consensus.consensus_reached",
    payload = {
      schema = "consensus.consensus_reached.v1",
      proposal_id = proposal_id,
      decision = "approve",
      body = "All angles approve.",
      dedup_key = "consensus:autochrono/issue/owner/repo/" .. tostring(issue_number) .. "/2026-06-03T01-02-03Z",
      source_ref = source_ref(),
    },
    source_ref = {
      kind = "external",
      reference = "owner/repo#issue/" .. tostring(issue_number),
    },
  }
end

local function mock_dry_run_comment_write()
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_run_graph_autochrono_reply_handoffs_to_github_autochrono_outbound_glue = function()
    mock_dry_run_comment_write()

    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 4 }))
    graph.assert_covers(trace, {
      "consensus.consensus_reached -> autochrono.reply",
      "autochrono.reply -> github-autochrono.outbound_glue",
    })

    local reply, _, reply_index = graph.require_raise(trace, "autochrono.reply", function(raised)
      local payload = raised.payload or {}
      return payload.schema == "autochrono.reply.v1"
        and payload.repo == repo
        and payload.issue_number == tostring(issue_number)
        and payload.body == "All angles approve."
        and payload.dedup_key == "autochrono:owner/repo#issue/" .. tostring(issue_number)
        and payload.source_ref.ref == "owner/repo#issue/" .. tostring(issue_number)
    end)

    local outbound_step, outbound_index = graph.require_delivery(trace, {
      queue = "autochrono.reply",
      consumer = "github-autochrono.outbound_glue",
    })
    t.eq(outbound_step.exit_code, 0)
    t.is_true(outbound_index > reply_index)

    local comment_request, _, comment_request_index = graph.require_raise(
      trace,
      "github-proxy.github_issue_comment_request",
      function(raised)
        local payload = raised.payload or {}
        return payload.schema == "github-proxy.v1"
          and payload.repo == reply.payload.repo
          and payload.issue_number == reply.payload.issue_number
          and payload.body == reply.payload.body
          and payload.dedup_key == reply.payload.dedup_key
          and payload.source_ref.ref == reply.payload.source_ref.ref
      end
    )
    t.eq(comment_request_index, outbound_index)
    t.eq(comment_request.payload.source_ref.kind, "external")
  end,
}
