local M = {}
local root_ref = nil

local max_dependency_depth = 32

local function root()
  return root_ref or M
end

local function split_repo(repo)
  local owner, name = tostring(repo or ""):match("^([^/]+)/([^/]+)$")
  if owner == nil or owner == "" or name == nil or name == "" then
    return nil, nil
  end
  return owner, name
end

local function gate(kind, reason, unmet)
  return {
    ok = kind == "satisfied",
    kind = kind,
    unmet = unmet or {},
    reason = reason,
  }
end

local function add_gate_note(notes, note)
  if type(notes) ~= "table" or type(note) ~= "table" then
    return
  end
  table.insert(notes, note)
end

local function add_unmet(unmet, seen, number)
  local core = root()
  if not core._is_positive_pr_number(number) then
    return
  end
  local value = tonumber(number)
  if seen[value] then
    return
  end
  seen[value] = true
  table.insert(unmet, value)
end

local function dependency_unmet_field(unmet_numbers)
  local core = root()
  local parts = {}
  for _, number in ipairs(unmet_numbers or {}) do
    if core._is_positive_pr_number(number) then
      local next_value = tostring(math.floor(tonumber(number)))
      local candidate = #parts == 0 and next_value or (table.concat(parts, ",") .. "," .. next_value)
      if #candidate > 200 then
        break
      end
      table.insert(parts, next_value)
    end
  end
  return table.concat(parts, ",")
end

local function marker_attr(marker, name)
  return tostring(marker or ""):match(name .. '="([^"]*)"')
end

local function safe_dependency_attr(value)
  local core = root()
  local text = tostring(value or "")
  text = text:gsub("<!%-%- fkst:[^\n]*%-%->", " ")
  text = text:gsub("&lt;!%-%- fkst:[^\n]*%-%-&gt;", " ")
  text = text:gsub("%c", " "):gsub('"', "'"):gsub("[<>]", ""):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  if #text > 240 then
    text = core.truncate_utf8(text, 240)
  end
  return text
end

local function decode_dependency_attr(value)
  if type(value) ~= "string" or value == "" then
    return nil
  end
  if value:find("%c") ~= nil or value:find("[<>]") ~= nil or value:find('"', 1, true) ~= nil then
    return nil
  end
  return value
end

local function parse_blocked_by(stdout)
  local core = root()
  local ok, decoded = pcall(json.decode, stdout or "")
  if not ok or type(decoded) ~= "table" then
    return nil
  end
  local issue = decoded.data
    and decoded.data.repository
    and decoded.data.repository.issue
  if type(issue) ~= "table" then
    return nil
  end
  local blocked_by = issue.blockedBy
  local nodes = blocked_by and blocked_by.nodes
  if type(nodes) ~= "table" then
    return nil
  end

  local blockers = {}
  for _, node in ipairs(nodes) do
    if type(node) ~= "table" or not core._is_positive_pr_number(node.number) then
      return nil
    end
    local blocker_repo = node.repository and node.repository.nameWithOwner
    if type(blocker_repo) ~= "string" or blocker_repo == "" then
      return nil
    end
    table.insert(blockers, {
      number = tonumber(node.number),
      state = tostring(node.state or ""),
      state_reason = tostring(node.stateReason or node.state_reason or ""),
      repo = blocker_repo,
    })
  end

  -- Fail-closed on a truncated blockedBy list: an unseen 51st+ unmet blocker must
  -- never be read as absent (that would let dependency_gate return ok=true falsely).
  local truncated = false
  local total = blocked_by.totalCount
  local page = blocked_by.pageInfo
  if (type(total) == "number" and total > #blockers)
    or (type(page) == "table" and page.hasNextPage == true) then
    truncated = true
  end
  return blockers, truncated
end

local function normalized_state_reason(value)
  local text = tostring(value or ""):lower():gsub("_", "-")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  return text
end

local function fetch_blocked_by(repo, issue_number)
  local core = root()
  local result = core.gh_exec({ cmd = core.gh_blocked_by_cmd(repo, issue_number), timeout = 30 })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    return nil, "gh-failed"
  end
  local blockers, truncated = parse_blocked_by(result.stdout)
  if blockers == nil then
    return nil, "malformed-json"
  end
  if truncated then
    return nil, "blockedby-truncated"
  end
  return blockers, nil
end

local function merged_blocker_cache_key(repo, blocker_number)
  local core = root()
  if not core.issue_ref_round_trips(repo, blocker_number) then
    error("github-devloop: invalid merged blocker cache key target")
  end
  local key = "github-devloop/dependency/merged/"
    .. core.safe_repo(repo)
    .. "/issue/"
    .. core.safe_issue(blocker_number)
  if not core._is_path_safe_key(key, core._max_key_len) then
    error("github-devloop: invalid merged blocker cache key")
  end
  return key
end

local function cached_blocker_merged(repo, blocker_number)
  local key = merged_blocker_cache_key(repo, blocker_number)
  return cache_get(key) == "1"
end

local function cache_blocker_merged(repo, blocker_number)
  local key = merged_blocker_cache_key(repo, blocker_number)
  cache_set(key, "1")
end

local function blocker_merged(repo, blocker_number)
  local core = root()
  local blocker_proposal_id = core.proposal_id(repo, blocker_number)
  local result = core.gh_exec({ cmd = core.gh_issue_view_observe_cmd(repo, blocker_number), timeout = 30 })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    return nil, "gh-failed"
  end
  local ok, current = pcall(core.parse_issue_view_observe, result.stdout)
  if not ok or type(current) ~= "table" then
    return nil, "malformed-json"
  end
  local state = core.current_entity_state(current.comments, blocker_proposal_id)
  if type(state) == "table" and state.state == "merged" then
    return true, nil
  end

  local link = core.pr_link_fact(current.comments, blocker_proposal_id)
  if link == nil then
    return false, nil
  end

  local pr_result = core.gh_exec({ cmd = core.gh_pr_view_observe_cmd(repo, link.pr_number), timeout = 30 })
  if type(pr_result) ~= "table" or pr_result.exit_code ~= 0 then
    return nil, "gh-pr-failed"
  end
  local pr_ok, pr_current = pcall(core.parse_pr_view_origin, pr_result.stdout)
  if not pr_ok or type(pr_current) ~= "table" then
    return nil, "malformed-pr-json"
  end
  local origin = core.pr_origin_fact(pr_current.comments)
  if origin == nil
    or tostring(origin.proposal_id or "") ~= blocker_proposal_id
    or tostring(origin.repo or "") ~= tostring(repo)
    or tostring(origin.issue_number or "") ~= tostring(blocker_number)
    or tostring(origin.branch or "") ~= tostring(link.branch or "")
    or tostring(origin.impl_version or "") ~= tostring(link.impl_version or "")
    or tostring(origin.base_branch or "") ~= tostring(link.base_branch or "") then
    return nil, "pr-origin-mismatch"
  end

  local pr_state = core.current_entity_state(pr_current.comments, blocker_proposal_id)
  if type(pr_state) ~= "table" or pr_state.state ~= "merged" then
    return false, nil
  end
  local merged = core.merged_fact(pr_current.comments, blocker_proposal_id, link.pr_number, pr_state.version)
  return merged ~= nil, nil
end

local function prove_blocker_merged(repo, blocker_number)
  if cached_blocker_merged(repo, blocker_number) then
    return true, nil
  end

  local merged, reason = blocker_merged(repo, blocker_number)
  if merged == true then
    cache_blocker_merged(repo, blocker_number)
  end
  return merged, reason
end

function M.dependency_waiver_fact(comments, proposal_id, version, blocker_number)
  local core = root()
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:dependency%-waiver:v1.-%-%->"
  for _, comment in ipairs(core._trusted_marker_comments(comments)) do
    for marker in core._comment_body(comment):gmatch(marker_pattern) do
      if marker_attr(marker, "proposal") == tostring(proposal_id)
        and marker_attr(marker, "version") == tostring(version)
        and tonumber(marker_attr(marker, "blocker") or "") == tonumber(blocker_number) then
        return {
          proposal_id = tostring(proposal_id),
          version = tostring(version),
          blocker_number = tonumber(blocker_number),
          reason = decode_dependency_attr(marker_attr(marker, "reason")) or "dependency-waiver",
          comment_created_at = core._comment_created_at(comment),
        }
      end
    end
  end
  return nil
end

local function has_dependency_waiver(context, blocker_number)
  if type(context) ~= "table" then
    return false
  end
  return M.dependency_waiver_fact(
    context.comments,
    context.proposal_id,
    context.version,
    blocker_number
  ) ~= nil
end

local visit
visit = function(repo, issue_number, stack, visited, unmet, unmet_seen, depth, context, notes)
  if depth > max_dependency_depth then
    add_unmet(unmet, unmet_seen, issue_number)
    return gate("unresolvable", "depth-cap-exceeded", unmet)
  end

  local key = tostring(repo) .. "#" .. tostring(issue_number)
  if stack[key] then
    add_unmet(unmet, unmet_seen, issue_number)
    return gate("cycle", "dependency-cycle", unmet)
  end
  if visited[key] then
    return gate("satisfied", "satisfied", unmet)
  end

  stack[key] = true
  local blockers, fetch_reason = fetch_blocked_by(repo, issue_number)
  if blockers == nil then
    stack[key] = nil
    add_unmet(unmet, unmet_seen, issue_number)
    return gate("unresolvable", fetch_reason or "gh-failed", unmet)
  end

  for _, blocker in ipairs(blockers) do
    if tostring(blocker.repo or "") ~= tostring(repo) then
      stack[key] = nil
      add_unmet(unmet, unmet_seen, blocker.number)
      return gate("unresolvable", "cross-repo-blocker", unmet)
    end

    local state_reason = normalized_state_reason(blocker.state_reason)
    if blocker.state == "CLOSED" and state_reason == "not-planned" then
      add_gate_note(notes, {
        kind = "dependency-void",
        blocker_number = blocker.number,
        reason = "not_planned",
      })
    elseif not cached_blocker_merged(repo, blocker.number) then
      local prefer_terminal_proof = blocker.state == "CLOSED"
      local merged = nil
      local merged_reason = nil

      if prefer_terminal_proof then
        merged, merged_reason = prove_blocker_merged(repo, blocker.number)
      end

      if not prefer_terminal_proof or (merged == false and state_reason ~= "completed") then
        local nested = visit(repo, blocker.number, stack, visited, unmet, unmet_seen, depth + 1, context, notes)
        if nested.kind == "cycle" or nested.kind == "unresolvable" then
          stack[key] = nil
          return nested
        end
      end

      if not prefer_terminal_proof then
        merged, merged_reason = prove_blocker_merged(repo, blocker.number)
      end

      if merged == nil then
        stack[key] = nil
        add_unmet(unmet, unmet_seen, blocker.number)
        return gate("unresolvable", merged_reason or "unknown-blocker", unmet)
      end
      if not merged then
        if prefer_terminal_proof and state_reason == "completed" and has_dependency_waiver(context, blocker.number) then
          add_gate_note(notes, {
            kind = "dependency-waiver",
            blocker_number = blocker.number,
            reason = "completed_without_merged_marker",
          })
        else
          add_unmet(unmet, unmet_seen, blocker.number)
          if prefer_terminal_proof and state_reason == "completed" then
            stack[key] = nil
            return gate("waiting", "dependency-waiver-required", unmet)
          end
        end
      end
    end
  end

  stack[key] = nil
  visited[key] = true
  if #unmet > 0 then
    return gate("waiting", "waiting-on-dependency", unmet)
  end
  local result = gate("satisfied", "satisfied", {})
  if type(notes) == "table" and #notes > 0 then
    result.reason = notes[1].kind
    result.notes = notes
  end
  return result
end

function M.gh_blocked_by_cmd(repo, issue_number)
  local core = root()
  local owner, name = split_repo(repo)
  if owner == nil or not core._is_positive_pr_number(issue_number) then
    error("github-devloop: invalid dependency query target")
  end
  local query = '{repository(owner:"' .. owner .. '",name:"' .. name
    .. '"){issue(number:' .. tostring(math.floor(tonumber(issue_number)))
    .. '){blockedBy(first:50){totalCount pageInfo{hasNextPage} nodes{number state stateReason repository{nameWithOwner}}}}}}'
  return "gh api graphql -f query=" .. core._shell_single_quote(query)
end

function M.dependency_gate(repo, issue_number, context)
  local core = root()
  if split_repo(repo) == nil or not core._is_positive_pr_number(issue_number) then
    return gate("unresolvable", "invalid-target", {})
  end
  local ok, result = pcall(visit, repo, issue_number, {}, {}, {}, {}, 0, context, {})
  if not ok or type(result) ~= "table" then
    return gate("unresolvable", "dependency-gate-exception", {})
  end
  result.ok = result.kind == "satisfied"
  return result
end

M.merged_blocker_cache_key = merged_blocker_cache_key

function M.dependency_wait_marker(proposal_id, version, unmet_numbers, hold_kind, reason)
  return '<!-- fkst:github-devloop:dependency-wait:v1 proposal="' .. tostring(proposal_id)
    .. '" version="' .. tostring(version)
    .. '" hold_kind="' .. safe_dependency_attr(hold_kind or "waiting")
    .. '" reason="' .. safe_dependency_attr(reason or "waiting-on-dependency")
    .. '" unmet="' .. dependency_unmet_field(unmet_numbers)
    .. '" -->'
end

function M.dependency_cycle_marker(proposal_id, version)
  return '<!-- fkst:github-devloop:dependency-cycle:v1 proposal="' .. tostring(proposal_id)
    .. '" version="' .. tostring(version)
    .. '" -->'
end

function M.dependency_unresolvable_marker(proposal_id, version, unmet_numbers, hold_kind, reason)
  return '<!-- fkst:github-devloop:dependency-unresolvable:v1 proposal="' .. tostring(proposal_id)
    .. '" version="' .. tostring(version)
    .. '" hold_kind="' .. safe_dependency_attr(hold_kind or "unresolvable")
    .. '" reason="' .. safe_dependency_attr(reason or "gh-failed")
    .. '" unmet="' .. dependency_unmet_field(unmet_numbers)
    .. '" -->'
end

function M.dependency_release_marker(proposal_id, version)
  return '<!-- fkst:github-devloop:dependency-release:v1 proposal="' .. tostring(proposal_id)
    .. '" version="' .. tostring(version)
    .. '" -->'
end

function M.dependency_void_marker(proposal_id, version, blocker_number, reason)
  return '<!-- fkst:github-devloop:dependency-void:v1 proposal="' .. tostring(proposal_id)
    .. '" version="' .. tostring(version)
    .. '" blocker="' .. dependency_unmet_field({ blocker_number })
    .. '" reason="' .. safe_dependency_attr(reason or "not_planned")
    .. '" -->'
end

function M.dependency_waiver_marker(proposal_id, version, blocker_number, reason)
  return '<!-- fkst:github-devloop:dependency-waiver:v1 proposal="' .. tostring(proposal_id)
    .. '" version="' .. tostring(version)
    .. '" blocker="' .. dependency_unmet_field({ blocker_number })
    .. '" reason="' .. safe_dependency_attr(reason or "dependency-waiver")
    .. '" -->'
end

function M.dependency_gate_note_markers(proposal_id, version, gate_result)
  local lines = {}
  if type(gate_result) ~= "table" or type(gate_result.notes) ~= "table" then
    return ""
  end
  for _, note in ipairs(gate_result.notes) do
    if type(note) == "table" and note.kind == "dependency-void" then
      table.insert(lines, M.dependency_void_marker(proposal_id, version, note.blocker_number, note.reason))
    elseif type(note) == "table" and note.kind == "dependency-waiver" then
      table.insert(lines, M.dependency_waiver_marker(proposal_id, version, note.blocker_number, note.reason))
    end
  end
  return table.concat(lines, "\n")
end

function M.dependency_gate_has_notes(gate_result)
  return type(gate_result) == "table"
    and type(gate_result.notes) == "table"
    and #gate_result.notes > 0
end

function M.dependency_hold_fact(comments, proposal_id)
  local core = root()
  if type(comments) ~= "table" then
    return nil
  end
  local current = core.current_state(comments, proposal_id)
  if type(current) ~= "table" or current.version == nil then
    return nil
  end
  local wait_pattern = "<!%-%- fkst:github%-devloop:dependency%-wait:v1.-%-%->"
  local cycle_pattern = "<!%-%- fkst:github%-devloop:dependency%-cycle:v1.-%-%->"
  local unresolvable_pattern = "<!%-%- fkst:github%-devloop:dependency%-unresolvable:v1.-%-%->"
  for _, comment in ipairs(core._trusted_marker_comments(comments)) do
    local body = core._comment_body(comment)
    local hold_kind = body:match("github%-devloop dependency hold:%s*([^\n]+)")
    local reason = body:match("Reason:%s*([^\n]+)")
    for marker in body:gmatch(wait_pattern) do
      if marker_attr(marker, "proposal") == tostring(proposal_id)
        and marker_attr(marker, "version") == tostring(current.version) then
        return {
          proposal_id = tostring(proposal_id),
          version = tostring(current.version),
          marker_kind = "dependency-wait",
          hold_kind = decode_dependency_attr(marker_attr(marker, "hold_kind")) or hold_kind or "waiting",
          reason = decode_dependency_attr(marker_attr(marker, "reason")) or reason or "waiting-on-dependency",
          comment_created_at = core._comment_created_at(comment),
        }
      end
    end
    for marker in body:gmatch(cycle_pattern) do
      if marker_attr(marker, "proposal") == tostring(proposal_id)
        and marker_attr(marker, "version") == tostring(current.version) then
        return {
          proposal_id = tostring(proposal_id),
          version = tostring(current.version),
          marker_kind = "dependency-cycle",
          hold_kind = hold_kind or "cycle",
          reason = reason or "dependency-cycle",
          comment_created_at = core._comment_created_at(comment),
        }
      end
    end
    for marker in body:gmatch(unresolvable_pattern) do
      if marker_attr(marker, "proposal") == tostring(proposal_id)
        and marker_attr(marker, "version") == tostring(current.version) then
        return {
          proposal_id = tostring(proposal_id),
          version = tostring(current.version),
          marker_kind = "dependency-unresolvable",
          hold_kind = decode_dependency_attr(marker_attr(marker, "hold_kind")) or hold_kind or "unresolvable",
          reason = decode_dependency_attr(marker_attr(marker, "reason")) or reason or "gh-failed",
          comment_created_at = core._comment_created_at(comment),
        }
      end
    end
  end
  return nil
end

function M.dependency_release_fact(comments, proposal_id, version)
  local core = root()
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:dependency%-release:v1.-%-%->"
  for _, comment in ipairs(core._trusted_marker_comments(comments)) do
    for marker in core._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      if marker_proposal == tostring(proposal_id)
        and marker_version == tostring(version) then
        return {
          proposal_id = marker_proposal,
          version = marker_version,
          comment_created_at = core._comment_created_at(comment),
        }
      end
    end
  end
  return nil
end

function M.dependency_wait_fact(comments, proposal_id)
  local core = root()
  if type(comments) ~= "table" then
    return nil
  end
  local current = core.current_state(comments, proposal_id)
  if type(current) ~= "table" or current.version == nil then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:dependency%-wait:v1.-%-%->"
  for _, comment in ipairs(core._trusted_marker_comments(comments)) do
    for marker in core._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      if marker_proposal == tostring(proposal_id)
        and marker_version == tostring(current.version) then
        return {
          proposal_id = marker_proposal,
          version = marker_version,
          comment_created_at = core._comment_created_at(comment),
        }
      end
    end
  end
  return nil
end

function M.install(root_module)
  root_ref = root_module
  for k, v in pairs(M) do
    if k ~= "install" then
      root_module[k] = v
    end
  end
end

return M
