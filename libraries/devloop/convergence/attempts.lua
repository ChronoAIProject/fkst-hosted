local S = {}
local transition_version = require("contract.transition_version")

function S.install(M, shared)
  local valid_round = shared.valid_round
  local max_attr_len = shared.max_attr_len
  local safe_attr = shared.safe_attr
  local attr = shared.attr

function M.timeout_attempt_marker(proposal_id, issue_version, state_name, round, source_ref)
  local n = valid_round(round)
  if n == nil or n <= 0 then
    error("github-devloop: invalid timeout attempt round")
  end
  local normalized = M.normalize_source_ref(source_ref)
  local lineage_version = transition_version.strip_suffixes(issue_version)
  return '<!-- fkst:github-devloop:timeout-attempt:v1 proposal="' .. safe_attr(proposal_id, M._max_key_len)
    .. '" version="' .. safe_attr(lineage_version, M._max_dedup_len)
    .. '" state="' .. safe_attr(state_name, max_attr_len)
    .. '" round="' .. tostring(n)
    .. '" dedup="' .. safe_attr("timeout-attempt:" .. tostring(lineage_version) .. "/" .. tostring(state_name) .. "/" .. tostring(n), M._max_dedup_len)
    .. '" source_ref_kind="' .. safe_attr(normalized.kind or "", max_attr_len)
    .. '" source_ref="' .. safe_attr(normalized.ref or "", M._max_key_len)
    .. '" -->'
end

function M.timeout_attempt_v2_marker(proposal_id, state_name, liveness_class_id, generation_key, round, source_ref)
  local n = valid_round(round)
  if n == nil or n <= 0 then
    error("github-devloop: invalid timeout attempt round")
  end
  local normalized = M.normalize_source_ref(source_ref)
  return '<!-- fkst:github-devloop:timeout-attempt:v2 proposal="' .. safe_attr(proposal_id, M._max_key_len)
    .. '" state="' .. safe_attr(state_name, max_attr_len)
    .. '" liveness_class_id="' .. safe_attr(liveness_class_id, max_attr_len)
    .. '" generation_key="' .. safe_attr(generation_key, M._max_dedup_len)
    .. '" round="' .. tostring(n)
    .. '" dedup="' .. safe_attr("timeout-attempt:v2:" .. tostring(state_name) .. "/" .. tostring(liveness_class_id) .. "/" .. tostring(generation_key) .. "/" .. tostring(n), M._max_dedup_len)
    .. '" source_ref_kind="' .. safe_attr(normalized.kind or "", max_attr_len)
    .. '" source_ref="' .. safe_attr(normalized.ref or "", M._max_key_len)
    .. '" -->'
end

function M.timeout_attempt_latest_marker(proposal_id, state_name, liveness_class_id, generation_key)
  return '<!-- fkst:github-devloop:timeout-attempt:latest:v1 proposal="' .. safe_attr(proposal_id, M._max_key_len)
    .. '" state="' .. safe_attr(state_name, max_attr_len)
    .. '" liveness_class_id="' .. safe_attr(liveness_class_id or "", max_attr_len)
    .. '" generation_key="' .. safe_attr(generation_key or "", M._max_dedup_len)
    .. '" -->'
end

function M.build_timeout_attempt_comment_request(target, proposal_id, state, row, source_ref, attempt)
  local normalized = M.normalize_source_ref(source_ref)
  local marker = M.timeout_attempt_marker(proposal_id, state.version, row.from_state, attempt, normalized)
  local latest_marker = M.timeout_attempt_latest_marker(proposal_id, row.from_state, "", transition_version.strip_suffixes(state.version))
  return M.build_entity_comment_request(target, "github-devloop timeout redrive attempt: "
    .. tostring(row.from_state)
    .. " "
    .. tostring(attempt)
    .. "\n\n"
    .. marker
    .. "\n"
    .. latest_marker
    .. "\n"
    .. "⟦AI:FKST⟧", M._dedup_key({
    "timeout-attempt",
    tostring(proposal_id),
    tostring(transition_version.strip_suffixes(state.version)),
    tostring(row.from_state),
    tostring(attempt),
  }), normalized, {
    replace_marker = latest_marker,
  })
end

function M.build_timeout_attempt_v2_comment_request(target, proposal_id, state, row, source_ref, attempt, generation_key)
  local normalized = M.normalize_source_ref(source_ref)
  local marker = M.timeout_attempt_v2_marker(proposal_id, row.from_state, row.liveness_class_id, generation_key, attempt, normalized)
  local latest_marker = M.timeout_attempt_latest_marker(proposal_id, row.from_state, row.liveness_class_id, generation_key)
  return M.build_entity_comment_request(target, "github-devloop timeout redrive attempt: "
    .. tostring(row.from_state)
    .. " "
    .. tostring(attempt)
    .. "\n\n"
    .. marker
    .. "\n"
    .. latest_marker
    .. "\n"
    .. "⟦AI:FKST⟧", M._dedup_key({
    "timeout-attempt:v2",
    tostring(proposal_id),
    tostring(row.from_state),
    tostring(row.liveness_class_id),
    tostring(generation_key),
    tostring(attempt),
  }), normalized, {
    replace_marker = latest_marker,
  })
end

function M.decompose_exhausted_marker(proposal_id, issue_version, round, source_ref)
  local n = valid_round(round)
  if n == nil or n <= 0 then
    error("github-devloop: invalid decompose exhausted round")
  end
  local normalized = M.normalize_source_ref(source_ref)
  local lineage_version = transition_version.strip_suffixes(issue_version)
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
    tostring(transition_version.strip_suffixes(state.version)),
    tostring(attempt),
  }), normalized)
end
function M.timeout_attempt_round(comments, proposal_id, issue_version, state_name)
  if type(comments) ~= "table" then
    return 0
  end
  local max_seen = 0
  local lineage_version = transition_version.strip_suffixes(issue_version)
  local marker_pattern = "<!%-%- fkst:github%-devloop:timeout%-attempt:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if attr(marker, "proposal") == tostring(proposal_id)
        and transition_version.strip_suffixes(attr(marker, "version")) == lineage_version
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

function M.timeout_attempt_v2_round(comments, proposal_id, row, generation_key)
  if type(comments) ~= "table" then
    return 0
  end
  local max_seen = 0
  local marker_pattern = "<!%-%- fkst:github%-devloop:timeout%-attempt:v2.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if attr(marker, "proposal") == tostring(proposal_id)
        and attr(marker, "state") == tostring(row and row.from_state)
        and attr(marker, "liveness_class_id") == tostring(row and row.liveness_class_id)
        and attr(marker, "generation_key") == tostring(generation_key) then
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
  local lineage_version = transition_version.strip_suffixes(issue_version)
  local marker_pattern = "<!%-%- fkst:github%-devloop:decompose%-exhausted:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      if attr(marker, "proposal") == tostring(proposal_id)
        and transition_version.strip_suffixes(attr(marker, "version")) == lineage_version then
        return true
      end
    end
  end
  return false
end
end

return S
