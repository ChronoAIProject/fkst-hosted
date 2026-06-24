local M = {}

M.label_by_state = {
  thinking = "fkst-dev:thinking",
  dependency_wait = "fkst-dev:ready",
  ready = "fkst-dev:ready",
  implementing = "fkst-dev:implementing",
  ["awaiting-pr"] = "fkst-dev:awaiting-pr",
  ["pr-open"] = "fkst-dev:pr-open",
  reviewing = "fkst-dev:reviewing",
  ["merge-ready"] = "fkst-dev:merge-ready",
  merging = "fkst-dev:merging",
  merged = "fkst-dev:merged",
  ["closed-unmerged"] = "fkst-dev:blocked",
  fixing = "fkst-dev:fixing",
  ["review-meta"] = "fkst-dev:review-meta",
  ["impl-failed"] = "fkst-dev:impl-failed",
  blocked = "fkst-dev:blocked",
}

M.state_labels = {
  ["fkst-dev:thinking"] = true,
  ["fkst-dev:ready"] = true,
  ["fkst-dev:implementing"] = true,
  ["fkst-dev:awaiting-pr"] = true,
  ["fkst-dev:pr-open"] = true,
  ["fkst-dev:reviewing"] = true,
  ["fkst-dev:merge-ready"] = true,
  ["fkst-dev:merging"] = true,
  ["fkst-dev:merged"] = true,
  ["fkst-dev:fixing"] = true,
  ["fkst-dev:review-meta"] = true,
  ["fkst-dev:impl-failed"] = true,
  ["fkst-dev:blocked"] = true,
}

M.state_graph = {
  unmanaged = { "thinking" },
  thinking = { "dependency_wait", "ready", "blocked" },
  dependency_wait = { "dependency_wait", "ready", "blocked" },
  ready = { "dependency_wait", "implementing", "blocked" },
  implementing = { "awaiting-pr", "impl-failed" },
  ["awaiting-pr"] = { "merged", "ready", "blocked" },
  ["pr-open"] = { "reviewing", "blocked" },
  reviewing = { "merge-ready", "fixing", "review-meta" },
  ["merge-ready"] = { "merging", "blocked" },
  merging = { "merged", "reviewing", "fixing", "blocked" },
  merged = {},
  ["closed-unmerged"] = {},
  fixing = { "reviewing", "review-meta" },
  ["review-meta"] = { "fixing", "blocked" },
  ["impl-failed"] = { "implementing" },
  blocked = {},
}

M.issue_state_order = { "thinking", "dependency_wait", "ready", "implementing", "pr-open", "reviewing", "merge-ready", "fixing", "impl-failed", "blocked", "review-meta", "merging", "merged", "awaiting-pr" }
M.state_order = { "thinking", "dependency_wait", "ready", "implementing", "pr-open", "reviewing", "merge-ready", "fixing", "impl-failed", "blocked", "review-meta", "merging", "merged", "closed-unmerged", "awaiting-pr" }

M.state_stage_rank = {
  thinking = 100,
  dependency_wait = 500,
  ready = 500,
  implementing = 600,
  ["awaiting-pr"] = 625,
  ["pr-open"] = 650,
  reviewing = 675,
  ["merge-ready"] = 690,
  merging = 695,
  fixing = 700,
  ["review-meta"] = 710,
  ["impl-failed"] = 750,
  blocked = 800,
  ["closed-unmerged"] = 825,
  merged = 900,
}

local function copy_array(values)
  local out = {}
  for _, value in ipairs(values or {}) do
    table.insert(out, value)
  end
  return out
end

function M.is_state(state)
  return M.label_by_state[state] ~= nil
end

function M.is_state_label(label)
  return M.state_labels[tostring(label)] == true
end

function M.state_label(state)
  return M.label_by_state[state]
end

function M.stage_rank(state)
  return M.state_stage_rank[state] or 0
end

function M.state_order_copy()
  return copy_array(M.state_order)
end

function M.issue_state_order_copy()
  return copy_array(M.issue_state_order)
end

function M.state_successors(state)
  return copy_array(M.state_graph[state])
end

function M.lifecycle_state_set()
  local out = {}
  for state, _ in pairs(M.label_by_state) do
    out[state] = true
  end
  for state, next_states in pairs(M.state_graph) do
    if state ~= "unmanaged" then
      out[state] = true
    end
    for _, next_state in ipairs(next_states or {}) do
      if next_state ~= "unmanaged" then
        out[next_state] = true
      end
    end
  end
  for _, state in ipairs(M.state_order) do
    out[state] = true
  end
  for state, _ in pairs(M.state_stage_rank) do
    out[state] = true
  end
  return out
end

return M
