local common = require("departments.observability.common")
local avm_scoreboard = require("departments.observability.avm_scoreboard")
local census = require("departments.observability.census")
local dashboard = require("departments.observability.dashboard")
local reaper = require("departments.observability.reaper")
local topology = require("departments.observability.topology")

local S = {}

function S.install(M)
common.install_common(M)
avm_scoreboard.install_avm_scoreboard(M)
census.install_census(M)
reaper.install_reaper(M)
dashboard.install_dashboard(M)

function M.observability_topology_mermaid()
  if type(graph_json) ~= "function" then
    return nil
  end
  local ok, result = pcall(function()
    local decoded = json.decode(graph_json())
    return topology.render_mermaid(decoded)
  end)
  if not ok then
    local reason = M._one_line and M._one_line(result) or tostring(result or "")
    log.warn("github-devloop dept=observability tag=TOPOLOGY_UNAVAILABLE reason=" .. tostring(reason))
    return nil
  end
  return result
end

function M.observe_devloop_entities(event)
  common.require_observe_bot(M)
  local repo = common.require_observe_repo(M)
  local limits = M.observability_limits()
  local deadline = M.observability_deadline(now(), limits)
  local observed = M.collect_observability_entities(event, repo, limits, deadline)

  M.reap_orphan_prs(repo, observed.list)
  local queue_starvation = M.observe_queue_starvation(repo, observed.list, limits, deadline, observed.now_seconds)
  local conflict_hotspot = M.observe_conflict_hotspots(repo, M.observability_call_timeout(limits, deadline))
  local rendered_dashboard = M.render_observability_dashboard({
    entities = observed.list,
    counts = observed.counts,
    stalls = observed.stalls,
    state_gap_report = observed.state_gap_report,
    now_seconds = observed.now_seconds,
    topology_mermaid = M.observability_topology_mermaid(),
  })
  M.publish_observability_dashboard(repo, rendered_dashboard, limits, deadline)

  return {
    entity_count = #observed.list,
    counts = observed.counts,
    queue_starvation = queue_starvation,
    conflict_hotspot = conflict_hotspot,
    state_gap_report = observed.state_gap_report,
    dashboard_hash = rendered_dashboard.hash,
  }
end
end

return S
