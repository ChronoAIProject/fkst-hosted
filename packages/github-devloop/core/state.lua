local S = {}

function S.install(M)
function M.has_label(labels, expected)
  if type(labels) ~= "table" then
    return false
  end
  for _, label in ipairs(labels) do
    if tostring(label) == expected then
      return true
    end
  end
  return false
end

function M.state_label(state)
  return M._label_by_state[state]
end

function M.state_marker(proposal_id, state, version)
  if state ~= "thinking"
    and state ~= "ready"
    and state ~= "implementing"
    and state ~= "pr-open"
    and state ~= "reviewing"
    and state ~= "review-meta"
    and state ~= "merge-ready"
    and state ~= "merging"
    and state ~= "merged"
    and state ~= "fixing"
    and state ~= "impl-failed"
    and state ~= "blocked"
    and state ~= "stuck" then
    error("github-devloop: invalid state")
  end
  return '<!-- fkst:github-devloop:state:v1 proposal="' .. tostring(proposal_id)
    .. '" state="' .. tostring(state)
    .. '" version="' .. tostring(version)
    .. '" stage_rank="' .. tostring(M._state_stage_rank[state])
    .. '" -->'
end

function M.version_order_key(version)
  local text = tostring(version or "")
  local rest = text
  if rest:sub(1, #"consensus:") == "consensus:" then
    rest = rest:sub(#"consensus:" + 1)
  elseif rest:sub(1, #"ready/") == "ready/" then
    rest = rest:sub(#"ready/" + 1):gsub("^consensus%-", "")
  end

  local timestamp = nil
  for found in rest:gmatch("(%d%d%d%d%-%d%d%-%d%dT%d%d[%-:]%d%d[%-:]%d%dZ)") do
    timestamp = found
  end
  if timestamp ~= nil then
    local _, end_pos = rest:find(timestamp, 1, true)
    local suffix = end_pos and rest:sub(end_pos + 1) or ""
    local loop_n = tonumber(suffix:match("/loop/(%d+)$")) or 0
    local suffix_tie = suffix:gsub("/loop/%d+$", "")
    return timestamp:gsub(":", "-") .. "/loop/" .. string.format("%012d", loop_n) .. suffix_tie
  end
  return rest
end

function M.stage_rank(state)
  return M._state_stage_rank[state] or 0
end

function M.version_updated_at(version)
  local text = tostring(version or "")
  local updated_at = ""
  for found in text:gmatch("(%d%d%d%d%-%d%d%-%d%dT%d%d[%-:]%d%d[%-:]%d%dZ)") do
    updated_at = found:gsub(":", "-")
  end
  return updated_at
end

function M.version_loop_round(version)
  local n = tostring(version or ""):match("[/-]loop[/-](%d+)$")
  return tonumber(n) or 0
end

function M.version_fix_round(version)
  local max_n = 0
  for n in tostring(version or ""):gmatch("[/-]fix[/-](%d+)") do
    local parsed = tonumber(n) or 0
    if parsed > max_n then
      max_n = parsed
    end
  end
  return max_n
end

function M.version_review_meta_action_round(version)
  local max_n = 0
  for n in tostring(version or ""):gmatch("[/-]review%-meta%-action[/-](%d+)") do
    local parsed = tonumber(n) or 0
    if parsed > max_n then
      max_n = parsed
    end
  end
  return max_n
end

function M.version_review_loop_round(version)
  local max_n = 0
  for n in tostring(version or ""):gmatch("[/-]review%-loop[/-](%d+)") do
    local parsed = tonumber(n) or 0
    if parsed > max_n then
      max_n = parsed
    end
  end
  return max_n
end

function M.next_fix_version(version)
  local base = tostring(version or "")
  local next_n = M.version_fix_round(base) + 1
  return base .. "/fix/" .. tostring(next_n)
end

function M.fix_version_from_review_version(version)
  return M.next_fix_version(version)
end

function M.next_review_meta_action_version(version)
  local base = tostring(version or "")
  local next_n = M.version_review_meta_action_round(base) + 1
  return base .. "/review-meta-action/" .. tostring(next_n)
end

local function version_primary_key(version)
  local updated_at = M.version_updated_at(version)
  if updated_at ~= "" then
    return updated_at
  end
  return M.version_order_key(version)
end

local function version_sort_key(version, stage_rank)
  return {
    primary = version_primary_key(version),
    loop_n = M.version_loop_round(version),
    fix_n = M.version_fix_round(version),
    review_loop_n = M.version_review_loop_round(version),
    review_meta_action_n = M.version_review_meta_action_round(version),
    stage_rank = tonumber(stage_rank) or 0,
  }
end

local function marker_stage_rank(marker, state)
  local explicit_rank = tonumber(marker:match('stage_rank="(%d+)"'))
  return explicit_rank or M.stage_rank(state)
end

local function compare_version_keys(left, right)
  if left.primary ~= right.primary then
    return left.primary > right.primary and 1 or -1
  end
  if left.loop_n ~= right.loop_n then
    return left.loop_n > right.loop_n and 1 or -1
  end
  if left.fix_n ~= right.fix_n then
    return left.fix_n > right.fix_n and 1 or -1
  end
  if left.review_meta_action_n ~= right.review_meta_action_n then
    return left.review_meta_action_n > right.review_meta_action_n and 1 or -1
  end
  if left.review_loop_n ~= right.review_loop_n then
    return left.review_loop_n > right.review_loop_n and 1 or -1
  end
  return 0
end

local function versions_equivalent(left, right)
  if left == nil or right == nil then
    return left == right
  end
  if tostring(left) == tostring(right) then
    return true
  end
  return M.safe_version_segment(left) == M.safe_version_segment(right)
end

local function strip_transition_version_suffixes(version)
  local text = tostring(version or "")
  local previous = nil
  while previous ~= text do
    previous = text
    text = text
      :gsub("/review%-meta%-action/%d+$", "")
      :gsub("%-review%-meta%-action%-%d+$", "")
      :gsub("/review%-loop/%d+$", "")
      :gsub("%-review%-loop%-%d+$", "")
      :gsub("/fix/%d+$", "")
      :gsub("%-fix%-%d+$", "")
  end
  return text
end

local function strip_latest_fix_version_suffix(version)
  return tostring(version or "")
    :gsub("/fix/%d+$", "")
    :gsub("%-fix%-%d+$", "")
end

local function compare_same_base_transition_versions(incoming_version, current_version)
  local incoming_key = version_sort_key(incoming_version, 0)
  local current_key = version_sort_key(current_version, 0)
  if incoming_key.loop_n ~= current_key.loop_n then
    return incoming_key.loop_n > current_key.loop_n and 1 or -1
  end
  if incoming_key.fix_n ~= current_key.fix_n then
    return incoming_key.fix_n > current_key.fix_n and 1 or -1
  end
  if incoming_key.review_meta_action_n ~= current_key.review_meta_action_n then
    return incoming_key.review_meta_action_n > current_key.review_meta_action_n and 1 or -1
  end
  if incoming_key.review_loop_n ~= current_key.review_loop_n then
    return incoming_key.review_loop_n > current_key.review_loop_n and 1 or -1
  end
  return 0
end

local function compare_transition_versions(incoming_version, current_version)
  if incoming_version == current_version then
    return 0
  end
  if incoming_version == nil then
    return current_version == nil and 0 or -1
  end
  if current_version == nil then
    return 1
  end
  local incoming_base = strip_transition_version_suffixes(incoming_version)
  local current_base = strip_transition_version_suffixes(current_version)
  if versions_equivalent(incoming_base, current_base) then
    return compare_same_base_transition_versions(incoming_version, current_version)
  end
  return compare_version_keys(version_sort_key(incoming_version, 0), version_sort_key(current_version, 0))
end

local function sign_order(value)
  if value > 0 then
    return 1
  end
  if value < 0 then
    return -1
  end
  return 0
end

function M.compare_state_marker_order(current, target_state, target_version)
  if current == nil or current.version == nil then
    return -1
  end
  local version_order = compare_transition_versions(current.version, target_version)
  if version_order ~= 0 then
    return sign_order(version_order)
  end
  return sign_order(M.stage_rank(current.state) - M.stage_rank(target_state))
end

local function compare_state_marker(a, b)
  if a == nil then
    return true
  end
  local a_key = version_sort_key(a.version, a.stage_rank)
  local b_key = version_sort_key(b.version, b.stage_rank)
  local version_order = compare_version_keys(b_key, a_key)
  if version_order ~= 0 then
    return version_order > 0
  end
  if a.version == b.version
    and ((a.state == "ready" and b.state == "blocked") or (a.state == "blocked" and b.state == "ready")) then
    return b.state == "blocked"
  end
  if b_key.stage_rank ~= a_key.stage_rank then
    return b_key.stage_rank > a_key.stage_rank
  end
  return false
end

function M.comment_bodies(comments)
  local bodies = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(bodies, M._comment_body(comment))
  end
  return bodies
end

function M.current_state(comments, proposal_id)
  if type(comments) ~= "table" then
    return nil
  end

  local current = nil
  local marker_pattern = "<!%-%- fkst:github%-devloop:state:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_state = marker:match('state="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      if marker_proposal == proposal_id and M._label_by_state[marker_state] ~= nil then
        local candidate = {
          state = marker_state,
          version = marker_version,
          stage_rank = marker_stage_rank(marker, marker_state),
        }
        if compare_state_marker(current, candidate) then
          current = candidate
        end
      end
    end
  end
  return current or {
    state = nil,
    version = nil,
    stage_rank = 0,
  }
end

function M.has_state_marker(comments, proposal_id, state, version)
  if type(comments) ~= "table" then
    return false
  end
  local marker = M.state_marker(proposal_id, state, version)
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    if M._comment_body(comment):find(marker, 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function normalize_state(state)
  if state == nil then
    return "unmanaged"
  end
  return state
end

local function can_reach(from_state, to_state, seen)
  local from = normalize_state(from_state)
  if from == to_state then
    return true
  end
  if M._state_graph[from] == nil then
    return false
  end
  local visited = seen or {}
  if visited[from] then
    return false
  end
  visited[from] = true
  for _, next_state in ipairs(M._state_graph[from]) do
    if can_reach(next_state, to_state, visited) then
      return true
    end
  end
  return false
end

function M.transition_status(current, from_states, to_state)
  local current_state = current
  if type(current) == "table" then
    current_state = current.state
  end
  if current_state == to_state then
    return "idempotent"
  end
  local normalized_current = normalize_state(current_state)
  for _, from_state in ipairs(from_states or {}) do
    if normalized_current == normalize_state(from_state) then
      return "apply"
    end
  end
  for _, from_state in ipairs(from_states or {}) do
    if can_reach(normalized_current, normalize_state(from_state)) then
      return "pending"
    end
  end
  return "stale"
end

function M.versioned_transition_status(current, from_states, to_state, incoming_version)
  if type(current) == "table"
    and current.version ~= nil
    and incoming_version ~= nil
    and compare_transition_versions(incoming_version, current.version) < 0 then
    return "stale"
  end
  local status = M.transition_status(current, from_states, to_state)
  return status
end

function M.cyclic_transition_status(current, from_states, to_state, incoming_version, target_version)
  local current_state = current
  local current_version = nil
  if type(current) == "table" then
    current_state = current.state
    current_version = current.version
  end
  if incoming_version == nil then
    return M.transition_status(current, from_states, to_state)
  end
  if target_version ~= nil and current_state == to_state and versions_equivalent(current_version, target_version) then
    return "idempotent"
  end

  local version_order = compare_transition_versions(incoming_version, current_version)
  if version_order > 0 then
    return "pending"
  end
  if version_order < 0 then
    return "stale"
  end

  if current_state == to_state then
    return "idempotent"
  end
  local normalized_current = normalize_state(current_state)
  for _, from_state in ipairs(from_states or {}) do
    if normalized_current == normalize_state(from_state) then
      return "apply"
    end
  end
  if M.stage_rank(to_state) > M.stage_rank(current_state) then
    return "apply"
  end
  return "stale"
end

function M.cas_outcome(current, transition, incoming_version)
  if transition == "apply" then
    return "applied"
  end
  if transition == "idempotent" then
    return "skip-idempotent(already at to_state)"
  end
  if transition == "pending" then
    return "retry-pending(from-state marker not yet visible)"
  end
  if transition == "stale" then
    if type(current) == "table"
      and current.version ~= nil
      and incoming_version ~= nil
      and compare_transition_versions(incoming_version, current.version) < 0 then
      return "skip-stale(incoming version < current marker version)"
    end
    return "skip-advanced-or-diverged"
  end
  return tostring(transition or "unknown")
end

function M.state_label_changes(to_state)
  local add_label = M.state_label(to_state)
  if add_label == nil then
    error("github-devloop: invalid state")
  end

  local remove_labels = {}
  for _, state in ipairs(M._state_order) do
    local label = M._label_by_state[state]
    if state ~= to_state then
      table.insert(remove_labels, label)
    end
  end
  return { add_label }, remove_labels
end

function M.state_label_hint_matches(labels, state)
  local expected_label = M.state_label(state)
  if expected_label == nil then
    return false
  end

  local has_expected = false
  for _, label in ipairs(labels or {}) do
    local label_text = tostring(label)
    if label_text == expected_label then
      has_expected = true
    elseif M._state_labels[label_text] then
      return false
    end
  end
  return has_expected
end

function M.build_reconcile_state_label_request(repo, issue_number, proposal_id, state, version, source_ref)
  return M.build_state_label_request(
    repo,
    issue_number,
    state,
    M._dedup_key({
      "reconcile",
      "label",
      tostring(proposal_id),
      tostring(state),
      tostring(version or "unversioned"),
    }),
    source_ref
  )
end

function M.has_terminal_label(labels)
  return M.has_label(labels, M._ready_label)
    or M.has_label(labels, M._implementing_label)
    or M.has_label(labels, M._pr_open_label)
    or M.has_label(labels, M._reviewing_label)
    or M.has_label(labels, M._review_meta_label)
    or M.has_label(labels, M._merge_ready_label)
    or M.has_label(labels, M._merging_label)
    or M.has_label(labels, M._merged_label)
    or M.has_label(labels, M._fixing_label)
    or M.has_label(labels, M._impl_failed_label)
    or M.has_label(labels, M._blocked_label)
    or M.has_label(labels, M._stuck_label)
end

function M.has_thinking_label(labels)
  return M.has_label(labels, M._thinking_label)
end

function M.has_stuck_label(labels)
  return M.has_label(labels, M._stuck_label)
end

function M.has_blocked_label(labels)
  return M.has_label(labels, M._blocked_label)
end

function M.has_ready_label(labels)
  return M.has_label(labels, M._ready_label)
end

function M.has_implementing_label(labels)
  return M.has_label(labels, M._implementing_label)
end

function M.has_pr_open_label(labels)
  return M.has_label(labels, M._pr_open_label)
end

function M.has_reviewing_label(labels)
  return M.has_label(labels, M._reviewing_label)
end

function M.has_merge_ready_label(labels)
  return M.has_label(labels, M._merge_ready_label)
end

function M.has_merging_label(labels)
  return M.has_label(labels, M._merging_label)
end

function M.has_merged_label(labels)
  return M.has_label(labels, M._merged_label)
end

function M.has_fixing_label(labels)
  return M.has_label(labels, M._fixing_label)
end

function M.has_review_meta_label(labels)
  return M.has_label(labels, M._review_meta_label)
end

function M.has_impl_failed_label(labels)
  return M.has_label(labels, M._impl_failed_label)
end

function M.has_decision_terminal_label(labels)
  return M.has_label(labels, M._ready_label)
    or M.has_label(labels, M._implementing_label)
    or M.has_label(labels, M._pr_open_label)
    or M.has_label(labels, M._reviewing_label)
    or M.has_label(labels, M._review_meta_label)
    or M.has_label(labels, M._merge_ready_label)
    or M.has_label(labels, M._merging_label)
    or M.has_label(labels, M._merged_label)
    or M.has_label(labels, M._fixing_label)
    or M.has_label(labels, M._impl_failed_label)
    or M.has_label(labels, M._blocked_label)
end

function M.is_loop_terminal(labels)
  return M.has_label(labels, M._ready_label)
    or M.has_label(labels, M._implementing_label)
    or M.has_label(labels, M._pr_open_label)
    or M.has_label(labels, M._reviewing_label)
    or M.has_label(labels, M._review_meta_label)
    or M.has_label(labels, M._merge_ready_label)
    or M.has_label(labels, M._merging_label)
    or M.has_label(labels, M._merged_label)
    or M.has_label(labels, M._fixing_label)
    or M.has_label(labels, M._impl_failed_label)
    or M.has_label(labels, M._blocked_label)
    or M.has_label(labels, M._stuck_label)
end

function M.has_result_marker(comments, proposal_id, decision, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  -- Match the FULL marker (proposal + decision + dedup) so a stale opposite/older-version marker
  -- does not suppress writing the current decision's result marker.
  local needle = M.result_marker(proposal_id, decision, dedup_key)
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    if M._comment_body(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end


M._strip_latest_fix_version_suffix = strip_latest_fix_version_suffix
M._compare_transition_versions = compare_transition_versions
end

return S
