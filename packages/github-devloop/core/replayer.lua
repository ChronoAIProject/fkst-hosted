local S = {}
function S.install(M)
local function transition_row(state_name)
  for _, row in ipairs(M.restart_transition_table()) do
    if row.from_state == state_name then
      return row
    end
  end
  return nil
end

function M.restart_transition_row(state_name)
  return transition_row(state_name)
end
local marker_aliases = {
  ["pr-delegation"] = { pr = "pr_number", pr_proposal = "pr_proposal_id" },
  ["pr-link"] = { pr = "pr_number" },
  ["review-result"] = { gap = "blocking_gap" },
  ["merge-gate"] = { review_proposal = "review_proposal_id", review_dedup = "review_dedup_key", head_sha = "reviewed_head_sha" },
  ["merge-ready"] = { pr = "pr_number", review_proposal = "review_proposal_id", review_dedup = "review_dedup_key", head_sha = "head_sha" },
  merging = { head_sha = "head_sha" },
  ["review-converge-round"] = { proposal = "proposal_id", dedup = "dedup_key", round = "n" },
}
local function marker_source(facts, family)
  if family == "state" then
    return facts.state
  end
  if family == "pr-delegation" then
    return facts["pr-delegation"] or facts.pr_delegation
  end
  if family == "child-state" then
    return facts.child_state
  end
  if family == "pr-link" then
    return facts.link
  end
  if family == "review-result" or family == "merge-gate" then
    return facts.feedback
  end
  if family == "review-meta" or family == "fix-reflection" or family == "review-converge-round" then
    return facts.review_meta
  end
  if family == "decomposed" then
    return facts.decomposed
  end
  if family == "merge-ready" then
    return facts.merge_ready or facts["merge-ready"]
  end
  return facts[family]
end
local function marker_value(facts, family, attr)
  local source = marker_source(facts, family)
  if source == nil then
    return nil
  end
  if family == "state" and attr == "proposal" then
    return facts.proposal_id
  end
  if attr == "proposal" and source.proposal_id ~= nil then
    return source.proposal_id
  end
  if attr == "dedup" and source.dedup_key ~= nil then
    return source.dedup_key
  end
  local aliases = marker_aliases[family] or {}
  local key = aliases[attr] or attr
  return source[key]
end
local function resolve_payload_fields(row, state, facts)
  local resolved = {}
  local context = facts or {}
  context.state = state
  for field, reference in pairs(row.payload_fields or {}) do
    local family, attr = tostring(reference or ""):match("^marker:([^%.]+)%.(.+)$")
    if family ~= nil then
      resolved[field] = marker_value(context, family, attr)
    else
      local derivation = tostring(reference or ""):match("^source_ref:(.+)$")
      if derivation == "issue" or derivation == "entity" then
        resolved[field] = context.issue and context.issue.source_ref or nil
      elseif derivation == "pr" then
        local pr_number = context.pr_number or (context.link and context.link.pr_number)
          or (context.feedback and context.feedback.pr_number) or (context.review_meta and context.review_meta.pr_number)
          or (context.decomposed and context.decomposed.pr_number)
        resolved[field] = M.pr_source_ref(context.issue and context.issue.repo or "", pr_number)
      end
    end
  end
  return resolved
end

function M.resolve_replay_payload_fields(row, state, facts)
  return resolve_payload_fields(row, state, facts or {})
end

local function find_linked_pr(snapshot, pr_number)
  for _, item in ipairs(snapshot and snapshot.prs or {}) do
    if tostring(item.number or "") == tostring(pr_number or "") then
      return item.current
    end
  end
  return nil
end

local function snapshot_with_pr_comments(current_pr)
  local snapshot = { comments = {}, prs = {} }
  for _, comment in ipairs(current_pr and current_pr.comments or {}) do
    table.insert(snapshot.comments, comment)
  end
  return snapshot
end

local function has_reviewing_marker_for_comments(comments, proposal_id, version)
  return M.has_state_marker(comments, proposal_id, "reviewing", version)
end

local pr_review_tools = nil

local function maybe_terminal_linked_pr_action(dept, issue, state, proposal_id, link, current_pr, facts)
  if pr_review_tools == nil or type(pr_review_tools.terminal_linked_pr_action) ~= "function" then
    return nil
  end
  return pr_review_tools.terminal_linked_pr_action(dept, issue, state, proposal_id, link, current_pr, facts)
end

local function snapshot_from_issue_comments(repo, proposal_id, comments)
  return M.linked_entity_snapshot(repo, proposal_id, comments or {})
end

local function validate_required_fact(required)
  if type(required) ~= "table" or type(required.family) ~= "string" or required.family == "" then
    error("github-devloop: invalid replay required fact")
  end
  if required.freshness ~= "marker-read" and required.freshness ~= "fetch-before-compare" then
    error("github-devloop: invalid replay fact freshness")
  end
end

local function has_required_fact(row, family)
  for _, required in ipairs(row.required_facts or {}) do
    validate_required_fact(required)
    if required.family == family then
      return true
    end
  end
  return false
end

local function current_pr_fact(facts)
  local link = facts.link
  if link == nil then
    return nil
  end
  return find_linked_pr(facts.snapshot, link.pr_number)
end

local function child_pr_delegation_fact(facts)
  return facts.pr_delegation
    or facts["pr-delegation"]
    or M.pr_delegation_fact(facts.snapshot.comments, facts.proposal_id, facts.state and facts.state.version)
end

local function fetch_child_state_fact(facts)
  if facts.child_state ~= nil then
    return facts.child_state
  end
  local delegation = child_pr_delegation_fact(facts)
  if delegation == nil then
    return nil
  end
  facts.pr_delegation = delegation
  facts["pr-delegation"] = delegation
  if facts.current_pr == nil then
    local view = M.fetch_pr_view_origin(facts.issue.repo, delegation.pr_number, nil, {
      force_fresh = true,
      consumer = "replay_child_state",
    })
    if view.exit_code ~= 0 then
      error("github-devloop: child-state PR view failed: " .. tostring(view.stderr))
    end
    facts.current_pr = M.parse_pr_view_origin(view.stdout)
    facts.current_pr.number = delegation.pr_number
  end
  facts.child_state = M.current_entity_state(facts.current_pr.comments, delegation.pr_proposal_id)
  return facts.child_state
end

local function require_marker_fact(facts, family)
  if family == "state" then
    return facts.state
  end
  if family == "pr-link" then
    return facts.link or M.pr_link_fact(facts.snapshot.comments, facts.proposal_id)
  end
  if family == "pr-delegation" then
    return child_pr_delegation_fact(facts)
  end
  if family == "child-state" then
    return fetch_child_state_fact(facts)
  end
  if family == "converge-round" then
    local base_version = M.version_loop_round(facts.state.version) > 0 and M.converge_base_version(facts.state.version) or nil
    return M.latest_complete_converge_round(facts.snapshot.comments, facts.proposal_id, base_version, facts.issue.source_ref)
  end
  if family == "dependency-release" then
    return M.dependency_release_fact(facts.snapshot.comments, facts.proposal_id, facts.state.version)
  end
  if family == "review-result" then
    return M.review_reject_fact(facts.snapshot.comments, facts.proposal_id, facts.state.version)
  end
  if family == "review-meta" then
    local current_pr = current_pr_fact(facts)
    if current_pr ~= nil and M._is_git_sha(current_pr.head_sha) then
      return M.review_meta_replay_fact(facts.snapshot.comments, facts.proposal_id, facts.state.version, facts.link.pr_number, current_pr.head_sha)
    end
    return M.review_meta_fix_fact(facts.snapshot.comments, facts.proposal_id, facts.state.version)
  end
  if family == "fix-reflection" or family == "review-converge-round" then
    local current_pr = current_pr_fact(facts)
    if current_pr == nil or not M._is_git_sha(current_pr.head_sha) then
      return nil
    end
    return M.review_meta_replay_fact(facts.snapshot.comments, facts.proposal_id, facts.state.version, facts.link.pr_number, current_pr.head_sha)
  end
  if family == "merge-gate" then
    return M.merge_gate_fix_fact(facts.snapshot.comments, facts.proposal_id, facts.state.version)
  end
  if family == "merge-gate-wait" then
    local current_pr = current_pr_fact(facts)
    if current_pr == nil or not M._is_git_sha(current_pr.head_sha) then
      return nil
    end
    return M.merge_gate_wait_fact(facts.snapshot.comments, facts.proposal_id, facts.state.version, facts.link.pr_number, current_pr.head_sha)
  end
  if family == "decomposed" then
    local link = facts.link
    if link == nil then
      return nil
    end
    return M.decomposed_fact(facts.snapshot.comments, facts.proposal_id, facts.state.version, link.pr_number)
  end
  if family == "implementing" then
    return M.implementing_fact(facts.snapshot.comments, facts.proposal_id, facts.state.version)
  end
  if family == "implement-attempt" then
    local attempt_version = facts.state.version
    if facts.state.state == "implementing" then
      attempt_version = tostring(attempt_version or "")
        :gsub("/timeout/implementing/%d+$", "")
        :gsub("%-timeout%-implementing%-%d+$", "")
    end
    return M.latest_implement_attempt_fact(facts.snapshot.comments, facts.proposal_id, attempt_version)
  end
  if family == "impl-failure" then
    return M.impl_failure_fact(facts.snapshot.comments, facts.proposal_id, facts.state.version)
  end
  if family == "merge-ready" then
    local current_pr = current_pr_fact(facts)
    if current_pr == nil or not M._is_git_sha(current_pr.head_sha) then
      return nil
    end
    return M.merge_ready_fact(facts.snapshot.comments, facts.proposal_id, facts.state.version, facts.link.pr_number, current_pr.head_sha)
  end
  if family == "merging" then
    local current_pr = current_pr_fact(facts)
    if current_pr == nil or not M._is_git_sha(current_pr.head_sha) then
      return nil
    end
    return M.merging_fact(facts.snapshot.comments, facts.proposal_id, facts.link.pr_number, facts.state.version, current_pr.head_sha)
  end
  if family == "review-carry-over" then
    return nil
  end
  error("github-devloop: unsupported replay marker fact family: " .. tostring(family))
end

local function gather_fetch_before_compare_fact(facts, entity, family)
  if family == "pr-head" then
    if facts.link ~= nil and facts.current_pr ~= nil then
      facts.snapshot = snapshot_with_pr_comments(facts.current_pr)
      for _, comment in ipairs(facts.current and facts.current.comments or {}) do table.insert(facts.snapshot.comments, comment) end
      table.insert(facts.snapshot.prs, { number = facts.link.pr_number, current = facts.current_pr })
      facts.snapshot.state = facts.state
    else
      facts.snapshot = snapshot_from_issue_comments(entity.repo, facts.proposal_id, facts.current and facts.current.comments or {})
      facts.link = M.pr_link_fact(facts.snapshot.comments, facts.proposal_id)
    end
    return true
  end
  if family == "base-head" or family == "ci-status" then
    return true
  end
  if family == "decompose-children" then
    local child_list = M.gh_issue_list_decompose_children(entity.repo, facts.proposal_id, 30)
    if child_list.exit_code ~= 0 then
      error("github-devloop: gh issue decompose child list failed: " .. tostring(child_list.stderr))
    end
    facts.decompose_children = M.parse_decompose_child_issue_list(child_list.stdout)
    return facts.decompose_children
  end
  if family == "branch-head" then
    return true
  end
  error("github-devloop: unsupported replay fetch-before-compare fact family: " .. tostring(family))
end

local function store_gathered_marker_fact(facts, family, value)
  facts[family] = value
  if family == "pr-link" then
    facts.link = value
  elseif family == "review-result" then
    facts.feedback = facts.feedback or value
  elseif family == "review-meta" then
    facts.review_meta = facts.review_meta or value
    facts.feedback = facts.feedback or value
  elseif family == "fix-reflection" or family == "review-converge-round" then
    facts.review_meta = facts.review_meta or value
  elseif family == "merge-gate" then
    facts.feedback = facts.feedback or value
  elseif family == "merge-gate-wait" then
    facts.merge_gate_wait = value
  elseif family == "decomposed" then
    facts.decomposed = value
  elseif family == "impl-failure" then
    facts.impl_failure = value
  elseif family == "merge-ready" then
    facts["merge-ready"] = value
    facts.merge_ready = value
  elseif family == "merging" then
    facts.merging = value
  elseif family == "pr-delegation" then
    facts.pr_delegation = value
    facts["pr-delegation"] = value
  elseif family == "child-state" then
    facts.child_state = value
  end
end

local function gather_required_facts(row, entity, state, provided)
  local gathered = {}
  for key, value in pairs(provided or {}) do
    gathered[key] = value
  end
  gathered.issue = entity
  gathered.state = state
  gathered.proposal_id = gathered.proposal_id or marker_value({ state = state }, "state", "proposal")

  gathered.snapshot = gathered.snapshot or { comments = gathered.current and gathered.current.comments or {}, prs = {}, state = state }
  if gathered.current_pr ~= nil and gathered.link ~= nil then
    -- current_pr.comments may be the SAME table as snapshot.comments: callers
    -- (e.g. the PR liveness sweep) pass current_pr === current and a snapshot
    -- whose comments alias current.comments. Appending into a list while
    -- iterating that same list with ipairs never terminates and allocates
    -- unboundedly. When they alias, the PR comments are already present, so the
    -- append is a no-op; only copy across when they are genuinely distinct lists.
    if gathered.current_pr.comments ~= gathered.snapshot.comments then
      for _, comment in ipairs(gathered.current_pr.comments or {}) do table.insert(gathered.snapshot.comments, comment) end
    end
    table.insert(gathered.snapshot.prs, { number = gathered.link.pr_number, current = gathered.current_pr })
  end

  for _, required in ipairs(row.required_facts or {}) do
    validate_required_fact(required)
    if required.freshness == "fetch-before-compare" then
      gather_fetch_before_compare_fact(gathered, entity, required.family)
    end
  end

  gathered.link = gathered.link or M.pr_link_fact(gathered.snapshot.comments, gathered.proposal_id)

  for _, required in ipairs(row.required_facts or {}) do
    if required.freshness == "marker-read" then
      store_gathered_marker_fact(gathered, required.family, require_marker_fact(gathered, required.family))
    end
  end

  return gathered
end

function M.gather_replay_required_facts(row, entity, state, facts)
  return gather_required_facts(row, entity, state, facts or {})
end

local function log_skip(dept, proposal_id, state, from_state, to_state, outcome, reason)
  if M._replay_skip_capture ~= nil then
    M._replay_skip_capture.outcome = outcome
    M._replay_skip_capture.reason = reason
    M._replay_skip_capture.from_state = from_state
    M._replay_skip_capture.to_state = to_state
  end
  M.log_cas_decision(dept, proposal_id, state, from_state, to_state, outcome, reason)
  return false
end

M.replay_log_skip = log_skip

local function raise_effects(dept, proposal_id, apply_state, version, label_changes, effects)
  local queues = {}
  for _, effect in ipairs(effects or {}) do
    table.insert(queues, effect.queue)
  end
  M.log_apply(dept, proposal_id, apply_state, version, label_changes or { add = {}, remove = {} }, queues)
  for _, effect in ipairs(effects or {}) do
    M.log_raise(dept, proposal_id, effect.queue, effect.payload)
  end
  return true
end

M.replay_raise_effects = raise_effects

local function build_thinking_replay_proposal(issue, proposal_id, state, current, event_ts)
  local stable_version = M.strip_transition_version_suffixes(state.version)
  local state_base_version = M.version_loop_round(stable_version) > 0 and M.converge_base_version(stable_version) or nil
  local latest = M.latest_complete_converge_round(current.comments, proposal_id, state_base_version, issue.source_ref)
  if latest ~= nil then
    local base_version = M.converge_proposal_base_dedup(latest.dedup)
    local next_n = latest.round + 1
    local next_dedup = base_version .. "/loop/" .. tostring(next_n)
    local content_fetch = M.context_fetch_ref_from_bundle({
      dept = "observe_issue",
      repo = issue.repo,
      issue_number = issue.number,
      proposal_id = proposal_id,
      version = next_dedup,
      tick = event_ts,
    })
    local proposal = M.build_board_loop_proposal(issue.repo, issue.number, {
      title = issue.title,
      updated_at = issue.updated_at,
    }, issue.source_ref, next_n, {
      narrowed_question = latest.narrowed_question,
      angle_digests = latest.angle_digests,
    }, event_ts, content_fetch, next_dedup)
    return M.validate_proposal(proposal) and proposal or nil
  end

  local replay_issue = {}
  for key, value in pairs(issue) do
    replay_issue[key] = value
  end
  local replay_dedup = M.proposal_dedup_key(proposal_id, issue.updated_at)
    .. "/replay"
    .. tostring(state.version or ""):sub(#stable_version + 1)
  replay_issue.content_fetch = M.context_fetch_ref_from_bundle({
    dept = "observe_issue",
    repo = issue.repo,
    issue_number = issue.number,
    proposal_id = proposal_id,
    version = replay_dedup,
    tick = event_ts,
  })
  local proposal = M.build_board_proposal(replay_issue, event_ts)
  proposal.dedup_key = replay_dedup
  return M.validate_proposal(proposal) and proposal or nil
end

function M.build_thinking_replay_proposal(issue, proposal_id, state, current, event_ts)
  return build_thinking_replay_proposal(issue, proposal_id, state, current, event_ts)
end

function M.has_thinking_converge_replay(current, proposal_id, state, source_ref)
  if state.state ~= "thinking" then
    return false
  end
  local base_version = M.version_loop_round(state.version) > 0 and M.converge_base_version(state.version) or state.version
  local sr_digest = M.source_ref_digest(source_ref)
  local facts = M.converge_round_facts(current.comments, proposal_id, base_version, sr_digest)
  local round = M.max_converge_round(facts)
  return M.latest_complete_converge_round(current.comments, proposal_id, base_version, source_ref) ~= nil
    or M.is_true_stall(facts, round)
end

local function replay_thinking(dept, issue, state, row, facts)
  local proposal_id = facts.proposal_id
  M.log_cas_decision(dept, proposal_id, state, "unmanaged", "thinking", "skip-idempotent(already at to_state)", "trusted thinking state marker is already visible")
  local proposal = build_thinking_replay_proposal(issue, proposal_id, state, facts.current, facts.event_ts)
  if proposal == nil then
    return log_skip(dept, proposal_id, state, row.from_state, row.driving_queue, "skip-foreign(payload)", "cannot rebuild thinking replay proposal")
  end
  M.log_cas_decision(dept, proposal_id, state, row.from_state, row.driving_queue, "applied(replay)", "replaying consensus proposal from trusted state facts")
  return raise_effects(dept, proposal_id, "thinking", proposal.dedup_key, { add = {}, remove = {} }, {
    { queue = "consensus.proposal", payload = proposal },
  })
end

local function replay_implementing(dept, issue, state, row, facts)
  local proposal_id = facts.proposal_id
  local attempt = facts["implement-attempt"]
  if attempt == nil then
    if facts.implementing == nil then
      return log_skip(dept, proposal_id, state, "implementing", row.driving_queue, "skip-pending(no-attempt-marker)", "implement attempt marker is not visible")
    end
  end
  if attempt ~= nil and type(attempt.exec_ref) == "string" and attempt.exec_ref ~= "" and M.implement_exec_ref_running(attempt.exec_ref) then
    return log_skip(dept, proposal_id, state, "implementing", row.driving_queue, "skip-pending(codex-run-live)", "matching implement codex run is still running")
  end
  -- Pass the INNER (unwrapped) version: build_devloop_ready_payload re-applies
  -- the "ready/" wrapper, so re-wrapping the already-wrapped state.version would
  -- double-wrap it ("ready/ready/..."). Preserve the retry suffix as structured
  -- attempt metadata so re-drives reproduce frozen "ready/.../reimplement/N"
  -- markers exactly.
  local payload = M.build_devloop_ready_payload({
    proposal_id = proposal_id,
    dedup_key = M.ready_payload_inner_version(state.version),
    source_ref = issue.source_ref,
    impl_retry_attempt = M.implementation_retry_attempt(state.version),
  })
  M.log_cas_decision(dept, proposal_id, state, "implementing", "implementing", "applied(codex-run-absent)", "no matching implement codex run is running")
  return raise_effects(dept, proposal_id, "implementing", state.version, { add = {}, remove = {} }, {
    { queue = "devloop_ready", payload = payload },
  })
end

local function replay_impl_failed(dept, issue, state, row, facts)
  local proposal_id = facts.proposal_id
  local failure = facts.impl_failure
  if not M.impl_failure_retry_allowed(failure) then
    return log_skip(dept, proposal_id, state, "impl-failed", "implementing", "skip-idempotent(retry-limit)", "implementation failure is not a bounded codex retry candidate")
  end
  local fields = resolve_payload_fields(row, state, {
    issue = issue,
    state = state,
    proposal_id = proposal_id,
    ["impl-failure"] = failure,
  })
  local payload = M.build_devloop_ready_payload({
    proposal_id = fields.proposal_id,
    dedup_key = M.ready_payload_inner_version(fields.dedup_key),
    source_ref = fields.source_ref,
    impl_retry_attempt = M.next_impl_retry_attempt(failure),
  })
  M.log_cas_decision(dept, proposal_id, state, "impl-failed", "implementing", "applied(replay)", "retryable implementation failure is below the retry ceiling")
  return raise_effects(dept, proposal_id, nil, nil, { add = {}, remove = {} }, {
    { queue = "devloop_ready", payload = payload },
  })
end

local function replay_fixing_to_reviewing(dept, issue, state, proposal_id, link, current_pr, feedback, source_ref)
  local intended_head_sha = M.current_branch_head_sha(link.branch)
  if intended_head_sha == nil then
    M.log_cas_decision(dept, proposal_id, state, "fixing", "reviewing", "retry-pending(head-advanced)", "PR head changed and deterministic branch head is not readable")
    error("github-devloop: PR head changed before fix replay and deterministic branch head is not readable")
  end
  if tostring(current_pr.head_sha or "") ~= intended_head_sha then
    return log_skip(dept, proposal_id, state, "fixing", "fixing", "skip-stale(head-advanced)", "PR head advanced since rejected review")
  end
  local reviewing_version = M.next_fix_version(state.version)
  local comments = (issue._replay_issue_comments ~= nil and issue._replay_issue_comments) or {}
  if has_reviewing_marker_for_comments(comments, proposal_id, reviewing_version)
    or has_reviewing_marker_for_comments(current_pr.comments, proposal_id, reviewing_version) then
    return log_skip(dept, proposal_id, state, "fixing", "reviewing", "skip-idempotent(reviewing marker already visible)", "reviewing state marker for recovered head is already visible")
  end
  local fix = {
    proposal_id = proposal_id,
    pr_number = link.pr_number,
    version = state.version,
    review_proposal_id = feedback.review_proposal_id,
    review_dedup_key = feedback.review_dedup_key,
    reviewed_head_sha = feedback.reviewed_head_sha,
    source_ref = source_ref,
  }
  M.raise_fix_reviewing({
    dept = dept,
    repo = issue.repo,
    issue_number = issue.number,
    fix = fix,
    old_head_sha = feedback.reviewed_head_sha,
    new_head_sha = current_pr.head_sha,
    new_version = reviewing_version,
    reason = "push already visible; self-healing missing reviewing marker",
    current_state = state,
  })
  return true
end

local function replay_fixing(dept, issue, state, row, facts)
  local proposal_id = facts.proposal_id
  local link = facts.link
  if link == nil or not M.fixing_version_matches_link(state.version, link.impl_version) then
    return log_skip(dept, proposal_id, state, "fixing", "fixing|reviewing", "skip-foreign(pr-link)", "fixing recovery requires a same-version pr-link marker")
  end
  local current_pr = find_linked_pr(facts.snapshot, link.pr_number)
  if current_pr == nil then
    local terminal = maybe_terminal_linked_pr_action(dept, issue, state, proposal_id, link, nil, facts)
    if terminal ~= nil then return terminal end
    return log_skip(dept, proposal_id, state, "fixing", "fixing|reviewing", "skip-foreign(pr-link)", "linked PR fact is not visible")
  end
  local terminal = maybe_terminal_linked_pr_action(dept, issue, state, proposal_id, link, current_pr, facts)
  if terminal ~= nil then return terminal end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    return log_skip(dept, proposal_id, state, "fixing", "fixing|reviewing", "skip-stale(pr-closed)", "linked PR is not open")
  end
  if not M._is_git_sha(current_pr.head_sha) then
    return log_skip(dept, proposal_id, state, "fixing", "fixing|reviewing", "skip-foreign(head)", "linked PR head sha is missing")
  end

  local feedback = M.fixing_replay_feedback_fact(facts.snapshot.comments, proposal_id, state.version)
  if feedback ~= nil then
    if feedback.review_proposal_id == nil or feedback.reviewed_head_sha == nil then
      return log_skip(dept, proposal_id, state, "fixing", "fixing", "skip-foreign(fix-feedback-binding)", "trusted fix feedback marker lacks review binding")
    end
    if tostring(current_pr.head_sha or "") ~= tostring(feedback.reviewed_head_sha or "") then
      return replay_fixing_to_reviewing(dept, issue, state, proposal_id, link, current_pr, feedback, facts.source_ref or M.pr_source_ref(issue.repo, link.pr_number))
    end
    local reviewing_version = M.next_fix_version(state.version)
    if has_reviewing_marker_for_comments(facts.snapshot.comments, proposal_id, reviewing_version) then
      return log_skip(dept, proposal_id, state, "fixing", "reviewing", "skip-idempotent(reviewing marker already visible)", "reviewing state marker for fix is already visible")
    end
    local fields = resolve_payload_fields(row, state, {
      issue = issue,
      state = state,
      link = link,
      feedback = feedback,
      proposal_id = proposal_id,
    })
    local fix_payload = M.build_replayed_fixing_payload({
      proposal_id = fields.proposal_id,
      impl_version = fields.version,
    }, fields.pr_number, feedback, fields.source_ref)
    M.log_cas_decision(dept, proposal_id, state, "fixing", "fixing", "applied(replay)", "trusted feedback fact is visible")
    return raise_effects(dept, proposal_id, "fixing", state.version, { add = {}, remove = {} }, {
      { queue = "devloop_fixing", payload = fix_payload },
    })
  end

  if dept ~= "observe_pr" then
    local new_version = M.next_fix_version(state.version)
    local source_ref = M.pr_source_ref(issue.repo, link.pr_number)
    local reviewing_payload = M.build_devloop_reviewing_payload({
      proposal_id = proposal_id,
      impl_version = new_version,
    }, link.pr_number, source_ref, new_version)
    local comment_request = M.build_merge_head_reviewing_comment_request(
      issue.repo,
      issue.number,
      {
        proposal_id = proposal_id,
        pr_number = link.pr_number,
      },
      current_pr.head_sha,
      current_pr.head_sha,
      new_version,
      source_ref
    )
    local label_request = M.build_state_label_request(issue.repo, issue.number, "reviewing", M._dedup_key({
      "observe",
      "fixing",
      "renormalize",
      tostring(proposal_id),
      tostring(new_version),
      tostring(link.pr_number),
    }), issue.source_ref)
    M.log_cas_decision(dept, proposal_id, state, "fixing", "reviewing", "applied(replay)", "no feedback fact is visible; re-entering review for current PR head")
    return raise_effects(dept, proposal_id, "reviewing", new_version, { add = { "fkst-dev:reviewing" }, remove = { "fkst-dev:fixing" } }, {
      { queue = "github-proxy.github_pr_comment_request", payload = comment_request },
      { queue = "github-proxy.github_issue_label_request", payload = label_request },
      { queue = "devloop_reviewing", payload = reviewing_payload },
    })
  end

  return log_skip(dept, proposal_id, state, "fixing", "fixing", "skip-stale(no-trusted-fix-feedback)", "trusted fix feedback marker is not visible")
end

local function replay_review_meta(dept, issue, state, row, facts)
  local proposal_id = facts.proposal_id
  local link = facts.link
  if link == nil then
    return log_skip(dept, proposal_id, state, "review-meta", "review-meta", "skip-foreign(pr-link)", "review-meta recovery requires a pr-link marker")
  end
  local current_pr = find_linked_pr(facts.snapshot, link.pr_number)
  if current_pr == nil then
    local terminal = maybe_terminal_linked_pr_action(dept, issue, state, proposal_id, link, nil, facts)
    if terminal ~= nil then return terminal end
    return log_skip(dept, proposal_id, state, "review-meta", "review-meta", "skip-foreign(pr-link)", "linked PR fact is not visible")
  end
  local terminal = maybe_terminal_linked_pr_action(dept, issue, state, proposal_id, link, current_pr, facts)
  if terminal ~= nil then return terminal end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    return log_skip(dept, proposal_id, state, "review-meta", "review-meta", "skip-stale(pr-closed)", "linked PR is not open")
  end
  if tostring(current_pr.head_ref_name or "") ~= tostring(link.branch or "") then
    return log_skip(dept, proposal_id, state, "review-meta", "review-meta", "skip-foreign(head)", "linked PR head branch does not match pr-link marker")
  end
  if tostring(current_pr.base_ref_name or "") ~= tostring(link.base_branch or "") then
    return log_skip(dept, proposal_id, state, "review-meta", "review-meta", "skip-foreign(base)", "linked PR base branch does not match pr-link marker")
  end
  if not M._is_git_sha(current_pr.head_sha) then
    return log_skip(dept, proposal_id, state, "review-meta", "review-meta", "skip-foreign(head)", "linked PR head sha is missing")
  end
  local fact = M.review_meta_replay_fact(facts.snapshot.comments, proposal_id, state.version, link.pr_number, current_pr.head_sha)
  if fact == nil then
    return log_skip(dept, proposal_id, state, "review-meta", "review-meta", "skip-foreign(review-meta)", "review-meta recovery facts are not visible")
  end
  local fields = resolve_payload_fields(row, state, {
    issue = issue,
    state = state,
    link = link,
    review_meta = fact,
    proposal_id = proposal_id,
  })
  local payload = nil
  if fact.mode == "fix-reflection" then
    payload = M.build_devloop_fix_reflection_payload(fact, proposal_id, fields.version, fields.pr_number, fact.fix_round or fact.n, fields.source_ref)
    payload.blocking_gap = fact.blocking_gap
  else
    payload = M.build_devloop_review_meta_payload(fact, proposal_id, fields.version, fields.pr_number, fact.n, fields.source_ref)
  end
  M.log_cas_decision(dept, proposal_id, state, "review-meta", "review-meta", "applied(replay)", "trusted review-meta fact is visible")
  return raise_effects(dept, proposal_id, "review-meta", state.version, { add = {}, remove = {} }, {
    { queue = "devloop_review_meta", payload = payload },
  })
end

local function raise_reviewing_for_current_head(dept, issue, state, proposal_id, link, current_pr, outcome, reason)
  if tostring(current_pr.state or ""):lower() ~= "open" then
    return log_skip(dept, proposal_id, state, "merge-ready", "reviewing", "skip-stale(pr-closed)", "linked PR is not open")
  end
  if not M._is_git_sha(current_pr.head_sha) then
    return log_skip(dept, proposal_id, state, "merge-ready", "reviewing", "skip-foreign(head)", "linked PR head sha is missing")
  end
  local reviewing_payload = M.build_current_head_reviewing_payload({ repo = issue.repo, proposal_id = proposal_id }, link.pr_number, current_pr, state, M.pr_source_ref(issue.repo, link.pr_number))
  M.log_cas_decision(dept, proposal_id, state, "merge-ready", "reviewing", outcome, reason)
  if reviewing_payload == nil then
    return false
  end
  return raise_effects(dept, proposal_id, nil, nil, { add = {}, remove = {} }, {
    { queue = "devloop_reviewing", payload = reviewing_payload },
  })
end

local function maybe_replay_review_carry_over(dept, issue, state, row, facts, link, current_pr)
  local proposal_id = facts.proposal_id
  if state.state ~= "merge-ready" then
    return false
  end
  if tostring(current_pr.state or ""):lower() ~= "open" or not M.is_safe_head_sha(current_pr.head_sha) then
    return false
  end
  local carry, carry_reason = M.approved_lineage_carry_over(
    issue.repo,
    link.pr_number,
    proposal_id,
    state.version,
    facts.snapshot.comments,
    link.base_branch,
    current_pr.head_sha
  )
  if carry_reason == "missing-merge-ready-fact" or carry_reason == "head-unchanged" then
    return false
  end
  if carry_reason == "missing-review-result-approve" then
    return false
  end
  if carry == nil then
    local outcome = "skip-stale(" .. tostring(carry_reason):match("^([^:]+)") .. ")"
    return raise_reviewing_for_current_head(dept, issue, state, proposal_id, link, current_pr, outcome, tostring(carry_reason))
  end
  if M.has_any_review_result_marker(current_pr.comments, carry.new_review_proposal_id, proposal_id) then
    return false
  end
  local source_ref = M.pr_source_ref(issue.repo, link.pr_number)
  local comment_request = M.build_review_carry_over_comment_request(issue.repo, link.pr_number, proposal_id, state.version, carry, source_ref)
  local merge_payload = M.build_devloop_merge_ready_payload(proposal_id, link.pr_number, state.version, {
    review_proposal_id = carry.new_review_proposal_id,
    review_dedup_key = carry.new_review_dedup_key,
    reviewed_head_sha = current_pr.head_sha,
    current_head_sha = current_pr.head_sha,
  }, source_ref)
  M.log_cas_decision(dept, proposal_id, state, "merge-ready", "merge-ready", "applied(review-carry-over)", "resolution delta is empty")
  return raise_effects(dept, proposal_id, "merge-ready", state.version, { add = {}, remove = {} }, {
    { queue = "github-proxy.github_pr_comment_request", payload = comment_request },
    { queue = "devloop_merge_ready", payload = merge_payload },
  })
end

local function replay_merge_ready_like(dept, issue, state, row, facts)
  local proposal_id = facts.proposal_id
  local link = facts.link
  if link == nil then
    return log_skip(dept, proposal_id, state, row.from_state, "merge-ready", "skip-foreign(pr-link)", "merge-ready recovery requires a pr-link marker")
  end
  local current_pr = find_linked_pr(facts.snapshot, link.pr_number)
  if current_pr == nil then
    local terminal = maybe_terminal_linked_pr_action(dept, issue, state, proposal_id, link, nil, facts)
    if terminal ~= nil then return terminal end
    return log_skip(dept, proposal_id, state, row.from_state, "merge-ready", "skip-foreign(pr-link)", "linked PR fact is not visible")
  end
  local terminal = maybe_terminal_linked_pr_action(dept, issue, state, proposal_id, link, current_pr, facts)
  if terminal ~= nil then return terminal end
  if maybe_replay_review_carry_over(dept, issue, state, row, facts, link, current_pr) then
    return true
  end
  local fact = M.merge_ready_fact(facts.snapshot.comments, proposal_id, state.version, link.pr_number, current_pr.head_sha)
  if fact == nil then
    return log_skip(dept, proposal_id, state, row.from_state, "merge-ready", "skip-foreign(merge-ready)", "head-bound merge-ready marker is not visible")
  end
  local fields = resolve_payload_fields(row, state, {
    issue = issue,
    state = state,
    link = link,
    merge_ready = fact,
    ["merge-ready"] = fact,
    proposal_id = proposal_id,
  })
  local payload = M.build_devloop_merge_ready_payload(fields.proposal_id, fields.pr_number, fields.version, {
    review_proposal_id = fields.review_proposal_id,
    review_dedup_key = fields.review_dedup_key,
    reviewed_head_sha = fields.reviewed_head_sha,
    current_head_sha = current_pr.head_sha,
  }, fields.source_ref)
  M.log_cas_decision(dept, proposal_id, state, row.from_state, "merge-ready", "applied(replay)", "trusted head-bound merge-ready fact is visible")
  return raise_effects(dept, proposal_id, nil, nil, { add = {}, remove = {} }, {
    { queue = "devloop_merge_ready", payload = payload },
  })
end

local function replay_blocked(dept, issue, state, row, facts)
  local proposal_id = facts.proposal_id
  local link = facts.link
  if link == nil then
    return log_skip(dept, proposal_id, state, "blocked", "decomposed", "skip-foreign(pr-link)", "decompose recovery requires a pr-link marker")
  end
  local current_pr = find_linked_pr(facts.snapshot, link.pr_number)
  local decomposed = M.decomposed_fact(facts.snapshot.comments, proposal_id, state.version, link.pr_number)
  if decomposed == nil then
    return log_skip(dept, proposal_id, state, "blocked", "decomposed", "skip-foreign(decomposed)", "decomposed marker is not visible")
  end
  local complete, completed_count = M.decompose_children_complete(
    facts.snapshot.comments,
    facts.decompose_children or {},
    proposal_id,
    decomposed.version,
    decomposed.pr_number,
    decomposed.count
  )
  if complete then
    return log_skip(dept, proposal_id, state, "blocked", "decomposed", "skip-idempotent(decomposed children already visible)", "decompose children are complete")
  end
  local fields = resolve_payload_fields(row, state, {
    issue = issue,
    state = state,
    link = link,
    decomposed = decomposed,
    proposal_id = proposal_id,
  })
  local payload = M.build_decompose_replay_payload(decomposed, facts.snapshot.comments, fields.source_ref, completed_count)
  if payload == nil then
    return log_skip(dept, proposal_id, state, "blocked", "decomposed", "skip-foreign(decompose-binding)", "trusted fix feedback for decomposed replay is not visible")
  end
  M.log_cas_decision(dept, proposal_id, state, "blocked", "decomposed", "applied(decomposed-children-missing)", "decomposed marker count exceeds derived child count " .. tostring(completed_count))
  return raise_effects(dept, proposal_id, "blocked", state.version, { add = {}, remove = {} }, {
    { queue = "devloop_decompose", payload = payload },
  })
end

local replayers = {
  thinking = replay_thinking,
  dependency_wait = M.replay_dependency_wait_state,
  ready = M.replay_ready_state,
  implementing = replay_implementing,
  ["awaiting-pr"] = M.replay_awaiting_pr_state,
  ["impl-failed"] = replay_impl_failed,
  fixing = replay_fixing,
  ["review-meta"] = replay_review_meta,
  ["merge-ready"] = replay_merge_ready_like,
  merging = replay_merge_ready_like,
  blocked = replay_blocked,
}

pr_review_tools = {
  find_linked_pr = find_linked_pr,
  log_skip = log_skip,
  raise_effects = raise_effects,
  resolve_payload_fields = resolve_payload_fields,
}
M.install_pr_review_replayers(replayers, pr_review_tools)

function M.replay_from_table(dept, entity, state, table_row, facts)
  local row = table_row or transition_row(state and state.state)
  local proposal_id = facts and facts.proposal_id or nil
  if row == nil then
    return log_skip(dept, proposal_id, state, "unknown", "unknown", "skip-foreign(table-row)", "no restart transition table row is declared")
  end
  if type(state) ~= "table" or state.state ~= row.from_state then
    return log_skip(dept, proposal_id, state, row.from_state, row.driving_queue, "skip-foreign(state)", "current state does not match restart transition table row")
  end
  local replay = replayers[row.from_state]
  if replay == nil then return log_skip(dept, proposal_id, state, row.from_state, row.driving_queue, "skip-foreign(replayer)", "restart transition table row is not replayable by this department") end
  local replay_facts = gather_required_facts(row, entity, state, facts or {})
  return replay(dept, entity, state, row, replay_facts)
end

function M.replay_from_table_classified(dept, entity, state, table_row, facts)
  local capture = {}
  local previous = M._replay_skip_capture
  M._replay_skip_capture = capture
  local ok, issued = pcall(function() return M.replay_from_table(dept, entity, state, table_row, facts) end)
  M._replay_skip_capture = previous
  if not ok then error(issued) end
  if issued then
    return { kind = "issued", issued = true }
  end
  return { kind = "stuck", issued = false, outcome = capture.outcome, reason = capture.reason }
end
end

return S
