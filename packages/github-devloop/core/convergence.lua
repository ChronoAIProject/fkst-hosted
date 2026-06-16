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

local function decode_attr(value)
  if type(value) ~= "string" or value == "" then
    return nil
  end
  if value:find("%c") ~= nil or value:find("[<>]") ~= nil or value:find('"', 1, true) ~= nil then
    return nil
  end
  return value
end

local sorted_angle_items

local function encode_component(value)
  return tostring(value or "")
    :gsub("%%", "%%25")
    :gsub("|", "%%7C")
    :gsub(";", "%%3B")
end

local function decode_component(value)
  return tostring(value or "")
    :gsub("%%3[Bb]", ";")
    :gsub("%%7[Cc]", "|")
    :gsub("%%25", "%%")
end

local function encode_angle_replay(angle_digests)
  local parts = {}
  for _, item in ipairs(sorted_angle_items(angle_digests)) do
    table.insert(parts, encode_component(item.angle) .. "|" .. encode_component(item.verdict) .. "|" .. encode_component(item.digest))
  end
  return safe_attr(table.concat(parts, ";"), 1000)
end

local function decode_angle_replay(value)
  local text = decode_attr(value)
  if text == nil then
    return nil
  end
  local items = {}
  for part in text:gmatch("[^;]+") do
    local angle, verdict, item_digest = part:match("^([^|]+)|([^|]+)|(.*)$")
    if angle == nil or verdict == nil or item_digest == nil then
      return nil
    end
    table.insert(items, {
      angle = decode_component(angle),
      verdict = decode_component(verdict),
      digest = decode_component(item_digest),
    })
  end
  if #items == 0 then
    return nil
  end
  return items
end

local function valid_round(value)
  local n = tonumber(value)
  if n == nil or n < 0 or n ~= math.floor(n) or n > max_round then
    return nil
  end
  return n
end

function sorted_angle_items(angle_digests)
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
      local narrowed_question = decode_attr(attr(marker, "narrowed_question"))
      local angle_digests = decode_angle_replay(attr(marker, "angle_digests"))
      local version = attr(marker, "version")
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
          version = version,
          narrowed_question = narrowed_question,
          angle_digests = angle_digests,
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

function M.converge_proposal_base_dedup(consensus_dedup)
  local base_version = M.converge_base_version(consensus_dedup)
  return base_version:match("^consensus:(.+)$") or base_version
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

function M.build_devloop_review_reconcile_payload(unresolved, round, issue_proposal_id, issue_version, head_sha)
  return {
    schema = "github-devloop.review-reconcile.v1",
    proposal_id = issue_proposal_id,
    review_proposal_id = unresolved.proposal_id,
    issue_version = issue_version,
    head_sha = head_sha,
    round = round,
    dedup_key = "review-reconcile:" .. tostring(issue_version) .. "/review-loop/" .. tostring(round),
    source_ref = {
      kind = unresolved.source_ref.kind,
      ref = unresolved.source_ref.ref,
    },
  }
end

function M.build_devloop_fix_reconcile_payload(reject_ctx, issue_version)
  return {
    schema = "github-devloop.fix-reconcile.v1",
    proposal_id = reject_ctx.proposal_id,
    review_proposal_id = reject_ctx.review_proposal_id,
    review_dedup_key = reject_ctx.review_dedup_key,
    issue_version = issue_version,
    head_sha = reject_ctx.reviewed_head_sha,
    round = M.version_fix_round(issue_version),
    pr_number = reject_ctx.pr_number,
    dedup_key = "fix-reconcile:" .. tostring(issue_version),
    source_ref = {
      kind = reject_ctx.source_ref.kind,
      ref = reject_ctx.source_ref.ref,
    },
  }
end

function M.build_devloop_timeout_reconcile_payload(row, state, proposal_id, source_ref, attempt)
  return {
    schema = "github-devloop.timeout-reconcile.v1",
    proposal_id = proposal_id,
    state = row.from_state,
    issue_version = state.version,
    round = attempt,
    dedup_key = "timeout-reconcile:" .. tostring(state.version) .. "/timeout-reconcile/" .. tostring(row.from_state) .. "/" .. tostring(attempt),
    source_ref = {
      kind = source_ref.kind,
      ref = source_ref.ref,
    },
  }
end

function M.timeout_attempt_marker(proposal_id, issue_version, state_name, round, source_ref)
  local n = valid_round(round)
  if n == nil or n <= 0 then
    error("github-devloop: invalid timeout attempt round")
  end
  local normalized = M.normalize_source_ref(source_ref)
  local lineage_version = M.strip_transition_version_suffixes(issue_version)
  return '<!-- fkst:github-devloop:timeout-attempt:v1 proposal="' .. safe_attr(proposal_id, M._max_key_len)
    .. '" version="' .. safe_attr(lineage_version, M._max_dedup_len)
    .. '" state="' .. safe_attr(state_name, max_attr_len)
    .. '" round="' .. tostring(n)
    .. '" dedup="' .. safe_attr("timeout-attempt:" .. tostring(lineage_version) .. "/" .. tostring(state_name) .. "/" .. tostring(n), M._max_dedup_len)
    .. '" source_ref_kind="' .. safe_attr(normalized.kind or "", max_attr_len)
    .. '" source_ref="' .. safe_attr(normalized.ref or "", M._max_key_len)
    .. '" -->'
end

function M.build_timeout_attempt_comment_request(target, proposal_id, state, row, source_ref, attempt)
  local normalized = M.normalize_source_ref(source_ref)
  local marker = M.timeout_attempt_marker(proposal_id, state.version, row.from_state, attempt, normalized)
  return M.build_entity_comment_request(target, "github-devloop timeout redrive attempt: "
    .. tostring(row.from_state)
    .. " "
    .. tostring(attempt)
    .. "\n\n"
    .. marker
    .. "\n"
    .. "⟦AI:FKST⟧", M._dedup_key({
    "timeout-attempt",
    tostring(proposal_id),
    tostring(M.strip_transition_version_suffixes(state.version)),
    tostring(row.from_state),
    tostring(attempt),
  }), normalized)
end

function M.decompose_exhausted_marker(proposal_id, issue_version, round, source_ref)
  local n = valid_round(round)
  if n == nil or n <= 0 then
    error("github-devloop: invalid decompose exhausted round")
  end
  local normalized = M.normalize_source_ref(source_ref)
  local lineage_version = M.strip_transition_version_suffixes(issue_version)
  return '<!-- fkst:github-devloop:decompose-exhausted:v1 proposal="' .. safe_attr(proposal_id, M._max_key_len)
    .. '" version="' .. safe_attr(lineage_version, M._max_dedup_len)
    .. '" round="' .. tostring(n)
    .. '" reason_class="decompose-output-obligation-timeout"'
    .. '" source_ref_kind="' .. safe_attr(normalized.kind or "", max_attr_len)
    .. '" source_ref="' .. safe_attr(normalized.ref or "", M._max_key_len)
    .. '" -->'
end

function M.build_decompose_exhausted_comment_request(target, proposal_id, state, source_ref, attempt)
  local normalized = M.normalize_source_ref(source_ref)
  local marker = M.decompose_exhausted_marker(proposal_id, state.version, attempt, normalized)
  return M.build_entity_comment_request(target, "github-devloop decompose output obligation exhausted\n\n"
    .. "Structured WHY:\n"
    .. "reason_class=decompose-output-obligation-timeout\n"
    .. "from_state=blocked\n"
    .. "from_version=" .. tostring(state.version) .. "\n"
    .. "attempt=" .. tostring(attempt) .. "\n\n"
    .. marker
    .. "\n"
    .. "⟦AI:FKST⟧", M._dedup_key({
    "decompose-exhausted",
    tostring(proposal_id),
    tostring(M.strip_transition_version_suffixes(state.version)),
    tostring(attempt),
  }), normalized)
end

function M.review_reconcile_state_version(issue_version, round)
  return tostring(issue_version) .. "/review-loop/" .. tostring(round)
end

function M.reconcile_terminal_state_version(current_version, round)
  local n = valid_round(round)
  if n == nil then
    error("github-devloop: invalid reconcile round")
  end
  local next_n = M.version_loop_round(current_version) + 1
  if n > next_n then
    next_n = n
  end
  return tostring(current_version) .. "/loop/" .. tostring(next_n)
end

function M.review_reconcile_terminal_state_version(current_version, round)
  local n = valid_round(round)
  if n == nil then
    error("github-devloop: invalid review reconcile round")
  end
  local next_n = M.version_review_loop_round(current_version) + 1
  if n > next_n then
    next_n = n
  end
  return tostring(current_version) .. "/review-loop/" .. tostring(next_n)
end

function M.fix_reconcile_state_version(issue_version)
  return tostring(issue_version)
end

function M.timeout_reconcile_state_version(issue_version, state_name, round)
  return tostring(issue_version) .. "/timeout-reconcile/" .. tostring(state_name) .. "/" .. tostring(round)
end

function M.is_supported_review_reconcile(payload)
  if type(payload) ~= "table" then
    return false
  end
  local repo, issue_number = M.parse_proposal_id(payload.proposal_id)
  return payload.schema == "github-devloop.review-reconcile.v1"
    and repo ~= nil
    and issue_number ~= nil
    and M._is_path_safe_key(payload.proposal_id, M._max_key_len)
    and M._is_path_safe_key(payload.review_proposal_id, M._max_key_len)
    and M._is_bounded_string(payload.issue_version, M._max_dedup_len)
    and M._is_git_sha(payload.head_sha)
    and valid_round(payload.round) ~= nil
    and M._is_bounded_string(payload.dedup_key, M._max_dedup_len)
    and tostring(payload.dedup_key) == "review-reconcile:" .. tostring(payload.issue_version) .. "/review-loop/" .. tostring(payload.round)
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_fix_reconcile(payload)
  if type(payload) ~= "table" then
    return false
  end
  local repo, issue_number = M.parse_proposal_id(payload.proposal_id)
  return payload.schema == "github-devloop.fix-reconcile.v1"
    and repo ~= nil
    and issue_number ~= nil
    and M._is_path_safe_key(payload.proposal_id, M._max_key_len)
    and M._is_path_safe_key(payload.review_proposal_id, M._max_key_len)
    and M._is_bounded_string(payload.review_dedup_key, M._max_dedup_len)
    and M._is_bounded_string(payload.issue_version, M._max_dedup_len)
    and M._is_git_sha(payload.head_sha)
    and valid_round(payload.round) ~= nil
    and tonumber(payload.round) == M.version_fix_round(payload.issue_version)
    and M._is_positive_pr_number(payload.pr_number)
    and M._is_bounded_string(payload.dedup_key, M._max_dedup_len)
    and tostring(payload.dedup_key) == "fix-reconcile:" .. tostring(payload.issue_version)
    and M._has_bounded_source_ref(payload.source_ref)
end

function M.is_supported_timeout_reconcile(payload)
  if type(payload) ~= "table" then
    return false
  end
  local repo, issue_number = M.parse_proposal_id(payload.proposal_id)
  local row = M.restart_transition_row(payload.state)
  return payload.schema == "github-devloop.timeout-reconcile.v1"
    and repo ~= nil
    and issue_number ~= nil
    and row ~= nil
    and row.terminal == false
    and M._is_path_safe_key(payload.proposal_id, M._max_key_len)
    and M._is_bounded_string(payload.issue_version, M._max_dedup_len)
    and valid_round(payload.round) ~= nil
    and M._is_bounded_string(payload.dedup_key, M._max_dedup_len)
    and tostring(payload.dedup_key) == "timeout-reconcile:" .. tostring(payload.issue_version) .. "/timeout-reconcile/" .. tostring(payload.state) .. "/" .. tostring(payload.round)
    and M._has_bounded_source_ref(payload.source_ref)
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
    .. '" narrowed_question="' .. safe_attr(narrowed_question, max_question_len)
    .. '" angle_digests="' .. encode_angle_replay(angle_digests)
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

function M.review_reconcile_marker(issue_proposal_id, issue_version, round, action)
  local n = valid_round(round)
  if n == nil then
    error("github-devloop: invalid review reconcile round")
  end
  if action ~= "drop" and action ~= "re-design" and action ~= "re-cluster" then
    error("github-devloop: invalid review reconcile action")
  end
  return '<!-- fkst:github-devloop:review-reconcile:v1 proposal="' .. safe_attr(issue_proposal_id, M._max_key_len)
    .. '" version="' .. safe_attr(M.review_reconcile_state_version(issue_version, n), M._max_dedup_len)
    .. '" round="' .. tostring(n)
    .. '" action="' .. safe_attr(action, max_attr_len)
    .. '" dedup="' .. safe_attr("review-reconcile:" .. tostring(issue_version) .. "/review-loop/" .. tostring(n), M._max_dedup_len)
    .. '" -->'
end

function M.fix_reconcile_marker(proposal_id, issue_version, action)
  local n = valid_round(M.version_fix_round(issue_version))
  if n == nil then
    error("github-devloop: invalid fix reconcile round")
  end
  if action ~= "drop" and action ~= "re-design" and action ~= "re-cluster" then
    error("github-devloop: invalid fix reconcile action")
  end
  return '<!-- fkst:github-devloop:fix-reconcile:v1 proposal="' .. safe_attr(proposal_id, M._max_key_len)
    .. '" version="' .. safe_attr(issue_version, M._max_dedup_len)
    .. '" round="' .. tostring(n)
    .. '" action="' .. safe_attr(action, max_attr_len)
    .. '" dedup="' .. safe_attr("fix-reconcile:" .. tostring(issue_version), M._max_dedup_len)
    .. '" -->'
end

function M.timeout_reconcile_marker(proposal_id, issue_version, state_name, round, action, fields)
  local n = valid_round(round)
  if n == nil then
    error("github-devloop: invalid timeout reconcile round")
  end
  if action ~= "drop" then
    error("github-devloop: invalid timeout reconcile action")
  end
  local why = fields or {}
  local source_ref = type(why.source_ref) == "table" and why.source_ref or {}
  local marker_version = why.terminal_version or M.timeout_reconcile_state_version(issue_version, state_name, n)
  return '<!-- fkst:github-devloop:timeout-reconcile:v1 proposal="' .. safe_attr(proposal_id, M._max_key_len)
    .. '" version="' .. safe_attr(marker_version, M._max_dedup_len)
    .. '" state="' .. safe_attr(state_name, max_attr_len)
    .. '" round="' .. tostring(n)
    .. '" action="' .. safe_attr(action, max_attr_len)
    .. '" dedup="' .. safe_attr("timeout-reconcile:" .. tostring(issue_version) .. "/timeout-reconcile/" .. tostring(state_name) .. "/" .. tostring(n), M._max_dedup_len)
    .. '" from_state="' .. safe_attr(why.from_state or state_name, max_attr_len)
    .. '" from_version="' .. safe_attr(why.from_version or issue_version, M._max_dedup_len)
    .. '" age_minutes="' .. safe_attr(why.age_minutes or "", max_attr_len)
    .. '" budget_minutes="' .. safe_attr(why.budget_minutes or "", max_attr_len)
    .. '" attempt="' .. safe_attr(why.attempt or n, max_attr_len)
    .. '" attempt_limit="' .. safe_attr(why.attempt_limit or "", max_attr_len)
    .. '" driving_queue="' .. safe_attr(why.driving_queue or "", max_attr_len)
    .. '" reason_class="' .. safe_attr(why.reason_class or "state-output-obligation-timeout", max_attr_len)
    .. '" source_ref_kind="' .. safe_attr(source_ref.kind or "", max_attr_len)
    .. '" source_ref="' .. safe_attr(source_ref.ref or "", M._max_key_len)
    .. '" -->'
end

function M.review_converge_round_marker(review_proposal_id, issue_proposal_id, issue_version, head_sha, source_ref_digest, round, consensus_dedup, narrowed_question, angle_digests)
  local n = valid_round(round)
  if n == nil then
    error("github-devloop: invalid review converge round")
  end
  local heartbeat_version = M.liveness_heartbeat_version(issue_version, M.liveness_signal_producer_contract("review-converge-round"))
  return '<!-- fkst:github-devloop:review-converge-round:v1 proposal="' .. safe_attr(review_proposal_id, M._max_key_len)
    .. '" issue_proposal="' .. safe_attr(issue_proposal_id, M._max_key_len)
    .. '" version="' .. safe_attr(heartbeat_version, M._max_dedup_len)
    .. '" head_sha="' .. safe_attr(head_sha, max_attr_len)
    .. '" source_ref="' .. safe_attr(source_ref_digest, max_digest_len)
    .. '" round="' .. tostring(n)
    .. '" dedup="' .. safe_attr(consensus_dedup, M._max_dedup_len)
    .. '" question="' .. M.converge_question_digest(narrowed_question)
    .. '" verdicts="' .. M.converge_verdicts_digest(angle_digests)
    .. '" angles="' .. M.converge_angles_digest(angle_digests)
    .. '" narrowed_question="' .. safe_attr(narrowed_question, max_question_len)
    .. '" angle_digests="' .. encode_angle_replay(angle_digests)
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

function M.converge_round_facts_for_source(comments, proposal_id, source_ref_digest)
  local matches = function(marker)
    return attr(marker, "proposal") == tostring(proposal_id)
      and attr(marker, "source_ref") == tostring(source_ref_digest)
  end
  return converge_record_map(M, comments, "converge%-round", matches)
end

function M.converge_round_facts_for_proposal(comments, proposal_id)
  local matches = function(marker)
    return attr(marker, "proposal") == tostring(proposal_id)
  end
  return converge_record_map(M, comments, "converge%-round", matches)
end

function M.converge_round_facts_for_proposal_boundary(comments, proposal_id, narrowed_question, angle_digests)
  local question = M.converge_question_digest(narrowed_question)
  local verdicts = M.converge_verdicts_digest(angle_digests)
  local matches = function(marker)
    return attr(marker, "proposal") == tostring(proposal_id)
      and attr(marker, "question") == question
      and attr(marker, "verdicts") == verdicts
  end
  return converge_record_map(M, comments, "converge%-round", matches)
end

function M.review_converge_round_facts(comments, review_proposal_id, issue_proposal_id, issue_version, head_sha, source_ref_digest)
  local heartbeat_version = M.liveness_heartbeat_version(issue_version, M.liveness_signal_producer_contract("review-converge-round"))
  local matches = function(marker)
    return attr(marker, "proposal") == tostring(review_proposal_id)
      and attr(marker, "issue_proposal") == tostring(issue_proposal_id)
      and attr(marker, "version") == tostring(heartbeat_version)
      and attr(marker, "head_sha") == tostring(head_sha)
      and attr(marker, "source_ref") == tostring(source_ref_digest)
  end
  return converge_record_map(M, comments, "review%-converge%-round", matches)
end

function M.converge_budget_round(comments, proposal_id)
  return M.max_converge_round(M.converge_round_facts_for_proposal(comments, proposal_id))
end

function M.converge_boundary_budget_round(comments, proposal_id, narrowed_question, angle_digests)
  return M.max_converge_round(M.converge_round_facts_for_proposal_boundary(comments, proposal_id, narrowed_question, angle_digests))
end

function M.review_converge_budget_round(comments, review_proposal_id, issue_proposal_id)
  local matches = function(marker)
    return attr(marker, "proposal") == tostring(review_proposal_id)
      and attr(marker, "issue_proposal") == tostring(issue_proposal_id)
  end
  return M.max_converge_round(converge_record_map(M, comments, "review%-converge%-round", matches))
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

function M.has_review_reconcile_marker(comments, issue_proposal_id, issue_version, round)
  local n = valid_round(round)
  if n == nil or type(comments) ~= "table" then
    return false
  end
  local version = M.review_reconcile_state_version(issue_version, n)
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-reconcile:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if attr(marker, "proposal") == tostring(issue_proposal_id)
        and attr(marker, "version") == version
        and valid_round(attr(marker, "round")) == n then
        return true
      end
    end
  end
  return false
end

function M.has_fix_reconcile_marker(comments, proposal_id, issue_version)
  local n = valid_round(M.version_fix_round(issue_version))
  if n == nil or type(comments) ~= "table" then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:fix%-reconcile:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if attr(marker, "proposal") == tostring(proposal_id)
        and attr(marker, "version") == tostring(issue_version)
        and valid_round(attr(marker, "round")) == n then
        return true
      end
    end
  end
  return false
end

function M.has_timeout_reconcile_marker(comments, proposal_id, issue_version, state_name, round)
  local n = valid_round(round)
  if n == nil or type(comments) ~= "table" then
    return false
  end
  local version = M.timeout_reconcile_state_version(issue_version, state_name, n)
  local marker_pattern = "<!%-%- fkst:github%-devloop:timeout%-reconcile:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if attr(marker, "proposal") == tostring(proposal_id)
        and attr(marker, "version") == version
        and attr(marker, "state") == tostring(state_name)
        and valid_round(attr(marker, "round")) == n then
        return true
      end
    end
  end
  return false
end

function M.timeout_attempt_round(comments, proposal_id, issue_version, state_name)
  if type(comments) ~= "table" then
    return 0
  end
  local max_seen = 0
  local lineage_version = M.strip_transition_version_suffixes(issue_version)
  local marker_pattern = "<!%-%- fkst:github%-devloop:timeout%-attempt:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if attr(marker, "proposal") == tostring(proposal_id)
        and M.strip_transition_version_suffixes(attr(marker, "version")) == lineage_version
        and attr(marker, "state") == tostring(state_name) then
        local round = valid_round(attr(marker, "round"))
        if round ~= nil and round > max_seen then
          max_seen = round
        end
      end
    end
  end
  return max_seen
end

function M.has_decompose_exhausted_marker(comments, proposal_id, issue_version)
  if type(comments) ~= "table" then
    return false
  end
  local lineage_version = M.strip_transition_version_suffixes(issue_version)
  local marker_pattern = "<!%-%- fkst:github%-devloop:decompose%-exhausted:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if attr(marker, "proposal") == tostring(proposal_id)
        and M.strip_transition_version_suffixes(attr(marker, "version")) == lineage_version then
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
