local S = {}

local max_digest_len = 64
local max_attr_len = 240
local max_question_len = 2000
local max_round = 100000

local function normalize_text(value)
  return tostring(value or ""):gsub("%s+", " "):gsub("^%s+", ""):gsub("%s+$", "")
end

local function digest(M, prefix, value)
  local text = tostring(value or "")
  return tostring(prefix) .. "-" .. #text .. "-" .. M._decimal_checksum(text)
end

local function safe_attr(value, limit)
  local text = tostring(value or ""):gsub("%c", " "):gsub('"', "'"):gsub("[<>]", ""):gsub("%s+", " ")
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  local cap = limit or max_attr_len
  if #text > cap then
    text = text:sub(1, cap)
  end
  return text
end

local function valid_round(value)
  local n = tonumber(value)
  if n == nil or n < 0 or n ~= math.floor(n) or n > max_round then
    return nil
  end
  return n
end

local function sorted_angle_items(angle_digests)
  local items = {}
  if type(angle_digests) ~= "table" then
    return items
  end
  for _, item in ipairs(angle_digests) do
    if type(item) == "table" then
      table.insert(items, {
        angle = safe_attr(item.angle or "unknown", max_attr_len),
        verdict = safe_attr(item.verdict or "invalid", max_attr_len),
        digest = safe_attr(item.digest or "", max_attr_len),
      })
    end
  end
  table.sort(items, function(a, b)
    if a.angle == b.angle then
      return a.verdict .. ":" .. a.digest < b.verdict .. ":" .. b.digest
    end
    return a.angle < b.angle
  end)
  return items
end

local function attr(marker, name)
  return marker:match(name .. '="([^"]*)"')
end

local function is_digest(value)
  return type(value) == "string" and value ~= "" and #value <= max_digest_len and value:find("%c") == nil
end

local function is_bounded_attr(M, value, limit)
  return M._is_bounded_string(value, limit or max_attr_len) and value:find("%c") == nil
end

local function converge_record_map(M, comments, kind, matches)
  local records_by_round = {}
  if type(comments) ~= "table" then
    return {}
  end

  local marker_pattern = "<!%-%- fkst:github%-devloop:" .. kind .. ":v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local round = valid_round(attr(marker, "round"))
      local question = attr(marker, "question")
      local verdicts = attr(marker, "verdicts")
      local dedup = attr(marker, "dedup")
      if round ~= nil
        and matches(marker)
        and is_digest(question)
        and is_digest(verdicts)
        and is_bounded_attr(M, dedup, M._max_dedup_len) then
        records_by_round[round] = {
          round = round,
          question = question,
          verdicts = verdicts,
          dedup = dedup,
        }
      end
    end
  end

  local facts = {}
  for _, record in pairs(records_by_round) do
    table.insert(facts, record)
  end
  table.sort(facts, function(a, b)
    return a.round < b.round
  end)
  return facts
end

function S.install(M)
function M.source_ref_digest(source_ref)
  if type(source_ref) ~= "table" then
    return digest(M, "sr", "")
  end
  return digest(M, "sr", tostring(source_ref.kind or "") .. "\n" .. tostring(source_ref.ref or ""))
end

function M.converge_question_digest(narrowed_question)
  local normalized = normalize_text(narrowed_question)
  if #normalized > max_question_len then
    normalized = normalized:sub(1, max_question_len)
  end
  return digest(M, "q", normalized)
end

function M.converge_verdicts_digest(angle_digests)
  local parts = {}
  for _, item in ipairs(sorted_angle_items(angle_digests)) do
    table.insert(parts, item.angle .. "=" .. item.verdict)
  end
  return digest(M, "v", table.concat(parts, "\n"))
end

function M.converge_angles_digest(angle_digests)
  local parts = {}
  for _, item in ipairs(sorted_angle_items(angle_digests)) do
    table.insert(parts, item.angle .. "=" .. item.verdict .. ":" .. item.digest)
  end
  return digest(M, "a", table.concat(parts, "\n"))
end

function M.converge_base_version(consensus_dedup)
  return (tostring(consensus_dedup or ""):gsub("/loop/%d+$", ""))
end

function M.build_devloop_reconcile_payload(unresolved, round, base_version)
  return {
    schema = "github-devloop.reconcile.v1",
    proposal_id = unresolved.proposal_id,
    dedup_key = "reconcile:" .. tostring(base_version) .. "/loop/" .. tostring(round),
    round = round,
    base_version = base_version,
    source_ref = {
      kind = unresolved.source_ref.kind,
      ref = unresolved.source_ref.ref,
    },
  }
end

function M.is_supported_reconcile(payload)
  if type(payload) ~= "table" then
    return false
  end
  local dedup_tail = tostring(payload.dedup_key or ""):match("^reconcile:(.+)$")
  -- The reconcile dedup carries the consensus base version (`reconcile:consensus:<path>/loop/N`).
  -- Strip the inherent `consensus:` prefix before path-checking, mirroring
  -- is_safe_consensus_result_ref, so the legitimate colon is not rejected.
  local inner_dedup = dedup_tail ~= nil and (dedup_tail:match("^consensus:(.+)$") or dedup_tail) or nil
  -- parse_proposal_id returns TWO values; do NOT wrap it in `and ... or` (that truncates
  -- the multi-return so issue_number would always be nil).
  local repo, issue_number = M.parse_proposal_id(payload.proposal_id)
  return payload.schema == "github-devloop.reconcile.v1"
    and repo ~= nil
    and issue_number ~= nil
    and M._is_path_safe_key(payload.proposal_id, M._max_key_len)
    and M._is_bounded_string(payload.dedup_key, M._max_dedup_len)
    and M._is_bounded_string(payload.base_version, M._max_dedup_len)
    and tostring(payload.dedup_key) == "reconcile:" .. tostring(payload.base_version) .. "/loop/" .. tostring(payload.round)
    and inner_dedup ~= nil
    and M._is_path_safe_key(inner_dedup, M._max_dedup_len)
    and M._has_bounded_source_ref(payload.source_ref)
    and valid_round(payload.round) ~= nil
end

function M.converge_round_marker(proposal_id, base_version, source_ref_digest, round, consensus_dedup, narrowed_question, angle_digests)
  local n = valid_round(round)
  if n == nil then
    error("github-devloop: invalid converge round")
  end
  return '<!-- fkst:github-devloop:converge-round:v1 proposal="' .. safe_attr(proposal_id, M._max_key_len)
    .. '" version="' .. safe_attr(base_version, M._max_dedup_len)
    .. '" source_ref="' .. safe_attr(source_ref_digest, max_digest_len)
    .. '" round="' .. tostring(n)
    .. '" dedup="' .. safe_attr(consensus_dedup, M._max_dedup_len)
    .. '" question="' .. M.converge_question_digest(narrowed_question)
    .. '" verdicts="' .. M.converge_verdicts_digest(angle_digests)
    .. '" angles="' .. M.converge_angles_digest(angle_digests)
    .. '" -->'
end

function M.reconcile_state_version(base_version, round)
  return tostring(base_version) .. "/loop/" .. tostring(round)
end

function M.reconcile_marker(proposal_id, base_version, round, action)
  local n = valid_round(round)
  if n == nil then
    error("github-devloop: invalid reconcile round")
  end
  if action ~= "drop" and action ~= "re-design" and action ~= "re-cluster" then
    error("github-devloop: invalid reconcile action")
  end
  return '<!-- fkst:github-devloop:reconcile:v1 proposal="' .. safe_attr(proposal_id, M._max_key_len)
    .. '" version="' .. safe_attr(M.reconcile_state_version(base_version, n), M._max_dedup_len)
    .. '" round="' .. tostring(n)
    .. '" action="' .. safe_attr(action, max_attr_len)
    .. '" dedup="' .. safe_attr("reconcile:" .. tostring(base_version) .. "/loop/" .. tostring(n), M._max_dedup_len)
    .. '" -->'
end

function M.review_converge_round_marker(review_proposal_id, issue_proposal_id, issue_version, head_sha, source_ref_digest, round, consensus_dedup, narrowed_question, angle_digests)
  local n = valid_round(round)
  if n == nil then
    error("github-devloop: invalid review converge round")
  end
  return '<!-- fkst:github-devloop:review-converge-round:v1 proposal="' .. safe_attr(review_proposal_id, M._max_key_len)
    .. '" issue_proposal="' .. safe_attr(issue_proposal_id, M._max_key_len)
    .. '" version="' .. safe_attr(issue_version, M._max_dedup_len)
    .. '" head_sha="' .. safe_attr(head_sha, max_attr_len)
    .. '" source_ref="' .. safe_attr(source_ref_digest, max_digest_len)
    .. '" round="' .. tostring(n)
    .. '" dedup="' .. safe_attr(consensus_dedup, M._max_dedup_len)
    .. '" question="' .. M.converge_question_digest(narrowed_question)
    .. '" verdicts="' .. M.converge_verdicts_digest(angle_digests)
    .. '" angles="' .. M.converge_angles_digest(angle_digests)
    .. '" -->'
end

function M.converge_round_facts(comments, proposal_id, base_version, source_ref_digest)
  local matches = function(marker)
    return attr(marker, "proposal") == tostring(proposal_id)
      and attr(marker, "version") == tostring(base_version)
      and attr(marker, "source_ref") == tostring(source_ref_digest)
  end
  return converge_record_map(M, comments, "converge%-round", matches)
end

function M.review_converge_round_facts(comments, review_proposal_id, issue_proposal_id, issue_version, head_sha, source_ref_digest)
  local matches = function(marker)
    return attr(marker, "proposal") == tostring(review_proposal_id)
      and attr(marker, "issue_proposal") == tostring(issue_proposal_id)
      and attr(marker, "version") == tostring(issue_version)
      and attr(marker, "head_sha") == tostring(head_sha)
      and attr(marker, "source_ref") == tostring(source_ref_digest)
  end
  return converge_record_map(M, comments, "review%-converge%-round", matches)
end

function M.max_converge_round(facts)
  local max_seen = 0
  if type(facts) ~= "table" then
    return max_seen
  end
  for _, fact in ipairs(facts) do
    local round = valid_round(type(fact) == "table" and fact.round or nil)
    if round ~= nil and round > max_seen then
      max_seen = round
    end
  end
  return max_seen
end

function M.has_converge_round_marker(comments, proposal_id, base_version, source_ref_digest, round)
  local n = valid_round(round)
  if n == nil then
    return false
  end
  for _, fact in ipairs(M.converge_round_facts(comments, proposal_id, base_version, source_ref_digest)) do
    if fact.round == n then
      return true
    end
  end
  return false
end

function M.has_reconcile_marker(comments, proposal_id, base_version, round)
  local n = valid_round(round)
  if n == nil or type(comments) ~= "table" then
    return false
  end
  local version = M.reconcile_state_version(base_version, n)
  local marker_pattern = "<!%-%- fkst:github%-devloop:reconcile:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if attr(marker, "proposal") == tostring(proposal_id)
        and attr(marker, "version") == version
        and valid_round(attr(marker, "round")) == n then
        return true
      end
    end
  end
  return false
end

function M.has_review_converge_round_marker(comments, review_proposal_id, issue_proposal_id, issue_version, head_sha, source_ref_digest, round)
  local n = valid_round(round)
  if n == nil then
    return false
  end
  for _, fact in ipairs(M.review_converge_round_facts(comments, review_proposal_id, issue_proposal_id, issue_version, head_sha, source_ref_digest)) do
    if fact.round == n then
      return true
    end
  end
  return false
end

function M.is_true_stall(facts, current_round)
  local round = valid_round(current_round)
  if round == nil or round < 3 or type(facts) ~= "table" then
    return false
  end

  local by_round = {}
  for _, fact in ipairs(facts) do
    if type(fact) == "table" then
      local fact_round = valid_round(fact.round)
      if fact_round ~= nil then
        by_round[fact_round] = fact
      end
    end
  end

  local current = by_round[round]
  local previous = by_round[round - 1]
  local before_previous = by_round[round - 2]
  if current == nil or previous == nil or before_previous == nil then
    return false
  end

  return current.question == previous.question
    and previous.question == before_previous.question
    and current.verdicts == previous.verdicts
    and previous.verdicts == before_previous.verdicts
end
end

return S
