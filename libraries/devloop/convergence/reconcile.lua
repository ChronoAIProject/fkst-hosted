local S = {}
local replay_fields = require("devloop.replay_fields")
local forge_validators = require("devloop.forge_validators")

function S.install(M, shared)
  local source_refs = shared.source_refs
  local valid_round = shared.valid_round
  local max_attr_len = shared.max_attr_len
  local safe_attr = shared.safe_attr
  local attr = shared.attr

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
    and source_refs.has_bounded_source_ref(payload.source_ref, M._max_key_len)
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

function M.timeout_reconcile_reason_body(fields)
  local source_ref = type(fields.source_ref) == "table" and fields.source_ref or {}
  return "reason_class=" .. tostring(fields.reason_class or "state-output-obligation-timeout")
    .. "\nfrom_state=" .. tostring(fields.from_state or "")
    .. "\nfrom_version=" .. tostring(fields.from_version or "")
    .. "\nage_minutes=" .. tostring(fields.age_minutes or "")
    .. "\nbudget_minutes=" .. tostring(fields.budget_minutes or "")
    .. "\nattempt=" .. tostring(fields.attempt or "")
    .. "\nattempt_limit=" .. tostring(fields.attempt_limit or "")
    .. "\ndriving_queue=" .. tostring(fields.driving_queue or "")
    .. "\nsource_ref.kind=" .. tostring(source_ref.kind or "")
    .. "\nsource_ref.ref=" .. tostring(source_ref.ref or "")
end

function M.build_timeout_reconcile_comment_request(repo, issue_number, reconcile, action, reason, version, fields)
  local marker = M.timeout_reconcile_marker(reconcile.proposal_id, reconcile.issue_version, reconcile.state, reconcile.round, action, fields)
  local state_marker = M.state_marker(reconcile.proposal_id, "blocked", version)
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = "github-devloop timeout reconcile action: " .. tostring(action)
      .. "\n\nReason:\n" .. tostring(reason or "")
      .. "\n\nStructured WHY:\n" .. M.timeout_reconcile_reason_body(fields or {})
      .. "\n\n" .. state_marker .. "\n" .. marker
      .. "\n" .. "⟦AI:FKST⟧",
    dedup_key = M._dedup_key({
      "timeout-reconcile",
      "comment",
      tostring(reconcile.dedup_key),
    }),
    source_ref = M.normalize_source_ref(reconcile.source_ref),
  }
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
    and forge_validators.is_git_sha(payload.head_sha)
    and valid_round(payload.round) ~= nil
    and M._is_bounded_string(payload.dedup_key, M._max_dedup_len)
    and tostring(payload.dedup_key) == "review-reconcile:" .. tostring(payload.issue_version) .. "/review-loop/" .. tostring(payload.round)
    and source_refs.has_bounded_source_ref(payload.source_ref, M._max_key_len)
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
    and forge_validators.is_git_sha(payload.head_sha)
    and valid_round(payload.round) ~= nil
    and tonumber(payload.round) == M.version_fix_round(payload.issue_version)
    and M._is_positive_pr_number(payload.pr_number)
    and M._is_bounded_string(payload.dedup_key, M._max_dedup_len)
    and tostring(payload.dedup_key) == "fix-reconcile:" .. tostring(payload.issue_version)
    and source_refs.has_bounded_source_ref(payload.source_ref, M._max_key_len)
end

function M.is_supported_timeout_reconcile(payload)
  if type(payload) ~= "table" then
    return false
  end
  local repo, issue_number = M.parse_proposal_id(payload.proposal_id)
  local row = replay_fields.restart_transition_row(M.restart_transition_table(), payload.state)
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
    and source_refs.has_bounded_source_ref(payload.source_ref, M._max_key_len)
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

function M.timeout_reconcile_fact_for_terminal_version(comments, proposal_id, terminal_version)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:timeout%-reconcile:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_proposal = attr(marker, "proposal")
      local version = attr(marker, "version")
      local state_name = attr(marker, "state")
      local round = valid_round(attr(marker, "round"))
      local action = attr(marker, "action")
      local from_state = attr(marker, "from_state")
      local from_version = attr(marker, "from_version")
      local dedup = attr(marker, "dedup")
      local expected_dedup = "timeout-reconcile:" .. tostring(from_version)
        .. "/timeout-reconcile/" .. tostring(state_name) .. "/" .. tostring(round)
      if marker_proposal == tostring(proposal_id)
        and version == tostring(terminal_version)
        and action == "drop"
        and round ~= nil
        and state_name ~= nil
        and from_state == state_name
        and replay_fields.restart_transition_row(M.restart_transition_table(), from_state) ~= nil
        and replay_fields.restart_transition_row(M.restart_transition_table(), from_state).terminal == false
        and M._is_bounded_string(from_version, M._max_dedup_len)
        and dedup == expected_dedup then
        return {
          proposal_id = marker_proposal,
          terminal_version = version,
          state = state_name,
          round = round,
          action = action,
          dedup_key = dedup,
          from_state = from_state,
          from_version = from_version,
          source_ref = {
            kind = attr(marker, "source_ref_kind"),
            ref = attr(marker, "source_ref"),
          },
          comment_created_at = M._comment_created_at(comment),
        }
      end
    end
  end
  return nil
end
end

return S
