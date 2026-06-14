local common = require("departments.observability.common")
local census = require("departments.observability.census")
local dashboard = require("departments.observability.dashboard")
local reaper = require("departments.observability.reaper")

local S = {}

function S.install(M)
common.install_common(M)
census.install_census(M)
reaper.install_reaper(M)
dashboard.install_dashboard(M)

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
