local core = require("core")
local saga = require("std.saga")

local LIVENESS_SCAN_MAX_PER_TICK = 100
local LIVENESS_SCAN_CALL_TIMEOUT = 10
local LIVENESS_SCAN_WALL_CLOCK_BUDGET = 25
local LIVENESS_SCAN_CURSOR_PREFIX = "github-devloop/liveness-scan/cursor/"

local spec = {
  consumes = { "devloop_liveness_tick" },
  produces = {
    "github-proxy.github_entity_changed",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_pr_comment_request",
    "consensus.proposal",
    "devloop_ready",
    "devloop_reviewing",
    "devloop_fixing",
    "devloop_review_meta",
    "devloop_merge_ready",
    "devloop_decompose",
    "devloop_reconcile",
    "devloop_review_reconcile",
    "devloop_timeout_reconcile",
  },
  fanout = { "devloop_liveness_tick" },
  stall_window = "30s",
}

local function read_repo()
  local repo = core.read_env("FKST_GITHUB_REPO")
  if repo == nil or not core.issue_ref_round_trips(repo, 1) then
    return nil
  end
  return repo
end

local function state_is_non_terminal(state)
  local row = core.restart_transition_row(state and state.state)
  return row ~= nil and row.terminal ~= true
end

local function liveness_scan_limits()
  return {
    entity_cap = LIVENESS_SCAN_MAX_PER_TICK,
    call_timeout = LIVENESS_SCAN_CALL_TIMEOUT,
    wall_clock_budget = LIVENESS_SCAN_WALL_CLOCK_BUDGET,
  }
end

local function log_deferred(reason, fields)
  core.log_line("info", "liveness_scan", "github-devloop/liveness-scan", "LIVENESS_DEFERRED", {
    "reason=" .. tostring(reason or "budget"),
    "listed_issues=" .. tostring(fields and fields.listed_issues or 0),
    "listed_prs=" .. tostring(fields and fields.listed_prs or 0),
    "processed=" .. tostring(fields and fields.processed or 0),
    "deferred=" .. tostring(fields and fields.deferred or 0),
    "entity_cap=" .. tostring(fields and fields.entity_cap or 0),
  })
end

local function liveness_cursor_key(repo)
  return LIVENESS_SCAN_CURSOR_PREFIX .. core.safe_repo(repo)
end

local function is_timeout_result(result)
  return type(result) == "table" and result.exit_code ~= 0
    and (tonumber(result.exit_code) == 124 or core.error_fact_class({ message = result.stderr }) == "timeout")
end

local function update_liveness_cursor(cursor_key, cursor, total, processed)
  if cursor_key == nil then
    return
  end
  cache_set(cursor_key, tostring(core.sweep_cursor_advance(cursor, total, processed)))
end

local function build_observe_payload(repo, entity, kind, tick)
  local number = tostring(entity.number or "")
  local updated_at = tostring(entity.updated_at or "")
  local source_ref = kind == "pr" and core.pr_source_ref(repo, number) or core.issue_source_ref(repo, number)
  return {
    schema = "github-proxy.v1",
    type = kind,
    repo = repo,
    number = tonumber(number),
    state = entity.state,
    updated_at = updated_at,
    dedup_key = core._dedup_key({
      "liveness-scan",
      tostring(repo),
      kind,
      number,
      updated_at,
      tostring(tick or ""),
    }),
    source = "liveness-scan",
    source_ref = source_ref,
  }
end

local function should_reinject_state(proposal_id, state)
  if state == nil or state.state == nil then
    core.log_cas_decision("liveness_scan", proposal_id, { state = nil, version = nil }, "tick", "observe", "skip-no-state", "no current restart state marker")
    return false
  end
  if not state_is_non_terminal(state) then
    core.log_cas_decision("liveness_scan", proposal_id, state, "tick", "observe", "skip-terminal", "current restart state is terminal or unknown")
    return false
  end
  return true
end

local function issue_entity(repo, issue_number)
  return {
    repo = repo,
    number = issue_number,
    source_ref = core.issue_source_ref(repo, issue_number),
  }
end

local function issue_state_needs_pr_surface(state_name)
  return state_name == "pr-open"
    or state_name == "reviewing"
    or state_name == "fixing"
    or state_name == "review-meta"
    or state_name == "merge-ready"
    or state_name == "merging"
end

local function maybe_timeout_action(entity, state, facts)
  local row = core.restart_transition_row(state and state.state)
  if row == nil or row.terminal == true then
    return nil
  end
  local epoch = row.actionable_epoch
  if type(epoch) == "table"
    and epoch.allows_state_entry_if_never_deferred == true
    and type(facts.dependency_gate) ~= "table" then
    facts.dependency_gate = core.dependency_gate(entity and entity.repo, entity and entity.number, {
      proposal_id = facts.proposal_id or state.proposal_id,
      version = state and state.version,
      comments = facts.current and facts.current.comments,
    })
  end
  if state.state == "ready" then
    facts.dependency_gate = facts.dependency_gate or core.dependency_gate(entity and entity.repo, entity and entity.number, {
      proposal_id = facts.proposal_id or state.proposal_id,
      version = state and state.version,
      comments = facts.current and facts.current.comments,
    })
    if core.canonicalize_legacy_ready_dependency_wait("liveness_scan", entity, state, facts) then
      return "handled"
    end
  end
  local proposal_id = facts.proposal_id or state.proposal_id
  if core.restart_row_liveness_deferred(row, state, facts, facts.now_seconds or now()) then
    core.log_cas_decision("liveness_scan", proposal_id, state, row.from_state, row.driving_queue, "skip-active-output-obligation", "receiver liveness contract signal is still fresh")
    return nil
  end
  if core.maybe_timeout_redrive_from_table("liveness_scan", entity, state, row, facts) then
    return "handled"
  end
  return nil
end

local function should_reinject_issue(repo, issue, limits, deadline)
  if not core.issue_ref_round_trips(repo, issue.number) then
    return false
  end

  if not core.sweep_has_budget(deadline) then
    return nil, "deadline"
  end
  local proposal_id = core.proposal_id(repo, issue.number)
  local state_view = core.fetch_issue_view_state(repo, issue.number, issue.updated_at, {
    consumer = "liveness_scan",
    timeout = core.sweep_call_timeout(limits, deadline),
  })
  if state_view.exit_code ~= 0 then
    if is_timeout_result(state_view) then
      return nil, "deadline"
    end
    error("github-devloop: liveness-scan-issue-view-failed: " .. tostring(state_view.stderr))
  end

  local current = core.parse_issue_view_state(state_view.stdout)
  if tostring(current.state or ""):upper() ~= "OPEN" then
    core.log_cas_decision("liveness_scan", proposal_id, { state = nil, version = nil }, "tick", "observe", "skip-closed", "issue is not open")
    return false
  end

  local issue_state = core.current_entity_state(current.comments, proposal_id)
  local snapshot = issue_state_needs_pr_surface(issue_state and issue_state.state)
    and core.linked_pr_surface_snapshot(repo, proposal_id, current.comments, { cache_only = true })
    or { comments = current.comments or {}, prs = {}, absent_prs = {}, state = issue_state }
  if snapshot.deferred == true then
    core.log_cas_decision("liveness_scan", proposal_id, issue_state, "tick", "observe", "liveness-deadline-deferred:" .. tostring(snapshot.defer_reason or "linked-surface"), "linked PR surface is not cached for this sweep")
    return false
  end
  snapshot.state = issue_state
  local state = issue_state
  if not should_reinject_state(proposal_id, state) then
    return false
  end
  local link = core.pr_link_fact(snapshot.comments, proposal_id)
  local current_pr = nil
  if link ~= nil then
    for _, item in ipairs(snapshot.prs or {}) do
      if tostring(item.number or "") == tostring(link.pr_number or "") then
        current_pr = item.current
        break
      end
    end
  end
  local pr_phase_source_ref = current_pr ~= nil and link ~= nil and state.state ~= "pr-open"
  local timeout_action = maybe_timeout_action(issue_entity(repo, issue.number), state, {
    proposal_id = proposal_id,
    current = { comments = snapshot.comments, labels = current.labels or {} },
    current_issue = current,
    current_pr = current_pr,
    link = link,
    snapshot = snapshot,
    event_ts = issue.updated_at,
    source_ref = pr_phase_source_ref
      and core.pr_source_ref(repo, link.pr_number)
      or core.issue_source_ref(repo, issue.number),
    head_sha = current_pr and current_pr.head_sha or nil,
    review_proposal_id = state.state == "reviewing" and current_pr ~= nil and core._is_git_sha(current_pr.head_sha)
      and link ~= nil
      and core.pr_review_proposal_id(repo, link.pr_number, state.version, current_pr.head_sha)
      or nil,
    fresh_current_state = state,
    now_seconds = now(),
  })
  if timeout_action == "handled" then
    return false
  end
  return true
end

local function should_reinject_pr(repo, pr, limits, deadline)
  if not core._is_positive_pr_number(pr.number) then
    return false
  end

  if not core.sweep_has_budget(deadline) then
    return nil, "deadline"
  end
  local state_view = core.fetch_pr_view_origin(repo, pr.number, pr.updated_at, {
    consumer = "liveness_scan",
    timeout = core.sweep_call_timeout(limits, deadline),
  })
  if state_view.exit_code ~= 0 then
    if is_timeout_result(state_view) then
      return nil, "deadline"
    end
    error("github-devloop: liveness-scan-pr-view-failed: " .. tostring(state_view.stderr))
  end

  local current = core.parse_pr_view_origin(state_view.stdout)
  local origin = core.pr_origin_fact(current.comments)
  local proposal_id = origin and origin.proposal_id or core.pr_proposal_id(repo, pr.number)
  if origin == nil then
    core.log_cas_decision("liveness_scan", proposal_id, { state = nil, version = nil }, "tick", "observe", "skip-no-state", "PR has no origin marker")
    return false
  end
  if not core.verify_pr_review_issue_claim("liveness_scan", origin.repo, origin.issue_number, nil, origin.proposal_id) then
    return false
  end

  local state = core.current_entity_state(current.comments, origin.proposal_id)
  if not should_reinject_state(proposal_id, state) then
    return false
  end
  local entity = issue_entity(origin.repo, origin.issue_number)
  local source_ref = core.pr_source_ref(repo, pr.number)
  local timeout_action = maybe_timeout_action(entity, state, {
    proposal_id = origin.proposal_id,
    current = { comments = current.comments or {}, labels = current.labels or {} },
    current_pr = current,
    link = {
      proposal_id = origin.proposal_id,
      pr_number = pr.number,
      branch = origin.branch,
      impl_version = origin.impl_version,
      base_branch = origin.base_branch,
    },
    snapshot = {
      comments = current.comments or {},
      prs = { { number = pr.number, current = current } },
      state = state,
    },
    source_ref = source_ref,
    head_sha = current.head_sha,
    review_proposal_id = state.state == "reviewing" and core._is_git_sha(current.head_sha)
      and core.pr_review_proposal_id(origin.repo, pr.number, state.version, current.head_sha)
      or nil,
    now_seconds = now(),
  })
  if timeout_action == "handled" then
    return false
  end
  return true
end

local function list_open_issues(repo, timeout, poll_key)
  local list = core.fetch_shared_issue_observe_list(repo, {
    timeout = timeout or 60,
    poll_key = poll_key,
  })
  if list.exit_code ~= 0 then
    error("github-devloop: liveness-scan-issue-list-failed: " .. tostring(list.stderr))
  end
  return core.parse_issue_list_observe(list.stdout)
end

local function list_open_prs(repo, timeout, poll_key)
  local list = core.fetch_shared_pr_observe_list(repo, {
    timeout = timeout or 60,
    poll_key = poll_key,
  })
  if list.exit_code ~= 0 then
    error("github-devloop: liveness-scan-pr-list-failed: " .. tostring(list.stderr))
  end
  return core.parse_pr_list_observe(list.stdout)
end

local function sort_by_number(items)
  table.sort(items, function(left, right)
    return tonumber(left.number or 0) < tonumber(right.number or 0)
  end)
  return items
end

local function activation_slice(repo, issues, prs)
  local activations = {}
  for _, issue in ipairs(sort_by_number(issues or {})) do
    table.insert(activations, { kind = "issue", entity = issue })
  end
  for _, pr in ipairs(sort_by_number(prs or {})) do
    table.insert(activations, { kind = "pr", entity = pr })
  end
  local total = #activations
  if total > LIVENESS_SCAN_MAX_PER_TICK then
    local cursor_key = liveness_cursor_key(repo)
    local cursor = cache_get(cursor_key)
    local bounded, deferred = core.sweep_cursor_batch(
      activations,
      cursor,
      LIVENESS_SCAN_MAX_PER_TICK,
      LIVENESS_SCAN_MAX_PER_TICK
    )
    core.log_cas_decision("liveness_scan", "github-devloop/liveness-scan", { state = nil, version = nil }, "tick", "observe", "deferred-cap", tostring(total - LIVENESS_SCAN_MAX_PER_TICK) .. " open entities deferred by LIVENESS_SCAN_MAX_PER_TICK")
    return bounded, deferred, cursor_key, cursor, total
  end
  cache_set(liveness_cursor_key(repo), "0")
  return activations, 0
end

local function reinject(repo, entity, kind, tick)
  local proposal_id = kind == "pr" and core.pr_proposal_id(repo, entity.number) or core.proposal_id(repo, entity.number)
  local payload = build_observe_payload(repo, entity, kind, tick)
  core.log_apply("liveness_scan", proposal_id, nil, nil, { add = {}, remove = {} }, {
    "github-proxy.github_entity_changed",
  })
  core.log_raise("liveness_scan", proposal_id, "github-proxy.github_entity_changed", payload)
end

local function liveness_scan_done(_event)
  return false
end

local function act_liveness_scan(event)
  core.log_entry("liveness_scan", event, "github-devloop/liveness-scan", "tick")
  core.assert_trusted_bot_configured()

  local repo = read_repo()
  if repo == nil then
    core.log_cas_decision("liveness_scan", "github-devloop/liveness-scan", { state = nil, version = nil }, "tick", "observe", "skip-invalid-repo", "FKST_GITHUB_REPO is missing or invalid")
    return
  end

  local limits = liveness_scan_limits()
  local deadline = core.sweep_deadline(now(), limits)
  local poll_key = core.entity_list_poll_key(event)
  local issue_timeout = core.sweep_call_timeout(limits, deadline)
  if issue_timeout <= 0 then
    log_deferred("deadline", { entity_cap = limits.entity_cap })
    return
  end
  local issues = list_open_issues(repo, issue_timeout, poll_key)
  local pr_timeout = core.sweep_call_timeout(limits, deadline)
  if pr_timeout <= 0 then
    log_deferred("deadline", {
      listed_issues = #issues,
      entity_cap = limits.entity_cap,
      deferred = #issues,
    })
    return
  end
  local prs = list_open_prs(repo, pr_timeout, poll_key)
  local activations, deferred_by_cap, cursor_key, cursor, total = activation_slice(repo, issues, prs)
  local processed = 0
  local attempted = 0

  for _, activation in ipairs(activations) do
    if not core.sweep_has_budget(deadline) then
      update_liveness_cursor(cursor_key, cursor, total, attempted)
      log_deferred("deadline", {
        listed_issues = #issues,
        listed_prs = #prs,
        processed = processed,
        deferred = (#activations - processed) + deferred_by_cap,
        entity_cap = limits.entity_cap,
      })
      return
    end

    local should_reinject, defer_reason
    attempted = attempted + 1
    if activation.kind == "issue" then
      should_reinject, defer_reason = should_reinject_issue(repo, activation.entity, limits, deadline)
    elseif activation.kind == "pr" then
      should_reinject, defer_reason = should_reinject_pr(repo, activation.entity, limits, deadline)
    end
    if defer_reason == "deadline" then
      update_liveness_cursor(cursor_key, cursor, total, attempted)
      log_deferred("deadline", {
        listed_issues = #issues,
        listed_prs = #prs,
        processed = processed,
        deferred = (#activations - processed) + deferred_by_cap,
        entity_cap = limits.entity_cap,
      })
      return
    end
    processed = processed + 1
    if activation.kind == "issue" and should_reinject then
      reinject(repo, activation.entity, "issue", event and event.ts)
    elseif activation.kind == "pr" and should_reinject then
      reinject(repo, activation.entity, "pr", event and event.ts)
    end
  end

  update_liveness_cursor(cursor_key, cursor, total, attempted)

  if deferred_by_cap > 0 then
    log_deferred("cap", {
      listed_issues = #issues,
      listed_prs = #prs,
      processed = processed,
      deferred = deferred_by_cap,
      entity_cap = limits.entity_cap,
    })
  end
end

return saga.department(spec, {
  done = liveness_scan_done,
  act = act_liveness_scan,
  name = "liveness_scan",
})
