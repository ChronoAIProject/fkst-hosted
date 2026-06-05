local M = {}

local max_key_len = 200
local max_title_len = 240
local max_body_len = 12000
local max_repo_key_len = 100
local max_issue_key_len = 30
local max_update_key_len = 50

local enabled_label = "fkst-dev:enabled"
local thinking_label = "fkst-dev:thinking"
local ready_label = "fkst-dev:ready"
local blocked_label = "fkst-dev:blocked"
local stuck_label = "fkst-dev:stuck"
local loop_budget = 3

local state_labels = {
  [thinking_label] = true,
  [ready_label] = true,
  [blocked_label] = true,
  [stuck_label] = true,
}

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function is_bounded_string(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

local function is_path_safe_key(value, limit)
  if not is_bounded_string(value, limit or max_key_len) then
    return false
  end
  if value:sub(1, 1) == "/" then
    return false
  end
  if value:find("\\", 1, true) ~= nil then
    return false
  end
  if value:find("%s") ~= nil then
    return false
  end
  if value:find("[^%w%._%-%/#]") ~= nil then
    return false
  end
  for segment in value:gmatch("[^/]+") do
    if segment == "." or segment == ".." then
      return false
    end
  end
  return true
end

local function has_bounded_source_ref(source_ref)
  return type(source_ref) == "table"
    and is_bounded_string(source_ref.kind, max_key_len)
    and is_bounded_string(source_ref.ref, max_key_len)
end

function M.sanitize_key(value)
  local sanitized = tostring(value or ""):gsub("[^%w%._%-%/#]", "-")
  sanitized = sanitized:gsub("/+", "/")
  sanitized = sanitized:gsub("^/+", ""):gsub("/+$", "")
  if sanitized == "" then
    return "empty"
  end

  local segments = {}
  for segment in sanitized:gmatch("[^/]+") do
    if segment == "." or segment == ".." then
      segment = "-"
    end
    table.insert(segments, segment)
  end

  sanitized = table.concat(segments, "/")
  if #sanitized > max_key_len then
    sanitized = sanitized:sub(1, max_key_len)
    sanitized = sanitized:gsub("/+$", "")
  end
  if sanitized == "" then
    return "empty"
  end
  return sanitized
end

function M.safe_repo(repo)
  local safe = M.sanitize_key(repo):sub(1, max_repo_key_len):gsub("/+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

function M.safe_issue(issue_number)
  local safe = M.sanitize_key(issue_number):sub(1, max_issue_key_len):gsub("/+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

function M.safe_updated_at(updated_at)
  local safe = M.sanitize_key(updated_at):sub(1, max_update_key_len):gsub("/+$", "")
  if safe == "" then
    return "empty"
  end
  return safe
end

function M.is_opted_in(labels)
  if type(labels) ~= "table" then
    return false
  end

  local enabled = false
  for _, label in ipairs(labels) do
    local name = tostring(label)
    if name == enabled_label then
      enabled = true
    end
    if state_labels[name] then
      return false
    end
  end
  return enabled
end

function M.proposal_id(repo, issue_number)
  return "github-devloop/issue/" .. M.safe_repo(repo) .. "/" .. M.safe_issue(issue_number)
end

function M.parse_proposal_id(id)
  if type(id) ~= "string" then
    return nil
  end

  local rest = id:match("^github%-devloop/issue/(.+)$")
  if rest == nil then
    return nil
  end

  local issue_number = rest:match("/([^/]+)$")
  local repo = issue_number and rest:sub(1, #rest - #issue_number - 1) or nil
  if repo == nil or repo == "" or issue_number == nil or issue_number == "" then
    return nil
  end
  return repo, issue_number
end

function M.is_safe_proposal_ref(proposal_id, dedup_key)
  if not is_path_safe_key(proposal_id, max_key_len) then
    return false
  end
  if not is_path_safe_key(dedup_key, max_key_len) then
    return false
  end

  local repo, issue_number = M.parse_proposal_id(proposal_id)
  if repo == nil or issue_number == nil then
    return false
  end
  return M.issue_ref_round_trips(repo, issue_number)
end

function M.is_safe_consensus_result_ref(proposal_id, dedup_key)
  if not is_path_safe_key(proposal_id, max_key_len) then
    return false
  end
  if not is_bounded_string(dedup_key, max_key_len) then
    return false
  end

  local inner_dedup_key = dedup_key:match("^consensus:(.+)$") or dedup_key
  if not is_path_safe_key(inner_dedup_key, max_key_len) then
    return false
  end

  local repo, issue_number = M.parse_proposal_id(proposal_id)
  if repo == nil or issue_number == nil then
    return false
  end
  return M.issue_ref_round_trips(repo, issue_number)
end

function M.issue_ref_round_trips(repo, issue_number)
  local repo_text = tostring(repo)
  local issue_text = tostring(issue_number)
  if M.safe_repo(repo) ~= repo_text then
    return false
  end
  if M.safe_issue(issue_number) ~= issue_text then
    return false
  end

  local parsed_repo, parsed_issue = M.parse_proposal_id(M.proposal_id(repo, issue_number))
  return parsed_repo == repo_text and parsed_issue == issue_text
end

function M.proposal_dedup_key(proposal_id, updated_at)
  return tostring(proposal_id) .. "/" .. M.safe_updated_at(updated_at)
end

function M.observe_lock_key(repo, issue_number)
  return "github-devloop/observe/" .. M.safe_repo(repo) .. "/issue/" .. M.safe_issue(issue_number)
end

function M.result_lock_key(proposal_id)
  local repo, issue_number = M.parse_proposal_id(proposal_id)
  if repo == nil then
    return nil
  end
  return "github-devloop/result/" .. M.safe_repo(repo) .. "/issue/" .. M.safe_issue(issue_number)
end

function M.loop_lock_key(proposal_id)
  local repo, issue_number = M.parse_proposal_id(proposal_id)
  if repo == nil then
    return nil
  end
  return "github-devloop/loop/" .. M.safe_repo(repo) .. "/issue/" .. M.safe_issue(issue_number)
end

function M.bounded_body(value)
  local text = tostring(value or "")
  if text == "" then
    return "(empty issue body)"
  end
  if #text <= max_body_len then
    return text
  end
  return text:sub(1, max_body_len)
end

function M.max_body_len()
  return max_body_len
end

function M.normalize_source_ref(source_ref)
  if not has_bounded_source_ref(source_ref) then
    error("github-devloop: invalid source_ref")
  end
  return {
    kind = source_ref.kind,
    ref = source_ref.ref,
  }
end

function M.gh_issue_view_body_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json body"
end

function M.gh_issue_view_state_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json labels,state"
end

function M.gh_issue_view_result_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json labels,comments"
end

function M.gh_issue_view_loop_cmd(repo, issue_number)
  return "gh issue view " .. shell_single_quote(issue_number)
    .. " --repo " .. shell_single_quote(repo)
    .. " --json title,body,updatedAt,labels,comments,state"
end

function M.parse_issue_view_body(stdout)
  local decoded = json.decode(stdout or "{}")
  return M.bounded_body(decoded.body)
end

function M.parse_issue_view_state(stdout)
  local decoded = json.decode(stdout or "{}")
  return M.issue_state_from_json(decoded)
end

function M.issue_state_from_json(decoded)
  local labels = {}
  for _, label in ipairs(decoded.labels or {}) do
    if type(label) == "table" and label.name ~= nil then
      table.insert(labels, tostring(label.name))
    elseif type(label) == "string" then
      table.insert(labels, label)
    end
  end

  return {
    labels = labels,
    state = decoded.state,
  }
end

function M.parse_issue_view_result(stdout)
  local decoded = json.decode(stdout or "{}")
  local state = M.issue_state_from_json(decoded)
  local comments = {}
  for _, comment in ipairs(decoded.comments or {}) do
    if type(comment) == "table" and comment.body ~= nil then
      table.insert(comments, tostring(comment.body))
    elseif type(comment) == "string" then
      table.insert(comments, comment)
    end
  end

  return {
    labels = state.labels,
    comments = comments,
  }
end

function M.parse_issue_view_loop(stdout)
  local decoded = json.decode(stdout or "{}")
  local result = M.parse_issue_view_result(stdout)
  return {
    title = tostring(decoded.title or ""),
    body = M.bounded_body(decoded.body),
    updated_at = decoded.updatedAt or decoded.updated_at,
    state = decoded.state,
    labels = result.labels,
    comments = result.comments,
  }
end

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

function M.has_terminal_label(labels)
  return M.has_label(labels, ready_label) or M.has_label(labels, blocked_label) or M.has_label(labels, stuck_label)
end

function M.has_thinking_label(labels)
  return M.has_label(labels, thinking_label)
end

function M.has_stuck_label(labels)
  return M.has_label(labels, stuck_label)
end

function M.has_decision_terminal_label(labels)
  return M.has_label(labels, ready_label) or M.has_label(labels, blocked_label)
end

function M.is_loop_terminal(labels)
  return M.has_label(labels, ready_label) or M.has_label(labels, blocked_label) or M.has_label(labels, stuck_label)
end

function M.has_result_marker(comments, proposal_id, decision, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  -- Match the FULL marker (proposal + decision + dedup) so a stale opposite/older-version marker
  -- does not suppress writing the current decision's result marker.
  local needle = M.result_marker(proposal_id, decision, dedup_key)
  for _, comment in ipairs(comments) do
    if tostring(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.loop_budget()
  return loop_budget
end

function M.loop_marker(proposal_id, n, dedup_key)
  return '<!-- fkst:github-devloop:loop:v1 proposal="' .. tostring(proposal_id)
    .. '" n="' .. tostring(n)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.stuck_marker(proposal_id, n, dedup_key)
  return '<!-- fkst:github-devloop:stuck:v1 proposal="' .. tostring(proposal_id)
    .. '" n="' .. tostring(n)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

local function marker_records(comments, kind, proposal_id)
  local records = {}
  if type(comments) ~= "table" then
    return records
  end

  local marker_pattern = "<!%-%- fkst:github%-devloop:" .. kind .. ":v1.-%-%->"
  for _, comment in ipairs(comments) do
    for marker in tostring(comment):gmatch(marker_pattern) do
      local marker_proposal = marker:match('proposal="([^"]+)"')
      local n = tonumber(marker:match('n="(%d+)"'))
      local dedup_key = marker:match('dedup="([^"]*)"')
      if marker_proposal == proposal_id and n ~= nil then
        table.insert(records, {
          n = n,
          dedup_key = dedup_key,
        })
      end
    end
  end
  return records
end

local function has_marker_round(comments, kind, proposal_id, n)
  for _, record in ipairs(marker_records(comments, kind, proposal_id)) do
    if record.n == n then
      return true
    end
  end
  return false
end

function M.has_loop_marker(comments, proposal_id, n, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  local needle = M.loop_marker(proposal_id, n, dedup_key)
  for _, comment in ipairs(comments) do
    if tostring(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_loop_marker_round(comments, proposal_id, n)
  return has_marker_round(comments, "loop", proposal_id, n)
end

function M.has_loop_marker_dedup(comments, proposal_id, dedup_key)
  for _, record in ipairs(marker_records(comments, "loop", proposal_id)) do
    if record.dedup_key == tostring(dedup_key) then
      return true
    end
  end
  return false
end

function M.has_stuck_marker(comments, proposal_id, n, dedup_key)
  if type(comments) ~= "table" then
    return false
  end
  local needle = M.stuck_marker(proposal_id, n, dedup_key)
  for _, comment in ipairs(comments) do
    if tostring(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.has_stuck_marker_round(comments, proposal_id, n)
  return has_marker_round(comments, "stuck", proposal_id, n)
end

function M.loop_count_from_github_markers(comments, proposal_id)
  local max_n = 0
  for _, record in ipairs(marker_records(comments, "loop", proposal_id)) do
    if record.n > max_n then
      max_n = record.n
    end
  end
  for _, record in ipairs(marker_records(comments, "stuck", proposal_id)) do
    if record.n > max_n then
      max_n = record.n
    end
  end
  return max_n
end

function M.parse_loop_round_from_dedup(dedup_key)
  local n = tostring(dedup_key or ""):match("/loop/(%d+)$")
  return tonumber(n) or 0
end

function M.build_proposal(issue, body)
  local proposal_id = M.proposal_id(issue.repo, issue.number)
  local title = tostring(issue.title or "")
  if #title > max_title_len then
    title = title:sub(1, max_title_len)
  end

  return {
    schema = "consensus.proposal.v1",
    proposal_id = proposal_id,
    title = title,
    body = M.bounded_body(body),
    dedup_key = M.proposal_dedup_key(proposal_id, issue.updated_at),
    source_ref = M.normalize_source_ref(issue.source_ref),
  }
end

function M.build_loop_proposal(repo, issue_number, current, source_ref, n)
  local issue = {
    repo = repo,
    number = issue_number,
    title = current.title,
    updated_at = current.updated_at,
    source_ref = source_ref,
  }
  local proposal = M.build_proposal(issue, current.body)
  proposal.dedup_key = proposal.dedup_key .. "/loop/" .. tostring(n)
  return proposal
end

function M.validate_proposal(proposal)
  if type(proposal) ~= "table" then
    return false
  end
  if proposal.schema ~= "consensus.proposal.v1" then
    return false
  end
  local repo, issue_number = M.parse_proposal_id(proposal.proposal_id)
  if repo == nil or issue_number == nil then
    return false
  end
  if not M.is_safe_proposal_ref(proposal.proposal_id, proposal.dedup_key) then
    return false
  end
  if not is_bounded_string(proposal.title, max_title_len) then
    return false
  end
  if not is_bounded_string(proposal.body, max_body_len) then
    return false
  end
  return has_bounded_source_ref(proposal.source_ref)
end

function M.result_marker(proposal_id, decision, dedup_key)
  if decision ~= "approve" and decision ~= "reject" then
    error("github-devloop: invalid decision")
  end
  return '<!-- fkst:github-devloop:result:v1 proposal="' .. tostring(proposal_id)
    .. '" decision="' .. decision
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.build_label_request(repo, issue_number, add_labels, remove_labels, dedup_key, source_ref)
  return {
    schema = "github-proxy.label.v1",
    repo = repo,
    issue_number = issue_number,
    add_labels = add_labels or {},
    remove_labels = remove_labels or {},
    dedup_key = dedup_key,
    source_ref = M.normalize_source_ref(source_ref),
  }
end

function M.build_thinking_label_request(issue, proposal)
  return M.build_label_request(
    issue.repo,
    issue.number,
    { thinking_label },
    {},
    proposal.dedup_key .. "/label/thinking",
    issue.source_ref
  )
end

function M.build_result_label_request(repo, issue_number, reached)
  local add_label = reached.decision == "approve" and ready_label or blocked_label
  -- Remove stale state labels so delayed/changed decisions cannot leave mutually exclusive
  -- fkst-dev:<state> labels coexisting.
  local opposite_label = reached.decision == "approve" and blocked_label or ready_label
  return M.build_label_request(
    repo,
    issue_number,
    { add_label },
    { thinking_label, opposite_label, stuck_label },
    tostring(reached.proposal_id) .. "/label/" .. tostring(reached.decision),
    reached.source_ref
  )
end

function M.build_result_comment_request(repo, issue_number, reached)
  local marker = M.result_marker(reached.proposal_id, reached.decision, reached.dedup_key)
  local body = "github-devloop decision: " .. tostring(reached.decision)
    .. "\n\n" .. tostring(reached.body or "")
    .. "\n\n" .. marker
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = body,
    -- Include the consensus dedup_key (version) so a new decision/version writes a fresh result
    -- marker instead of being suppressed by an older same-direction github-proxy comment marker.
    dedup_key = tostring(reached.proposal_id) .. "/comment/" .. tostring(reached.decision)
      .. "/" .. (tostring(reached.dedup_key):gsub(":", "-")),
    source_ref = M.normalize_source_ref(reached.source_ref),
  }
end

function M.build_loop_comment_request(repo, issue_number, unresolved, n)
  local marker = M.loop_marker(unresolved.proposal_id, n, unresolved.dedup_key)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop no-consensus loop: " .. tostring(n) .. "\n\n" .. marker,
    dedup_key = tostring(unresolved.proposal_id) .. "/comment/loop/" .. tostring(n)
      .. "/" .. (tostring(unresolved.dedup_key):gsub(":", "-")),
    source_ref = M.normalize_source_ref(unresolved.source_ref),
  }
end

function M.build_stuck_label_request(repo, issue_number, unresolved, n)
  return M.build_label_request(
    repo,
    issue_number,
    { stuck_label },
    { thinking_label },
    tostring(unresolved.proposal_id) .. "/label/stuck/" .. tostring(n)
      .. "/" .. (tostring(unresolved.dedup_key):gsub(":", "-")),
    unresolved.source_ref
  )
end

function M.build_stuck_comment_request(repo, issue_number, unresolved, n)
  local marker = M.stuck_marker(unresolved.proposal_id, n, unresolved.dedup_key)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop stuck: no consensus after " .. tostring(n) .. " attempts\n\n" .. marker,
    dedup_key = tostring(unresolved.proposal_id) .. "/comment/stuck/" .. tostring(n)
      .. "/" .. (tostring(unresolved.dedup_key):gsub(":", "-")),
    source_ref = M.normalize_source_ref(unresolved.source_ref),
  }
end

function M.is_supported_issue(payload)
  return type(payload) == "table"
    and payload.schema == "github-proxy.v1"
    and payload.type == "issue"
    and payload.repo ~= nil
    and payload.number ~= nil
    and payload.title ~= nil
    and payload.updated_at ~= nil
    and M.issue_ref_round_trips(payload.repo, payload.number)
    and has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_result(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_reached.v1"
    and (payload.decision == "approve" or payload.decision == "reject")
    and M.is_safe_consensus_result_ref(payload.proposal_id, payload.dedup_key)
    and is_bounded_string(payload.body, max_body_len)
    and has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_unresolved(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_unresolved.v1"
    and M.is_safe_consensus_result_ref(payload.proposal_id, payload.dedup_key)
    and payload.body == nil
    and payload.angle_results == nil
    and payload.decision == nil
    and has_bounded_source_ref(payload.source_ref)
end

return M
