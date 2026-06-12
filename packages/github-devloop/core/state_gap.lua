local S = {}

function S.install(M)

local max_dashboard_edges = 8
local max_worst_offenders = 3

local function state_marker_stage_rank(marker, state)
  local explicit_rank = tonumber(marker:match('stage_rank="(%d+)"'))
  return explicit_rank or M.stage_rank(state)
end

local function parse_marker_time(comment)
  local created_at = M._comment_created_at(comment)
  local seconds = M.iso_timestamp_epoch_seconds(created_at)
  if seconds == nil then
    return nil, nil
  end
  return created_at, seconds
end

local function append_state_markers(markers, comments, proposal_id)
  local marker_pattern = "<!%-%- fkst:github%-devloop:state:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments or {})) do
    local created_at, created_seconds = parse_marker_time(comment)
    if created_seconds ~= nil then
      for marker in M._comment_body(comment):gmatch(marker_pattern) do
        local marker_proposal = marker:match('proposal="([^"]+)"')
        local state = marker:match('state="([^"]+)"')
        local version = marker:match('version="([^"]*)"')
        if marker_proposal == proposal_id and M._label_by_state[state] ~= nil then
          table.insert(markers, {
            proposal_id = proposal_id,
            state = state,
            version = version,
            stage_rank = state_marker_stage_rank(marker, state),
            created_at = created_at,
            created_seconds = created_seconds,
          })
        end
      end
    end
  end
end

local function marker_sort_key(marker)
  return tostring(marker.created_at or "")
    .. "/"
    .. string.format("%04d", tonumber(marker.stage_rank) or 0)
    .. "/"
    .. tostring(marker.state or "")
    .. "/"
    .. tostring(marker.version or "")
end

function M.state_gap_marker_stream(entity)
  local markers = {}
  if entity == nil or entity.proposal_id == nil then
    return markers
  end
  append_state_markers(markers, entity.parent_issue and entity.parent_issue.comments or {}, entity.proposal_id)
  append_state_markers(markers, entity.pr and entity.pr.comments or {}, entity.proposal_id)
  table.sort(markers, function(a, b)
    return marker_sort_key(a) < marker_sort_key(b)
  end)
  return markers
end

local function budget_status(from_state, gap_seconds)
  local budget_minutes = M.liveness_budget_minutes(from_state)
  if budget_minutes == nil then
    return "no-budget", nil
  end
  local budget_seconds = math.floor(budget_minutes * 60)
  if gap_seconds > budget_seconds then
    return "over-budget", budget_seconds
  end
  if gap_seconds >= math.floor(budget_seconds * 0.8) then
    return "near-budget", budget_seconds
  end
  return "within-budget", budget_seconds
end

function M.state_gap_edges_for_entity(entity)
  local markers = M.state_gap_marker_stream(entity)
  local edges = {}
  local previous = nil
  for _, marker in ipairs(markers) do
    if previous ~= nil and previous.state ~= marker.state and marker.created_seconds >= previous.created_seconds then
      local gap_seconds = marker.created_seconds - previous.created_seconds
      local status, budget_seconds = budget_status(previous.state, gap_seconds)
      table.insert(edges, {
        proposal_id = entity.proposal_id,
        issue_number = entity.issue_number,
        from_state = previous.state,
        to_state = marker.state,
        edge = tostring(previous.state) .. "->" .. tostring(marker.state),
        gap_seconds = gap_seconds,
        from_created_at = previous.created_at,
        to_created_at = marker.created_at,
        budget_seconds = budget_seconds,
        budget_status = status,
      })
    end
    previous = marker
  end
  return edges
end

local function percentile(sorted_values, fraction)
  local count = #sorted_values
  if count == 0 then
    return nil
  end
  local rank = math.ceil(count * fraction)
  if rank < 1 then
    rank = 1
  elseif rank > count then
    rank = count
  end
  return sorted_values[rank]
end

local function sort_edge_summaries(summaries)
  table.sort(summaries, function(a, b)
    if a.p95_seconds ~= b.p95_seconds then
      return (a.p95_seconds or 0) > (b.p95_seconds or 0)
    end
    return tostring(a.edge or "") < tostring(b.edge or "")
  end)
end

function M.state_gap_report(entities)
  local by_edge = {}
  local all_edges = {}
  for _, entity in ipairs(entities or {}) do
    for _, edge in ipairs(M.state_gap_edges_for_entity(entity)) do
      table.insert(all_edges, edge)
      by_edge[edge.edge] = by_edge[edge.edge] or {
        edge = edge.edge,
        from_state = edge.from_state,
        to_state = edge.to_state,
        values = {},
        offenders = {},
        over_budget_count = 0,
        near_budget_count = 0,
        budget_seconds = edge.budget_seconds,
      }
      local bucket = by_edge[edge.edge]
      table.insert(bucket.values, edge.gap_seconds)
      table.insert(bucket.offenders, edge)
      if edge.budget_status == "over-budget" then
        bucket.over_budget_count = bucket.over_budget_count + 1
      elseif edge.budget_status == "near-budget" then
        bucket.near_budget_count = bucket.near_budget_count + 1
      end
    end
  end

  local summaries = {}
  for _, bucket in pairs(by_edge) do
    table.sort(bucket.values)
    table.sort(bucket.offenders, function(a, b)
      if a.gap_seconds ~= b.gap_seconds then
        return a.gap_seconds > b.gap_seconds
      end
      return tostring(a.proposal_id or "") < tostring(b.proposal_id or "")
    end)
    table.insert(summaries, {
      edge = bucket.edge,
      from_state = bucket.from_state,
      to_state = bucket.to_state,
      count = #bucket.values,
      p50_seconds = percentile(bucket.values, 0.50),
      p95_seconds = percentile(bucket.values, 0.95),
      max_seconds = bucket.values[#bucket.values],
      budget_seconds = bucket.budget_seconds,
      over_budget_count = bucket.over_budget_count,
      near_budget_count = bucket.near_budget_count,
      offenders = bucket.offenders,
    })
  end
  sort_edge_summaries(summaries)
  return {
    edges = all_edges,
    summaries = summaries,
  }
end

function M.state_gap_log_line(edge)
  return table.concat({
    "github-devloop",
    "dept=observability",
    "tag=GAP_EDGE",
    "proposal_id=" .. tostring(edge and edge.proposal_id or "unknown"),
    "gap_edge=" .. tostring(edge and edge.edge or "unknown"),
    "gap_seconds=" .. tostring(edge and edge.gap_seconds or 0),
    "budget_seconds=" .. tostring(edge and edge.budget_seconds or ""),
    "budget_status=" .. tostring(edge and edge.budget_status or "unknown"),
    "from_created_at=" .. tostring(edge and edge.from_created_at or ""),
    "to_created_at=" .. tostring(edge and edge.to_created_at or ""),
  }, " ")
end

local function format_duration(seconds)
  local value = tonumber(seconds)
  if value == nil then
    return "n/a"
  end
  if value < 60 then
    return tostring(math.floor(value)) .. "s"
  end
  local minutes = math.floor(value / 60)
  local rest = math.floor(value % 60)
  if minutes < 60 then
    return tostring(minutes) .. "m " .. tostring(rest) .. "s"
  end
  local hours = math.floor(minutes / 60)
  local minute_rest = minutes % 60
  return tostring(hours) .. "h " .. tostring(minute_rest) .. "m"
end

local function offender_ref(edge)
  if tonumber(edge.issue_number) ~= nil then
    return "#" .. tostring(edge.issue_number)
  end
  return tostring(edge.proposal_id or "unknown")
end

local function offender_summary(offenders)
  local parts = {}
  for index, edge in ipairs(offenders or {}) do
    if index > max_worst_offenders then
      break
    end
    table.insert(parts, offender_ref(edge) .. " " .. format_duration(edge.gap_seconds))
  end
  if #parts == 0 then
    return "none"
  end
  return table.concat(parts, ", ")
end

function M.append_state_gap_dashboard_section(lines, report)
  table.insert(lines, "")
  table.insert(lines, "## State-gap latency")
  local summaries = report and report.summaries or {}
  if #summaries == 0 then
    table.insert(lines, "- No completed state gaps in the trusted marker window.")
    return
  end
  local shown = 0
  for _, summary in ipairs(summaries) do
    if shown >= max_dashboard_edges then
      table.insert(lines, "- ... " .. tostring(#summaries - shown) .. " more")
      break
    end
    local budget = summary.budget_seconds ~= nil and format_duration(summary.budget_seconds) or "n/a"
    table.insert(lines, "- " .. tostring(summary.edge)
      .. ": count " .. tostring(summary.count)
      .. ", P50 " .. format_duration(summary.p50_seconds)
      .. ", P95 " .. format_duration(summary.p95_seconds)
      .. ", max " .. format_duration(summary.max_seconds)
      .. ", budget " .. budget
      .. ", near " .. tostring(summary.near_budget_count or 0)
      .. ", over " .. tostring(summary.over_budget_count or 0)
      .. "; worst " .. offender_summary(summary.offenders))
    shown = shown + 1
  end
end

end

return S
