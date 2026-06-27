local S = {}

local ALLOWLIST_PATH = "migration/hidden-state.allowlist"
local BOT = "fkst-test-bot"
local REPO = "owner/repo"
local ISSUE_NUMBER = 42
local PR_NUMBER = 7
local ISSUE_PROPOSAL = "github-devloop/issue/owner/repo/42"
local PR_PROPOSAL = "github-devloop/pr/owner/repo/7"
local BRANCH = "devloop-owner-repo-42-01HY"
local BASE_BRANCH = "integration/dev"
local HEAD_SHA = "0123456789abcdef0123456789abcdef01234567"
local BASE_SHA = "fedcba9876543210fedcba9876543210fedcba98"
local REVIEW_PROPOSAL = "consensus-review-github-devloop/pr/owner/repo/7"
local REVIEW_DEDUP = "review-dedup-1"
local SOURCE_REF = { kind = "external", ref = "owner/repo#issue/42" }
local PR_SOURCE_REF = { kind = "external", ref = "owner/repo#pr/7" }

local function package_name(core)
  return tostring(core.restart_package_name or "github-devloop")
end

local function comment(body, when)
  return {
    id = tostring(when or body):gsub("[^%w_%-]", "_"):sub(1, 60),
    body = body,
    author_login = BOT,
    created_at = when or "2026-06-03T01:02:03Z",
  }
end

local function key(core, row, fact_family, successor)
  return table.concat({
    package_name(core),
    tostring(row.from_state or "?"),
    tostring(fact_family or "?"),
    tostring(successor or "?"),
  }, "|")
end

local function key_prefix(core, row)
  return table.concat({
    package_name(core),
    tostring(row.from_state or "?"),
    "",
  }, "|")
end

local function parse_allowlist_line(line)
  local text = tostring(line or "")
  if text == "" or text:match("^%s*#") then
    return nil
  end
  local parts = {}
  for part in text:gmatch("[^|]+") do
    table.insert(parts, part)
  end
  if #parts < 6 then
    return nil, "invalid hidden-state allowlist line: " .. text
  end
  if not tostring(parts[5]):match("^issue=#?%d+$") or tostring(parts[6]) == "why=" or not tostring(parts[6]):match("^why=") then
    return nil, "invalid hidden-state allowlist metadata: " .. text
  end
  return table.concat({ parts[1], parts[2], parts[3], parts[4] }, "|")
end

local function load_allowlist()
  local out = {}
  local ok, text = pcall(file.read, ALLOWLIST_PATH)
  if not ok then
    return out
  end
  for line in tostring(text or ""):gmatch("[^\n]+") do
    local parsed, err = parse_allowlist_line(line)
    if err ~= nil then
      table.insert(out, "__ERROR__|" .. err)
    elseif parsed ~= nil then
      out[parsed] = true
    end
  end
  return out
end

local function has_poll_surface(fact)
  local surfaces = fact.observe_surfaces or {}
  return surfaces.issue == true or surfaces.pr == true or surfaces.liveness_scan == true
end

local function row_has_declared_surface(row, fact)
  local row_surfaces = row.observe_surfaces or {}
  for surface, enabled in pairs(fact.observe_surfaces or {}) do
    if enabled == true and row_surfaces[surface] == true then
      return true
    end
  end
  return false
end

local function declared_by_key(core, rows)
  local declared = {}
  for _, row in ipairs(rows or {}) do
    for _, fact in ipairs(row.advancing_facts or {}) do
      declared[key(core, row, fact.fact_family, fact.successor)] = true
    end
  end
  return declared
end

local function global_advancing_fact_variants(rows)
  local variants = {}
  local seen = {}
  for _, row in ipairs(rows or {}) do
    for _, fact in ipairs(row.advancing_facts or {}) do
      local family = tostring(fact.fact_family or "")
      if family ~= "" then
        local variant_key = family .. "\0" .. tostring(fact.successor or "")
        if seen[variant_key] ~= true then
          seen[variant_key] = true
          table.insert(variants, fact)
        end
      end
    end
  end
  return variants
end

local function first_successor(row)
  for _, successor in ipairs(row.to_states or {}) do
    local value = tostring(successor or "")
    if value ~= "" and value ~= tostring(row.from_state or "") then
      return value
    end
  end
  return nil
end

local function remember_fact_family(by_family, ordered, declared, overwrite)
  local family = tostring((declared or {}).fact_family or "")
  if family == "" then
    return
  end
  if by_family[family] == nil then
    table.insert(ordered, family)
  end
  if overwrite == true or by_family[family] == nil then
    by_family[family] = declared
  end
end

local function required_fact_variant(row, required)
  local family = tostring((required or {}).family or "")
  if family == "" then
    return nil
  end
  return {
    fact_family = family,
    successor = first_successor(row),
    synthetic_required_fact = true,
  }
end

local function has_declared_advancing_facts(row)
  return type(row.advancing_facts) == "table" and #row.advancing_facts > 0
end

local function exemption_reason(row)
  local exemption = row.non_durable_advance
  if type(exemption) ~= "table" then
    return nil
  end
  -- Exempt only rows with no autonomous poll-derived durable-fact successor:
  -- pure operator-command reentry or terminal/recovery holds.
  local category = tostring(exemption.category or "")
  if category ~= "operator-reentry" and category ~= "terminal-hold" then
    return nil
  end
  local reason = tostring(exemption.reason or "")
  if reason == "" then
    return nil
  end
  return reason
end

local function has_allowlisted_row(core, allowlist, row)
  local prefix = key_prefix(core, row)
  for item in pairs(allowlist or {}) do
    if item:sub(1, #prefix) == prefix then
      return true
    end
  end
  return false
end

local function declaration_errors(core, rows, allowlist)
  local messages = {}
  local allowed_derivations = {
    ["source_ref:entity"] = true,
    ["source_ref:issue"] = true,
    ["source_ref:pr"] = true,
  }
  for _, row in ipairs(rows or {}) do
    local successors = {}
    for _, successor in ipairs(row.to_states or {}) do
      successors[successor] = true
    end
    if row.terminal ~= true
      and not has_declared_advancing_facts(row)
      and exemption_reason(row) == nil
      and not has_allowlisted_row(core, allowlist, row) then
      table.insert(messages, key_prefix(core, row) .. "*: non-terminal row must declare advancing_facts, non_durable_advance, or a shrink-only allowlist entry")
    end
    for _, fact in ipairs(row.advancing_facts or {}) do
      local label = key(core, row, fact.fact_family, fact.successor)
      if type(fact.fact_family) ~= "string" or fact.fact_family == "" then
        table.insert(messages, label .. ": advancing_facts entry must declare fact_family")
      end
      if type(fact.successor) ~= "string" or fact.successor == "" then
        table.insert(messages, label .. ": advancing_facts entry must declare successor")
      elseif successors[fact.successor] ~= true and tostring(fact.successor or "") ~= tostring(row.from_state or "") then
        table.insert(messages, label .. ": advancing_facts successor is not in to_states")
      end
      if type(fact.observe_surfaces) ~= "table" or next(fact.observe_surfaces) == nil then
        table.insert(messages, label .. ": advancing_facts entry must declare observe_surfaces")
      elseif not row_has_declared_surface(row, fact) then
        table.insert(messages, label .. ": advancing_facts observe_surfaces are not declared on row")
      elseif not has_poll_surface(fact) then
        table.insert(messages, label .. ": advancing fact must be re-derivable on a poll observe surface")
      end
      if allowed_derivations[tostring(fact.source_ref_derivation or "")] ~= true then
        table.insert(messages, label .. ": advancing_facts entry must declare source_ref_derivation")
      end
    end
  end
  local declared = declared_by_key(core, rows)
  local current_package_prefix = package_name(core) .. "|"
  for item in pairs(allowlist or {}) do
    if item:match("^__ERROR__|") then
      table.insert(messages, item:gsub("^__ERROR__|", ""))
    elseif item:sub(1, #current_package_prefix) == current_package_prefix and declared[item] == nil then
      table.insert(messages, item .. ": hidden-state allowlist entry has no matching advancing_facts row")
    end
  end
  return messages
end

local function with_effect_capture(core, fn)
  local events = {
    decisions = {},
    raises = {},
    applies = {},
  }
  local previous_decision = core.log_cas_decision
  local previous_raise = core.log_raise
  local previous_apply = core.log_apply
  core.log_cas_decision = function(dept, proposal_id, state, from_state, to_state, outcome, reason)
    table.insert(events.decisions, {
      dept = dept,
      proposal_id = proposal_id,
      state = state,
      from_state = from_state,
      to_state = to_state,
      outcome = outcome,
      reason = reason,
    })
  end
  core.log_raise = function(dept, proposal_id, queue, payload)
    table.insert(events.raises, {
      dept = dept,
      proposal_id = proposal_id,
      queue = queue,
      payload = payload,
    })
  end
  core.log_apply = function(dept, proposal_id, apply_state, version, label_changes, queues)
    table.insert(events.applies, {
      dept = dept,
      proposal_id = proposal_id,
      apply_state = apply_state,
      version = version,
      label_changes = label_changes,
      queues = queues,
    })
  end
  local ok, result = pcall(fn)
  core.log_cas_decision = previous_decision
  core.log_raise = previous_raise
  core.log_apply = previous_apply
  if not ok then
    error(result)
  end
  return result, events
end

local function source_ref_for(core, derivation)
  if derivation == "source_ref:pr" then
    return PR_SOURCE_REF
  end
  return SOURCE_REF
end

local function row_source_ref(core, row)
  if package_name(core) == "github-devloop-pr" then
    return PR_SOURCE_REF
  end
  for _, fact in ipairs(row.advancing_facts or {}) do
    if fact.source_ref_derivation == "source_ref:pr" then
      return PR_SOURCE_REF
    end
  end
  return SOURCE_REF
end

local function base_version(row)
  if tostring(row.from_state or "") == "impl-failed" then
    return "ready/behavioral/2026-06-03T01-02-03Z"
  end
  return tostring(row.from_state) .. "/behavioral/2026-06-03T01-02-03Z"
end

local function state_for(row)
  return {
    state = row.from_state,
    version = base_version(row),
    proposal_id = ISSUE_PROPOSAL,
    marker_created_at = "2026-06-03T01:02:03Z",
  }
end

local function base_entity(core, row, source_ref)
  local state = state_for(row)
  local body = core.state_marker(ISSUE_PROPOSAL, row.from_state, state.version, "result-marker,ready-label,devloop-ready")
  local labels = { "fkst-dev:enabled", core.state_label(row.from_state) }
  return {
    schema = "github-proxy.v1",
    type = "issue",
    repo = REPO,
    number = ISSUE_NUMBER,
    title = "Behavioral hidden-state conformance",
    body = "",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    labels = labels,
    comments = { comment(body, "2026-06-03T01:02:03Z") },
    source_ref = source_ref,
  }, state
end

local function child_pr(core, state, child_state)
  local body = core.pr_origin_marker(ISSUE_PROPOSAL, ISSUE_NUMBER, BRANCH, state.version, BASE_BRANCH)
  if child_state ~= nil then
    body = body .. "\n" .. core.state_marker(PR_PROPOSAL, child_state, state.version)
  end
  if child_state == "merged" then
    body = body .. "\n" .. core.merged_marker(PR_PROPOSAL, PR_NUMBER, state.version, HEAD_SHA)
  end
  return {
    repo = REPO,
    number = PR_NUMBER,
    state = child_state == "merged" and "MERGED" or "OPEN",
    head_ref_name = BRANCH,
    base_ref_name = BASE_BRANCH,
    head_sha = HEAD_SHA,
    merged_at = child_state == "merged" and "2026-06-03T01:04:03Z" or nil,
    comments = { comment(body, "2026-06-03T01:04:03Z") },
  }
end

local function add_common_pr_facts(core, entity, state, facts)
  local link = {
    proposal_id = ISSUE_PROPOSAL,
    pr_number = PR_NUMBER,
    branch = BRANCH,
    impl_version = state.version,
    base_branch = BASE_BRANCH,
  }
  facts.link = link
  facts["pr-link"] = link
  facts.current_pr = facts.current_pr or child_pr(core, state, nil)
  table.insert(entity.comments, comment(core.pr_link_marker(ISSUE_PROPOSAL, PR_NUMBER, BRANCH, state.version, BASE_BRANCH), "2026-06-03T01:03:03Z"))
  facts.snapshot.prs = {
    { number = PR_NUMBER, current = facts.current_pr },
  }
end

local function fact_value(core, row, state, family, successor)
  if family == "dependency-gate" then
    if successor == "ready" or successor == "implementing" then
      return { ok = true, kind = "satisfied", reason = "all-blockers-closed", unmet = {} }
    end
    if successor == "blocked" then
      return { ok = false, kind = "unresolvable", reason = "dependency-gate-stale", unmet = { 99 } }
    end
    return { ok = false, kind = "waiting", reason = "waiting-on-dependency", unmet = { 99 } }
  end
  if family == "dependency-wait" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      version = state.version,
      hold_kind = "waiting",
      reason = "waiting-on-dependency",
      unmet = { 99 },
    }
  end
  if family == "dependency-release" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      version = state.version,
    }
  end
  if family == "implement-attempt" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      dedup_key = state.version,
      attempt = 1,
      started_at = "2026-06-03T01:03:03Z",
    }
  end
  if family == "implementing" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      dedup_key = state.version,
      branch = BRANCH,
      head_sha = HEAD_SHA,
      base_branch = BASE_BRANCH,
      base_sha = BASE_SHA,
    }
  end
  if family == "impl-failure" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      dedup_key = state.version,
      reason = "codex-failed",
      attempt = 1,
    }
  end
  if family == "child-state" then
    return {
      proposal_id = PR_PROPOSAL,
      state = successor,
      version = state.version,
    }
  end
  if family == "canonical-child-pr-merged" then
    return {
      proposal_id = PR_PROPOSAL,
      state = "merged",
      version = state.version,
      head_sha = HEAD_SHA,
    }
  end
  if family == "decomposed" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      version = state.version,
      pr_number = PR_NUMBER,
      count = 1,
    }
  end
  if family == "fix-feedback" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      version = state.version,
      pr_number = PR_NUMBER,
      review_proposal_id = REVIEW_PROPOSAL,
      review_dedup_key = REVIEW_DEDUP,
      reviewed_head_sha = HEAD_SHA,
      reason = "behavioral-fixture",
    }
  end
  if family == "review-result" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      pr_number = PR_NUMBER,
      review_proposal_id = REVIEW_PROPOSAL,
      review_dedup_key = REVIEW_DEDUP,
      reviewed_head_sha = HEAD_SHA,
      decision = successor == "merge-ready" and "approve" or "reject",
      blocking_gap = "behavioral-fixture",
    }
  end
  if family == "review-meta" or family == "review-converge-round" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      pr_number = PR_NUMBER,
      review_proposal_id = REVIEW_PROPOSAL,
      review_dedup_key = REVIEW_DEDUP,
      reviewed_head_sha = HEAD_SHA,
      version = state.version,
      n = 3,
      action = successor == "fixing" and "fix" or "block",
      blocking_gap = "behavioral-fixture",
    }
  end
  if family == "merge-ready" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      pr_number = PR_NUMBER,
      version = state.version,
      review_proposal_id = REVIEW_PROPOSAL,
      review_dedup_key = REVIEW_DEDUP,
      head_sha = HEAD_SHA,
    }
  end
  if family == "merging" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      pr_number = PR_NUMBER,
      version = state.version,
      head_sha = HEAD_SHA,
    }
  end
  if family == "decompose-children" then
    return {}
  end
  if family == "converge-round" then
    return {
      proposal_id = ISSUE_PROPOSAL,
      base_version = state.version,
      round = 3,
      dedup = state.version .. "/loop/3",
      narrowed_question = "behavioral fixture narrowed question",
      angle_digests = { "a", "b", "c" },
    }
  end
  if family == "state" then
    return state
  end
  return nil
end

local function store_fact_value(facts, family, value)
  facts[family] = value
  facts[tostring(family):gsub("%-", "_")] = value
  if family == "pr-link" then
    facts.link = value
  elseif family == "pr-delegation" then
    facts.pr_delegation = value
  elseif family == "child-state" then
    facts.child_state = value
  elseif family == "fix-feedback" then
    facts.fix_feedback = value
    facts.feedback = facts.feedback or value
  elseif family == "review-result" or family == "merge-gate" then
    facts.feedback = facts.feedback or value
  elseif family == "review-meta" or family == "review-converge-round" then
    facts.review_meta = value
    facts.feedback = facts.feedback or value
  elseif family == "merge-ready" then
    facts.merge_ready = value
  elseif family == "impl-failure" then
    facts.impl_failure = value
  end
end

local function install_marker(core, entity, state, family, value, is_synthetic)
  if family == "dependency-wait" then
    table.insert(entity.comments, comment(core.dependency_wait_marker(ISSUE_PROPOSAL, state.version, value.unmet or {}, value.hold_kind, value.reason), "2026-06-03T01:03:04Z"))
  elseif family == "dependency-release" then
    table.insert(entity.comments, comment(core.dependency_release_marker(ISSUE_PROPOSAL, state.version), "2026-06-03T01:03:05Z"))
  elseif family == "implement-attempt" then
    table.insert(entity.comments, comment(core.implement_attempt_marker(ISSUE_PROPOSAL, state.version, value.attempt, value.started_at), "2026-06-03T01:03:06Z"))
  elseif family == "implementing" then
    table.insert(entity.comments, comment(core.implementing_marker(ISSUE_PROPOSAL, state.version, BRANCH, HEAD_SHA, BASE_BRANCH, BASE_SHA), "2026-06-03T01:03:07Z"))
  elseif family == "impl-failure" then
    table.insert(entity.comments, comment(core.impl_failure_marker(ISSUE_PROPOSAL, state.version, value.reason or "codex-failed", value.attempt or 1), "2026-06-03T01:03:07Z"))
  elseif family == "decomposed" then
    table.insert(entity.comments, comment(core.decomposed_marker(ISSUE_PROPOSAL, state.version, PR_NUMBER, value.count or 1), "2026-06-03T01:03:08Z"))
  elseif family == "fix-feedback" then
    table.insert(entity.comments, comment(core.fix_marker(ISSUE_PROPOSAL, REVIEW_PROPOSAL, REVIEW_DEDUP, HEAD_SHA, HEAD_SHA), "2026-06-03T01:03:09Z"))
  elseif family == "review-result" then
    table.insert(entity.comments, comment(core.review_result_marker(REVIEW_PROPOSAL, ISSUE_PROPOSAL, value.decision or "reject", REVIEW_DEDUP, 1, value.blocking_gap or "behavioral-fixture"), "2026-06-03T01:03:09Z"))
    if value.decision == "approve" then
      table.insert(entity.comments, comment(core.merge_ready_marker(ISSUE_PROPOSAL, PR_NUMBER, state.version, REVIEW_PROPOSAL, REVIEW_DEDUP, HEAD_SHA), "2026-06-03T01:03:09Z"))
    else
      table.insert(entity.comments, comment(core.merge_gate_marker(ISSUE_PROPOSAL, PR_NUMBER, state.version, REVIEW_PROPOSAL, REVIEW_DEDUP, HEAD_SHA, BASE_SHA, value.blocking_gap or "behavioral-fixture"), "2026-06-03T01:03:09Z"))
    end
  elseif family == "review-meta" or family == "review-converge-round" then
    table.insert(entity.comments, comment(core.review_meta_marker(ISSUE_PROPOSAL, REVIEW_DEDUP, value.action, state.version, value.blocking_gap or "behavioral-fixture"), "2026-06-03T01:03:09Z"))
  elseif family == "merge-ready" then
    table.insert(entity.comments, comment(core.merge_ready_marker(ISSUE_PROPOSAL, PR_NUMBER, state.version, REVIEW_PROPOSAL, REVIEW_DEDUP, HEAD_SHA), "2026-06-03T01:03:09Z"))
  elseif family == "merging" then
    table.insert(entity.comments, comment(core.merging_marker(ISSUE_PROPOSAL, PR_NUMBER, state.version, HEAD_SHA), "2026-06-03T01:03:09Z"))
  elseif family == "converge-round" then
    table.insert(entity.comments, comment(core.converge_round_marker(ISSUE_PROPOSAL, state.version, core.source_ref_digest(SOURCE_REF), value.round, value.dedup, value.narrowed_question, value.angle_digests), "2026-06-03T01:03:10Z"))
  elseif is_synthetic == true then
    table.insert(entity.comments, comment('<!-- fkst:github-devloop:synthetic-visible-fact:v1 proposal="' .. ISSUE_PROPOSAL
      .. '" family="' .. tostring(family):gsub('"', "'")
      .. '" version="' .. tostring(state.version):gsub('"', "'")
      .. '" -->', "2026-06-03T01:03:11Z"))
  end
end

local function add_context_facts(core, row, entity, state, facts, source_ref, include_fact, declared)
  if package_name(core) == "github-devloop-pr" or source_ref == PR_SOURCE_REF then
    add_common_pr_facts(core, entity, state, facts)
    entity.type = "pr"
    entity.number = PR_NUMBER
    facts.current_pr = facts.current_pr or entity
    facts.current = entity
    facts.snapshot.comments = entity.comments
  elseif row.from_state == "awaiting-pr" then
    local child_state = include_fact and declared.successor or nil
    facts.current_pr = child_pr(core, state, child_state)
    table.insert(entity.comments, comment(core.pr_delegation_marker(ISSUE_PROPOSAL, PR_PROPOSAL, PR_NUMBER, state.version, "g1"), "2026-06-03T01:03:03Z"))
    facts.pr_delegation = {
      proposal_id = ISSUE_PROPOSAL,
      child = PR_PROPOSAL,
      pr_number = PR_NUMBER,
      version = state.version,
      delegation = "g1",
    }
    facts["pr-delegation"] = facts.pr_delegation
    facts.snapshot.prs = {
      { number = PR_NUMBER, current = facts.current_pr },
    }
  elseif row.from_state == "blocked" then
    add_common_pr_facts(core, entity, state, facts)
  end
end

local function build_fixture_base(core, row, source_ref)
  local entity, state = base_entity(core, row, source_ref)
  local facts = {
    proposal_id = ISSUE_PROPOSAL,
    source_ref = source_ref,
    event_ts = "2026-06-03T01:05:00Z",
    now_seconds = core.iso_timestamp_epoch_seconds("2026-06-03T01:05:00Z"),
  }
  facts.current = entity
  facts.current_issue = entity
  facts.snapshot = { comments = entity.comments, prs = {}, state = state }
  return entity, state, facts
end

local function build_fixture(core, row, declared, include_fact)
  local source_ref = source_ref_for(core, declared.source_ref_derivation)
  local entity, state, facts = build_fixture_base(core, row, source_ref)
  add_context_facts(core, row, entity, state, facts, source_ref, include_fact, declared)

  if include_fact then
    local value = fact_value(core, row, state, declared.fact_family, declared.successor)
    if value ~= nil then
      store_fact_value(facts, declared.fact_family, value)
      install_marker(core, entity, state, declared.fact_family, value)
    end
  end
  return entity, state, facts
end

local function build_exemption_fixture(core, row, rows, focus)
  local source_ref = row_source_ref(core, row)
  local entity, state, facts = build_fixture_base(core, row, source_ref)
  add_context_facts(core, row, entity, state, facts, source_ref, true, focus)

  local by_family = {}
  local ordered = {}
  for _, declared in ipairs(global_advancing_fact_variants(rows)) do
    remember_fact_family(by_family, ordered, declared, true)
  end
  for _, required in ipairs(row.required_facts or {}) do
    remember_fact_family(by_family, ordered, required_fact_variant(row, required), false)
  end
  if focus ~= nil and by_family[tostring(focus.fact_family or "")] ~= nil then
    by_family[tostring(focus.fact_family or "")] = focus
  end

  for _, family in ipairs(ordered) do
    local declared = by_family[family]
    local value = fact_value(core, row, state, family, declared.successor)
    if value == nil and declared.synthetic_required_fact == true then
      value = {
        proposal_id = ISSUE_PROPOSAL,
        version = state.version,
        family = family,
        synthetic_visible_fact = true,
      }
    end
    if value ~= nil then
      store_fact_value(facts, family, value)
      install_marker(core, entity, state, family, value, declared.synthetic_required_fact == true)
      if family == "canonical-child-pr-merged" then
        facts.current_pr = child_pr(core, state, "merged")
        facts.snapshot.prs = {
          { number = PR_NUMBER, current = facts.current_pr },
        }
      end
    end
  end
  return entity, state, facts
end

local function advanced_to(events, from_state, successor)
  local saw_effect = false
  for _, apply in ipairs(events.applies or {}) do
    if tostring(apply.apply_state or "") == tostring(successor or "") then
      return true
    end
    for _, queue in ipairs(apply.queues or {}) do
      if tostring(queue or "") ~= "" then
        saw_effect = true
      end
    end
  end
  for _, decision in ipairs(events.decisions or {}) do
    if tostring(decision.to_state or "") == tostring(successor or "") then
      local outcome = tostring(decision.outcome or "")
      if outcome:find("applied", 1, true) ~= nil or outcome:find("release", 1, true) ~= nil or outcome:find("hold", 1, true) ~= nil then
        return true
      end
    end
  end
  if saw_effect and tostring(successor or "") == tostring(from_state or "") then
    return true
  end
  return false
end

local function successor_states(row)
  local successors = {}
  for _, state in ipairs(row.to_states or {}) do
    local value = tostring(state or "")
    if value ~= "" and value ~= tostring(row.from_state or "") then
      successors[value] = true
    end
  end
  return successors
end

local function advanced_to_successor_state(events, row)
  local successors = successor_states(row)
  for _, apply in ipairs((events or {}).applies or {}) do
    local to_state = tostring(apply.apply_state or "")
    if successors[to_state] == true then
      return true, to_state
    end
  end
  for _, decision in ipairs((events or {}).decisions or {}) do
    local to_state = tostring(decision.to_state or "")
    if successors[to_state] == true then
      local outcome = tostring(decision.outcome or "")
      if outcome:find("applied", 1, true) ~= nil
        or outcome:find("apply", 1, true) ~= nil
        or outcome:find("release", 1, true) ~= nil
        or outcome:find("hold", 1, true) ~= nil then
        return true, to_state
      end
    end
  end
  return false, nil
end

local function replay(core, row, declared, include_fact)
  local entity, state, facts = build_fixture(core, row, declared, include_fact)
  local issued, events = with_effect_capture(core, function()
    return core.replay_from_table("behavioral_hidden_state_conformance", entity, state, row, facts)
  end)
  return issued, events
end

local function replay_exemption(core, row, rows, focus)
  local entity, state, facts = build_exemption_fixture(core, row, rows, focus)
  local issued, events = with_effect_capture(core, function()
    return core.replay_from_table("behavioral_hidden_state_conformance", entity, state, row, facts)
  end)
  return issued, events
end

local function with_poll_fakes(core, fn)
  local previous_children = core.gh_issue_list_decompose_children
  if type(previous_children) == "function" then
    core.gh_issue_list_decompose_children = function()
      return { exit_code = 0, stdout = "[]", stderr = "" }
    end
  end
  local ok, first, second = pcall(fn)
  if type(previous_children) == "function" then
    core.gh_issue_list_decompose_children = previous_children
  end
  if not ok then
    error(first)
  end
  return first, second
end

local function exemption_behavior_errors(core, rows, row)
  local messages = {}
  local variants = global_advancing_fact_variants(rows)
  if #variants == 0 then
    variants = { {} }
  end
  for _, focus in ipairs(variants) do
    local ok, issued, events = pcall(function()
      return with_poll_fakes(core, function()
        return replay_exemption(core, row, rows, focus)
      end)
    end)
    if not ok then
      table.insert(messages, key_prefix(core, row) .. "*: non_durable_advance all-facts poll fixture errored: " .. tostring(issued))
    else
      local advanced, successor = advanced_to_successor_state(events, row)
      if issued == true and advanced then
        table.insert(messages, key_prefix(core, row) .. "*: non_durable_advance exemption advanced to successor " .. tostring(successor) .. " with all durable fact families present")
      end
    end
  end
  return messages
end

local function behavioral_errors(core, rows, allowlist)
  local messages = {}
  for _, row in ipairs(rows or {}) do
    if row.terminal ~= true then
      for _, declared in ipairs(row.advancing_facts or {}) do
        local label = key(core, row, declared.fact_family, declared.successor)
        local prefix = key_prefix(core, row)
        local row_allowlisted = allowlist[label] == true
        for allowed in pairs(allowlist) do
          if allowed:sub(1, #prefix) == prefix then
            row_allowlisted = row_allowlisted or allowed == label
          end
        end
        local ok, issued, events = pcall(function()
          return replay(core, row, declared, true)
        end)
        local passes_positive = ok and issued == true and advanced_to(events, row.from_state, declared.successor)
        local positive_message = nil
        if not passes_positive then
          positive_message = label .. ": positive poll fixture did not advance to declared successor"
        end
        ok, issued, events = pcall(function()
          return replay(core, row, declared, false)
        end)
        local passes_negative = not (ok and issued == true and advanced_to(events, row.from_state, declared.successor))
        local negative_message = nil
        if ok and issued == true and advanced_to(events, row.from_state, declared.successor) then
          negative_message = label .. ": negative poll fixture advanced without declared fact"
        end
        if row_allowlisted then
          if passes_positive and passes_negative then
            table.insert(messages, label .. ": hidden-state allowlist entry is now passing; remove it")
          end
        else
          if positive_message ~= nil then
            table.insert(messages, positive_message)
          end
          if negative_message ~= nil then
            table.insert(messages, negative_message)
          end
        end
      end
      if exemption_reason(row) ~= nil then
        for _, message in ipairs(exemption_behavior_errors(core, rows, row)) do
          table.insert(messages, message)
        end
      end
    end
  end
  return messages
end

function S.errors(core, rows, allowlist)
  local effective_rows = rows or core.restart_transition_table()
  local effective_allowlist = allowlist or load_allowlist()
  local messages = declaration_errors(core, effective_rows, effective_allowlist)
  for _, message in ipairs(behavioral_errors(core, effective_rows, effective_allowlist)) do
    table.insert(messages, message)
  end
  table.sort(messages)
  return messages
end

function S.fixture(core, row, declared, include_fact)
  return build_fixture(core, row, declared, include_fact)
end

function S.install(M)
  function M.hidden_state_conformance_errors(rows, allowlist)
    return S.errors(M, rows, allowlist)
  end

  function M.hidden_state_behavior_fixture(row, declared, include_fact)
    return S.fixture(M, row, declared, include_fact)
  end
end

return S
