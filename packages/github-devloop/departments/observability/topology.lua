local M = {}

local lanes = {
  { id = "github_proxy_lane", label = "github-proxy" },
  { id = "consensus_lane", label = "consensus" },
  { id = "github_devloop_lane", label = "github-devloop" },
}

local macro_nodes = {
  { id = "github", label = "GitHub", lane = "github_proxy_lane" },
  { id = "poll", label = "poll", lane = "github_proxy_lane" },
  { id = "pr", label = "PR", lane = "github_proxy_lane" },
  { id = "consensus", label = "consensus(angles+meta)", lane = "consensus_lane" },
  { id = "observe", label = "observe", lane = "github_devloop_lane" },
  { id = "intake", label = "intake", lane = "github_devloop_lane" },
  { id = "ready", label = "ready(dep-gate)", lane = "github_devloop_lane" },
  { id = "implement", label = "implement", lane = "github_devloop_lane" },
  { id = "review", label = "review", lane = "github_devloop_lane" },
  { id = "merge_ready", label = "merge-ready", lane = "github_devloop_lane" },
  { id = "merge", label = "merge", lane = "github_devloop_lane" },
  { id = "rollup_dev", label = "rollup->dev", lane = "github_devloop_lane" },
}

local department_macro = {
  ["github-proxy.github_poll"] = "poll",
  ["github-proxy.github_pr_open"] = "pr",

  ["consensus.decide"] = "consensus",

  ["github-devloop.observe_issue"] = "observe",
  ["github-devloop.observe_pr"] = "observe",
  ["github-devloop.intake_scan"] = "intake",
  ["github-devloop.intake_probe"] = "intake",
  ["github-devloop.intake_judge"] = "intake",
  ["github-devloop.consensus_result"] = "ready",
  ["github-devloop.implement"] = "implement",
  ["github-devloop.open_pr"] = "implement",
  ["github-devloop.review_pr"] = "review",
  ["github-devloop.review_loop"] = "review",
  ["github-devloop.review_result"] = "merge_ready",
  ["github-devloop.merge"] = "merge",
  ["github-devloop.rollup_scan"] = "rollup_dev",
  ["github-devloop.rollup_merge"] = "rollup_dev",
}

local raiser_macro = {
  ["github-proxy.github_poll"] = "github",
}

local ignored_departments = {
  ["autochrono.propose"] = true,
  ["autochrono.reply"] = true,
  ["consensus.dead_letter"] = true,
  ["github-autochrono.inbound_glue"] = true,
  ["github-autochrono.outbound_glue"] = true,
  ["github-devloop.comment_handoff"] = true,
  ["github-devloop.dead_letter"] = true,
  ["github-devloop.decompose"] = true,
  ["github-devloop.doctor"] = true,
  ["github-devloop.ensure_repo"] = true,
  ["github-devloop.fix"] = true,
  ["github-devloop.liveness_scan"] = true,
  ["github-devloop.loop"] = true,
  ["github-devloop.observability"] = true,
  ["github-devloop.pr_freshness_scan"] = true,
  ["github-devloop.reconcile"] = true,
  ["github-devloop.review_meta"] = true,
  ["github-devloop.substrate_ref_scan"] = true,
  ["github-devloop.sync_conflict"] = true,
  ["github-devloop.sync_scan"] = true,
  ["github-proxy.github_comment"] = true,
  ["github-proxy.github_issue_blocked_by"] = true,
  ["github-proxy.github_issue_create"] = true,
  ["github-proxy.github_issue_label"] = true,
  ["github-proxy.github_pr_comment"] = true,
}

local function macro_index()
  local index = {}
  for order, node in ipairs(macro_nodes) do
    index[node.id] = {
      order = order,
      lane = node.lane,
      label = node.label,
    }
  end
  return index
end

local macros = macro_index()

local function canonical_from_node(node, expected_kind)
  local id = tostring(node and node.id or "")
  local prefix = tostring(expected_kind) .. ":"
  if id:sub(1, #prefix) == prefix then
    return id:sub(#prefix + 1)
  end
  local package = tostring(node and node.package or "")
  local name = tostring(node and node.name or "")
  if package ~= "" and name ~= "" then
    return package .. "." .. name
  end
  return nil
end

local function require_graph(graph)
  if type(graph) ~= "table" then
    error("github-devloop: topology: graph must be a table")
  end
  if graph.schema ~= "fkst.graph.v1" then
    error("github-devloop: topology: graph schema must be fkst.graph.v1")
  end
  if type(graph.nodes) ~= "table" or type(graph.edges) ~= "table" then
    error("github-devloop: topology: graph requires nodes and edges")
  end
end

local function validate_macro_references()
  for canonical, macro_id in pairs(department_macro) do
    if ignored_departments[canonical] then
      error("github-devloop: topology: department mapped and ignored: " .. canonical)
    end
    if macros[macro_id] == nil then
      error("github-devloop: topology: map references unknown macro: " .. tostring(macro_id))
    end
  end
  for canonical, macro_id in pairs(raiser_macro) do
    if macros[macro_id] == nil then
      error("github-devloop: topology: raiser map references unknown macro: " .. tostring(macro_id) .. " for " .. tostring(canonical))
    end
  end
end

function M.validate_graph(graph)
  require_graph(graph)
  validate_macro_references()

  for _, node in ipairs(graph.nodes) do
    if node.kind == "department" then
      local canonical = canonical_from_node(node, "department")
      if canonical == nil then
        error("github-devloop: topology: malformed department node in topology graph")
      end
      local mapped = department_macro[canonical] ~= nil
      local ignored = ignored_departments[canonical] == true
      if not mapped and not ignored then
        error("github-devloop: topology: unmapped department in topology graph: " .. canonical)
      end
    end
  end

  return true
end

local function mapped_macro_for_node(node)
  if node.kind == "department" then
    return department_macro[canonical_from_node(node, "department")]
  end
  if node.kind == "raiser" then
    return raiser_macro[canonical_from_node(node, "raiser")]
  end
  return nil
end

local function sorted_values(set)
  local values = {}
  for value in pairs(set or {}) do
    table.insert(values, value)
  end
  table.sort(values)
  return values
end

local function collect_edges(graph)
  local node_by_id = {}
  local macro_by_node = {}
  for _, node in ipairs(graph.nodes) do
    node_by_id[tostring(node.id or "")] = node
    local macro_id = mapped_macro_for_node(node)
    if macro_id ~= nil then
      macro_by_node[tostring(node.id or "")] = macro_id
    end
  end

  local queue_producers = {}
  local queue_consumers = {}
  for _, edge in ipairs(graph.edges) do
    local from = tostring(edge.from or "")
    local to = tostring(edge.to or "")
    local relation = tostring(edge.relation or "")
    if relation == "produces" or relation == "raises" then
      local producer = macro_by_node[from]
      local target = node_by_id[to]
      if producer ~= nil and target ~= nil and target.kind == "queue" then
        queue_producers[to] = queue_producers[to] or {}
        queue_producers[to][producer] = true
      end
    elseif relation == "consumes" then
      local source = node_by_id[from]
      local consumer = macro_by_node[to]
      if consumer ~= nil and source ~= nil and source.kind == "queue" then
        queue_consumers[from] = queue_consumers[from] or {}
        queue_consumers[from][consumer] = true
      end
    end
  end

  local edge_set = {}
  for queue_id, producers in pairs(queue_producers) do
    local consumers = queue_consumers[queue_id] or {}
    for _, producer in ipairs(sorted_values(producers)) do
      for _, consumer in ipairs(sorted_values(consumers)) do
        if producer ~= consumer then
          edge_set[producer .. "\t" .. consumer] = {
            from = producer,
            to = consumer,
          }
        end
      end
    end
  end

  local edges = {}
  for _, edge in pairs(edge_set) do
    table.insert(edges, edge)
  end
  table.sort(edges, function(left, right)
    local left_from = macros[left.from] and macros[left.from].order or 999
    local right_from = macros[right.from] and macros[right.from].order or 999
    if left_from ~= right_from then
      return left_from < right_from
    end
    local left_to = macros[left.to] and macros[left.to].order or 999
    local right_to = macros[right.to] and macros[right.to].order or 999
    if left_to ~= right_to then
      return left_to < right_to
    end
    return left.from .. "/" .. left.to < right.from .. "/" .. right.to
  end)
  return edges
end

local function append_lane(lines, lane)
  table.insert(lines, "  subgraph " .. lane.id .. "[\"" .. lane.label .. "\"]")
  for _, node in ipairs(macro_nodes) do
    if node.lane == lane.id then
      table.insert(lines, "    " .. node.id .. "[\"" .. node.label .. "\"]")
    end
  end
  table.insert(lines, "  end")
end

function M.render_mermaid(graph)
  M.validate_graph(graph)
  local lines = { "flowchart LR" }
  for _, lane in ipairs(lanes) do
    append_lane(lines, lane)
  end
  for _, edge in ipairs(collect_edges(graph)) do
    table.insert(lines, "  " .. edge.from .. " --> " .. edge.to)
  end
  return table.concat(lines, "\n")
end

M._department_macro = department_macro
M._ignored_departments = ignored_departments

return M
