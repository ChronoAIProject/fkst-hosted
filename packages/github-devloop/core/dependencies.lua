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

local function fetch_blocked_by(repo, issue_number)
  local core = root()
  local result = exec_sync({ cmd = core.gh_blocked_by_cmd(repo, issue_number), timeout = 30 })
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

local function blocker_merged(repo, blocker_number)
  local core = root()
  local result = exec_sync({ cmd = core.gh_issue_view_observe_cmd(repo, blocker_number), timeout = 30 })
  if type(result) ~= "table" or result.exit_code ~= 0 then
    return nil, "gh-failed"
  end
  local ok, current = pcall(core.parse_issue_view_observe, result.stdout)
  if not ok or type(current) ~= "table" then
    return nil, "malformed-json"
  end
  local state = core.current_entity_state(current.comments, core.proposal_id(repo, blocker_number))
  if type(state) ~= "table" then
    return nil, "unknown-blocker"
  end
  return state.state == "merged", nil
end

local visit
visit = function(repo, issue_number, stack, visited, unmet, unmet_seen, depth)
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

    local nested = visit(repo, blocker.number, stack, visited, unmet, unmet_seen, depth + 1)
    if nested.kind == "cycle" or nested.kind == "unresolvable" then
      stack[key] = nil
      return nested
    end

    local merged, merged_reason = blocker_merged(repo, blocker.number)
    if merged == nil then
      stack[key] = nil
      add_unmet(unmet, unmet_seen, blocker.number)
      return gate("unresolvable", merged_reason or "unknown-blocker", unmet)
    end
    if not merged then
      add_unmet(unmet, unmet_seen, blocker.number)
    end
  end

  stack[key] = nil
  visited[key] = true
  if #unmet > 0 then
    return gate("waiting", "waiting-on-dependency", unmet)
  end
  return gate("satisfied", "satisfied", {})
end

function M.gh_blocked_by_cmd(repo, issue_number)
  local core = root()
  local owner, name = split_repo(repo)
  if owner == nil or not core._is_positive_pr_number(issue_number) then
    error("github-devloop: invalid dependency query target")
  end
  local query = '{repository(owner:"' .. owner .. '",name:"' .. name
    .. '"){issue(number:' .. tostring(math.floor(tonumber(issue_number)))
    .. '){blockedBy(first:50){totalCount pageInfo{hasNextPage} nodes{number state repository{nameWithOwner}}}}}}'
  return "gh api graphql -f query=" .. core._shell_single_quote(query)
end

function M.dependency_gate(repo, issue_number)
  local core = root()
  if split_repo(repo) == nil or not core._is_positive_pr_number(issue_number) then
    return gate("unresolvable", "invalid-target", {})
  end
  local ok, result = pcall(visit, repo, issue_number, {}, {}, {}, {}, 0)
  if not ok or type(result) ~= "table" then
    return gate("unresolvable", "dependency-gate-exception", {})
  end
  result.ok = result.kind == "satisfied"
  return result
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
