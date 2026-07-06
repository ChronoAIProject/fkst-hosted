local graph = require("testkit.graph")

local function initial_event()
  return {
    queue = "dead_letter",
    payload = {
      delivery_id = "delivery/v3/raised/queue/consensus.consensus_reached/dept/github-devloop-pr.review_result/01HY",
      queue = "consensus.consensus_reached",
      dept = "github-devloop-pr.review_result",
      error_class = "review-result-failed",
      dedup_key = "consensus:github-devloop/pr/owner/repo/7/review",
      attempt = 12,
      error = "review result failed while applying marker",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    },
    source_ref = {
      kind = "external",
      reference = "dead/delivery/v3/raised/queue/consensus.consensus_reached/dept/github-devloop-pr.review_result/01HY",
    },
  }
end

return {
  test_forced_dead_letter_publish_routes_to_pr_dead_letter_consumer = function()
    local trace = graph.require_quiescent(graph.run(initial_event(), { max_steps = 2 }))
    local dead_step = graph.require_delivery(trace, {
      queue = "github-devloop-pr.dead_letter",
      consumer = "github-devloop-pr.dead_letter",
    })
    graph.assert_covers(trace, {
      "github-devloop-pr.dead_letter -> github-devloop-pr.dead_letter",
    })
    return dead_step
  end,
}
