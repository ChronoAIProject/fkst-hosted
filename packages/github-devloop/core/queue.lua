local S = {}

function S.install(M)
local active_wip_states = {
  implementing = true,
  ["pr-open"] = true,
  reviewing = true,
  fixing = true,
  ["review-meta"] = true,
  ["merge-ready"] = true,
  merging = true,
}

local merge_queue_lane_states = {
  ["merge-ready"] = true,
  merging = true,
}

local function compare_merge_queue_entries(left, right)
  local left_created = tostring(left.merge_ready_created_at or "")
  local right_created = tostring(right.merge_ready_created_at or "")
  if left_created ~= right_created then
    return left_created < right_created
  end
  return tonumber(left.pr_number or 0) < tonumber(right.pr_number or 0)
end

local function parse_name_only_paths(stdout)
  local paths = {}
  local seen = {}
  for line in tostring(stdout or ""):gmatch("[^\r\n]+") do
    local path = line:gsub("^%s+", ""):gsub("%s+$", "")
    if path ~= "" and not seen[path] then
      table.insert(paths, path)
      seen[path] = true
    end
  end
  table.sort(paths)
  return paths
end

local function path_set(paths)
  local set = {}
  for _, path in ipairs(paths or {}) do
    set[path] = true
  end
  return set
end

local function intersecting_path(left, right)
  for path in pairs(left or {}) do
    if right[path] then
      return path
    end
  end
  return nil
end

local function current_any_entity_state(M, entity_comments)
  local best = nil
  local marker_pattern = "<!%-%- fkst:github%-devloop:state:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(entity_comments or {})) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      if marker_proposal ~= nil and M.parse_entity_proposal_id(marker_proposal) ~= nil then
        local candidate = M.current_state(entity_comments, marker_proposal)
        candidate.proposal_id = marker_proposal
        if best == nil or M.compare_state_marker_order(best, candidate.state, candidate.version) < 0 then
          best = candidate
        end
      end
    end
  end
  return best or {
    state = nil,
    version = nil,
    stage_rank = 0,
  }
end

local function merge_ready_version_for_lane_state(M, state)
  local version = tostring((state or {}).version or "")
  if state ~= nil and state.state == "fixing" and M._strip_latest_fix_version_suffix ~= nil then
    return M._strip_latest_fix_version_suffix(version)
  end
  return version
end

local function merge_queue_entry_from_pr(M, repo, pr_number, pr, expected_base)
  if type(pr) ~= "table" or tostring(pr.state or ""):upper() ~= "OPEN" then
    return nil
  end
  if tostring(pr.base_ref_name or "") ~= tostring(expected_base or "") then
    return nil
  end
  local state = current_any_entity_state(M, pr.comments)
  if not merge_queue_lane_states[state.state] then
    return nil
  end
  local current_head_sha = tostring(pr.head_sha or "")
  local fact = M.merge_ready_fact(pr.comments, state.proposal_id or "", merge_ready_version_for_lane_state(M, state), pr_number, current_head_sha)
  if fact == nil then
    for _, comment in ipairs(M._trusted_marker_comments(pr.comments)) do
      for marker in M._comment_body(comment):gmatch("<!%-%- fkst:github%-devloop:merge%-ready:v1.-%-%->") do
        local marker_issue = marker:match('proposal="([^"]+)"')
        if marker_issue ~= nil then
          local candidate_state = M.current_entity_state(pr.comments, marker_issue)
          local merge_ready_version = merge_ready_version_for_lane_state(M, candidate_state)
          if merge_queue_lane_states[candidate_state.state]
            and tostring(merge_ready_version or "") == tostring(marker:match('version="([^"]*)"') or "") then
            fact = M.merge_ready_fact(pr.comments, marker_issue, merge_ready_version, pr_number, current_head_sha)
            state = candidate_state
            break
          end
        end
      end
      if fact ~= nil then
        break
      end
    end
  end
  if fact == nil then
    return nil
  end
  return {
    pr_number = tonumber(pr_number),
    proposal_id = fact.proposal_id,
    version = fact.version,
    review_proposal_id = fact.review_proposal_id,
    review_dedup_key = fact.review_dedup_key,
    state = state.state,
    head_branch = pr.head_ref_name,
    head_sha = fact.head_sha,
    base_sha = pr.base_ref_oid,
    merge_ready_created_at = fact.comment_created_at or "",
  }
end

function M.merge_queue_head(repo, base_branch, current)
  local entries = {}
  local seen = {}
  if type(current) == "table" and current.pr_number ~= nil and type(current.pr) == "table" then
    local entry = merge_queue_entry_from_pr(M, repo, current.pr_number, current.pr, base_branch)
    if entry ~= nil then
      table.insert(entries, entry)
      seen[tostring(entry.pr_number)] = true
    end
  end

  local list = M.gh_exec({ cmd = M.gh_pr_list_merge_queue_cmd(repo, base_branch), timeout = 30 })
  if list.exit_code ~= 0 then
    error("github-devloop: merge queue PR list failed: " .. tostring(list.stderr))
  end
  for _, pr_item in ipairs(M.parse_pr_list_merge_queue(list.stdout)) do
    local pr_number = tonumber(pr_item.number)
    if pr_number ~= nil and not seen[tostring(pr_number)] then
      local view = M.gh_exec({ cmd = M.gh_pr_view_merge_cmd(repo, pr_number), timeout = 30 })
      if view.exit_code ~= 0 then
        error("github-devloop: merge queue PR view failed: " .. tostring(view.stderr))
      end
      local pr = M.parse_pr_view_merge(view.stdout)
      local entry = merge_queue_entry_from_pr(M, repo, pr_number, pr, base_branch)
      if entry ~= nil then
        table.insert(entries, entry)
        seen[tostring(entry.pr_number)] = true
      end
    end
  end
  table.sort(entries, compare_merge_queue_entries)
  return entries[1], entries
end

function M.merge_queue_allows_event(repo, base_branch, merge_ready, current_pr)
  local head = M.merge_queue_head(repo, base_branch, {
    pr_number = merge_ready.pr_number,
    pr = current_pr,
  })
  if head == nil then
    return false, "merge-queue-empty"
  end
  if tostring(head.proposal_id or "") ~= tostring(merge_ready.proposal_id or "")
    or tostring(head.version or "") ~= tostring(merge_ready.version or "")
    or tostring(head.pr_number or "") ~= tostring(merge_ready.pr_number or "")
    or tostring(head.head_sha or "") ~= tostring(merge_ready.reviewed_head_sha or "") then
    return false, "merge-queue-head-pr-" .. tostring(head.pr_number or "unknown")
  end
  return true, "merge-queue-head"
end

function M.merge_queue_tick_dedup_key(repo, merged_pr_number, next_entry)
  if type(next_entry) ~= "table" then
    error("github-devloop: invalid merge queue next entry")
  end
  return M._dedup_key({
    "merge-queue",
    "requeue",
    M.safe_repo(repo),
    "merged-pr",
    M.safe_issue(merged_pr_number),
    "next-pr",
    M.safe_issue(next_entry.pr_number),
    M.safe_head_segment(next_entry.head_sha),
  })
end

function M.merge_queue_tick_payload(repo, merged_pr_number, next_entry)
  if type(next_entry) ~= "table" then
    return nil
  end
  return {
    schema = "github-devloop.merge-queue-tick.v1",
    dedup_key = M.merge_queue_tick_dedup_key(repo, merged_pr_number, next_entry),
    source_ref = M.pr_source_ref(repo, next_entry.pr_number),
    cause = {
      kind = "merge-progress",
      merged_pr_number = tonumber(merged_pr_number),
      next_pr_number = tonumber(next_entry.pr_number),
      next_head_sha = tostring(next_entry.head_sha or ""),
    },
  }
end

function M.merge_ready_payload_from_queue_entry(entry, source_ref)
  if type(entry) ~= "table" then
    return nil
  end
  return M.build_devloop_merge_ready_payload(
    entry.proposal_id,
    entry.pr_number,
    entry.version,
    {
      review_proposal_id = entry.review_proposal_id,
      review_dedup_key = entry.review_dedup_key,
      reviewed_head_sha = entry.head_sha,
    },
    source_ref
  )
end

function M.merge_queue_changed_files(repo, entry)
  local result = M.gh_exec({ cmd = M.gh_pr_diff_name_only_cmd(repo, entry.pr_number), timeout = 30 })
  if result.exit_code ~= 0 then
    return nil, "diff-name-only-failed: " .. tostring(result.stderr)
  end
  local paths = parse_name_only_paths(result.stdout)
  return {
    pr_number = entry.pr_number,
    proposal_id = entry.proposal_id,
    version = entry.version,
    base_sha = entry.base_sha,
    head_sha = entry.head_sha,
    paths = paths,
    set = path_set(paths),
  }, "changed-files-ok"
end

function M.merge_queue_files_disjoint(left, right)
  local path = intersecting_path(left and left.set, right and right.set)
  if path ~= nil then
    return false, path
  end
  return true, "disjoint"
end

function M.wip_capacity_allows_start(repo, current_issue_number)
  local max_inflight = M.max_inflight()
  if max_inflight == nil then
    return true, "wip-cap-disabled", 0, nil
  end

  local list = M.gh_exec({ cmd = M.gh_issue_list_wip_cmd(repo), timeout = 30 })
  if list.exit_code ~= 0 then
    error("github-devloop: WIP issue list failed: " .. tostring(list.stderr))
  end

  local count = 0
  for _, issue in ipairs(M.parse_issue_number_list(list.stdout)) do
    local issue_number = tonumber(issue.number)
    if issue_number ~= nil and tostring(issue_number) ~= tostring(current_issue_number) then
      local view = M.gh_exec({ cmd = M.gh_issue_view_state_cmd(repo, issue_number), timeout = 30 })
      if view.exit_code ~= 0 then
        error("github-devloop: WIP issue state view failed: " .. tostring(view.stderr))
      end
      local current = M.parse_issue_view_state(view.stdout)
      local proposal_id = M.proposal_id(repo, issue_number)
      local state = M.current_state(current.comments, proposal_id)
      if active_wip_states[state.state] then
        count = count + 1
      end
    end
  end
  if count >= max_inflight then
    return false, "wip-cap-reached", count, max_inflight
  end
  return true, "wip-cap-available", count, max_inflight
end
end

return S
