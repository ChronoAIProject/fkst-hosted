local S = {}

function S.install(M, shared)
local github = shared.github

local gate_owned_gap_patterns = {
  "ci%s+green",
  "ci%s+status",
  "green%s+ci",
  "green%s+gate",
  "statuscheckrollup",
  "status%s+check",
  "merge%s+gate",
  "mergeability",
  "mergeable",
  "merge%s+state",
  "branch%s+protection",
  "head%-bound",
  "head%s+bound",
  "%f[%w]head%f[%W]",
  "same%s+head",
  "required%s+checks",
  "check%s+runs",
}

local implementation_gap_patterns = {
  "bug",
  "broken",
  "crash",
  "regression",
  "missing%s+test",
  "missing%s+guard",
  "missing%s+implementation",
  "missing%s+parser",
  "missing%s+validation",
  "incorrect",
  "wrong",
  "unsafe",
  "leak",
  "race",
  "idempot",
  "retry",
  "payload",
  "contract",
  "diff",
  "code",
  "logic",
}

local out_of_contract_gap_patterns = {
  "beyond%s+the%s+issue",
  "beyond%s+issue",
  "outside%s+the%s+issue",
  "outside%s+issue",
  "outside%s+the%s+stated%s+scope",
  "outside%s+stated%s+scope",
  "beyond%s+the%s+stated%s+scope",
  "beyond%s+stated%s+scope",
  "outside%s+the%s+acceptance%s+bound",
  "outside%s+acceptance%s+bound",
  "beyond%s+the%s+acceptance%s+bound",
  "beyond%s+acceptance%s+bound",
  "not%s+in%s+the%s+issue",
  "not%s+part%s+of%s+the%s+issue",
  "not%s+stated%s+in%s+the%s+issue",
  "not%s+an%s+issue%s+requirement",
  "unstated%s+requirement",
  "new%s+requirement",
  "spec%s+amendment",
  "spec%-amendment",
  "missing%s+pr%s+body%s+duplicate%s+evidence%s+analysis",
  "missing%s+pr%s+body%s+evidence",
  "missing%s+pull%s+request%s+body%s+evidence",
  "missing%s+pr%s+description%s+evidence",
  "missing%s+pull%s+request%s+description%s+evidence",
}

function M.is_gate_owned_review_gap(gap)
  local text = tostring(gap or ""):lower():gsub("[_%-%/]+", " "):gsub("%s+", " ")
  if text == "" then
    return false
  end
  local has_gate_fact = false
  for _, pattern in ipairs(gate_owned_gap_patterns) do
    if text:find(pattern) ~= nil then
      has_gate_fact = true
      break
    end
  end
  if not has_gate_fact then
    return false
  end
  for _, pattern in ipairs(implementation_gap_patterns) do
    if text:find(pattern) ~= nil then
      return false
    end
  end
  return true
end

function M.is_out_of_contract_review_gap(gap)
  local text = tostring(gap or ""):lower():gsub("[_%-%/]+", " "):gsub("%s+", " ")
  if text == "" then
    return false
  end
  for _, pattern in ipairs(out_of_contract_gap_patterns) do
    if text:find(pattern) ~= nil then
      return true
    end
  end
  return false
end

function M.is_ready_hand_off(hand_off, ready)
  if type(hand_off) ~= "table" or type(ready) ~= "table" then
    return false
  end
  return hand_off.kind == "own-state-marker"
    and hand_off.proposal_id == ready.proposal_id
    and hand_off.state == "ready"
    and hand_off.event_version == ready.dedup_key
    and M._is_bounded_string(hand_off.marker_version, M._max_dedup_len)
    and hand_off.stage_rank == M.stage_rank("ready")
    and M.is_safe_comment_id(hand_off.comment_id)
end

function M.is_safe_comment_id(value)
  local text = tostring(value or "")
  return text ~= "" and #text <= 80 and text:find("^[%w_%-]+$") ~= nil
end

function M.is_own_state_marker_hand_off(hand_off, expected)
  if type(hand_off) ~= "table" or type(expected) ~= "table" then
    return false
  end
  local state = tostring(expected.state or "")
  return hand_off.kind == "own-state-marker"
    and hand_off.proposal_id == expected.proposal_id
    and hand_off.state == state
    and hand_off.event_version == expected.event_version
    and hand_off.marker_version == expected.marker_version
    and hand_off.stage_rank == M.stage_rank(state)
    and (expected.effects == nil or hand_off.effects == expected.effects)
    and M.is_safe_comment_id(hand_off.comment_id)
end

local function state_marker_comment_verified(M, repo, hand_off)
  if type(hand_off) ~= "table" or not M.is_safe_comment_id(hand_off.comment_id) then
    return false, "missing-comment-id"
  end
  local ok_result, result = pcall(github().comment_get, repo, hand_off.comment_id, 30)
  if not ok_result or type(result) ~= "table" then
    return false, "comment-get-failed"
  end
  local ok, decoded = pcall(json.decode, result.stdout or "{}")
  if not ok or type(decoded) ~= "table" then
    return false, "comment-json-invalid"
  end
  local comment = {
    id = decoded.databaseId or decoded.database_id or decoded.id,
    body = decoded.body,
    author_login = M._comment_author_login(decoded),
    created_at = decoded.createdAt or decoded.created_at,
  }
  if comment.id ~= nil and tostring(comment.id) ~= tostring(hand_off.comment_id) then
    return false, "comment-id-mismatch"
  end
  if not M._is_trusted_comment(comment) then
    return false, "comment-author-untrusted"
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:state:v1.-%-%->"
  for marker in M._comment_body(comment):gmatch(marker_pattern) do
    local marker_proposal = marker:match('proposal="([^"]+)"')
    local marker_state = marker:match('state="([^"]+)"')
    local marker_version = marker:match('version="([^"]*)"')
    local marker_stage_rank = marker:match('stage_rank="([^"]+)"')
    if marker_proposal == hand_off.proposal_id
      and marker_state == hand_off.state
      and marker_version == hand_off.marker_version
      and tonumber(marker_stage_rank) == M.stage_rank(hand_off.state) then
      return true, "verified"
    end
  end
  return false, "state-marker-missing"
end

function M.verify_own_state_marker_hand_off(repo, hand_off, expected)
  if not M.is_own_state_marker_hand_off(hand_off, expected) then
    return false, "payload-mismatch"
  end
  return state_marker_comment_verified(M, repo, hand_off)
end

function M.verified_hand_off_state(repo, hand_off, expected)
  local ok, reason = M.verify_own_state_marker_hand_off(repo, hand_off, expected)
  if not ok then
    return nil, reason
  end
  return {
    state = expected.state,
    version = expected.event_version,
    stage_rank = M.stage_rank(expected.state),
  }, reason
end
end

return S
