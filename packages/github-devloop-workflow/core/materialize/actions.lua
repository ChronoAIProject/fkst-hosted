local base_ids = require("devloop.base_ids")
local devloop_base = require("devloop.base")
local devloop_claims = require("devloop.claims")
local devloop_entity = require("devloop.entity")
local devloop_logging = require("devloop.logging")
local marker = require("core.marker")
local materialization = require("core.materialization")
local parsers_misc = require("devloop.parsers.misc")
local strings = require("contract.strings")

local M = {}

M.DEPT = "workflow_materialize_next"

local function safe_source_ref(repo, issue_number)
  return devloop_entity.issue_source_ref(repo, issue_number)
end

local function attr(text, name)
  return tostring(text or ""):match(name .. '="([^"]*)"')
end

local function issue_author_login(issue)
  local login = devloop_claims.issue_author_login(issue)
  return devloop_base.strip_bot_login_suffix(login)
end

local function issue_create_marker(child_dedup)
  return "<!-- fkst:github-proxy:issue-create:" .. tostring(child_dedup) .. " -->"
end

local function strip_after_marker(body, marker_text)
  local start_at = tostring(body or ""):find(marker_text, 1, true)
  if start_at == nil then
    return nil
  end
  return tostring(body or ""):sub(1, start_at - 1):gsub("%s+$", "")
end

local function strip_lineage_header(body, origin, blueprint_digest, slot_id)
  local lineage = marker.parse_lineage_header(body)
  if lineage == nil
    or lineage.origin ~= tostring(origin)
    or lineage.blueprint_digest ~= tostring(blueprint_digest)
    or lineage.slot ~= tostring(slot_id) then
    return nil
  end
  local stripped = tostring(body or ""):gsub("^%s*<!%-%- fkst:github%-devloop%-workflow:lineage:v1.-%-%->%s*", "", 1)
  return stripped:gsub("^%s+", ""):gsub("%s+$", "")
end

local function issue_number_or_nil(value)
  local number = tonumber(value)
  if number == nil or number <= 0 then
    return nil
  end
  return tostring(math.floor(number))
end

function M.source_ref_digest(source_ref)
  if type(source_ref) ~= "table" then
    return materialization.EMPTY_PREDECESSOR_REF_DIGEST
  end
  return "d-" .. strings.decimal_checksum(tostring(source_ref.kind or "") .. "\n" .. tostring(source_ref.ref or ""))
end

function M.predecessor_ref_digest(predecessor)
  if predecessor == nil then
    return materialization.EMPTY_PREDECESSOR_REF_DIGEST
  end
  -- The predecessor identity is the stable source_ref; result content is rehydrated by source_ref, not hashed into this CAS key component.
  return M.source_ref_digest(predecessor.source_ref)
end

function M.child_ref_for_entry(repo, entry)
  local issue_number = entry and entry.child_issue
  if issue_number == nil then
    return nil
  end
  return {
    kind = "issue",
    repo = repo,
    issue_number = tostring(issue_number),
    proposal_id = base_ids.proposal_id(repo, issue_number),
    source_ref = safe_source_ref(repo, issue_number),
  }
end

function M.ledger_for_frontier(repo, facts)
  local by_slot = marker.latest_materialization_by_slot(facts)
  for _, entry in pairs(by_slot) do
    if type(entry) == "table" and entry.state == "created" and entry.child_issue ~= nil then
      entry.child_ref = M.child_ref_for_entry(repo, entry)
      entry.child_proposal_id = entry.child_ref.proposal_id
      entry.child_source_ref = entry.child_ref.source_ref
    end
  end
  return by_slot
end

function M.find_step(plan, slot_id)
  for _, step in ipairs(plan and plan.steps or {}) do
    if tostring(step.id) == tostring(slot_id) then
      return step
    end
  end
  return nil
end

local function build_comment_request(repo, issue_number, origin, body, dedup_components)
  -- dedup_components is an array of deterministic string parts (slot, digests,
  -- state, ...). Spread them into the key so the dedup_key is deterministic;
  -- tostring()-ing the whole table would collapse it to a Lua address.
  local key_parts = { "workflow", "comment", tostring(origin) }
  for _, part in ipairs(dedup_components) do
    key_parts[#key_parts + 1] = tostring(part)
  end
  return {
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = tonumber(issue_number),
    body = body,
    dedup_key = base_ids.dedup_key(key_parts),
    source_ref = safe_source_ref(repo, issue_number),
  }
end

local function visible_materialization_line(entry, state, child_issue)
  local slot = tostring(entry and entry.slot or "unknown")
  if state == "created" then
    if child_issue ~= nil and tostring(child_issue) ~= "" then
      return "Materialized the `" .. slot .. "` step as sub-issue #" .. tostring(child_issue) .. "."
    end
    return "Materialized the `" .. slot .. "` step as a sub-issue."
  end
  if state == "generated" then
    return "Generated the `" .. slot .. "` step for materialization."
  end
  return "Recorded the `" .. slot .. "` materialization as " .. tostring(state) .. "."
end

local function visible_terminal_line(state, reason_code)
  if state == "done" then
    return "Workflow complete: every step merged."
  end
  if state == "blocked" then
    return "Workflow blocked: " .. tostring(reason_code) .. "."
  end
  return "Workflow errored: " .. tostring(reason_code) .. "."
end

function M.materialization_marker_body(origin, entry, state, child_issue)
  local built, err = marker.build_materialization_marker(
    origin,
    entry.blueprint_digest,
    entry.slot,
    entry.predecessor_ref_digest,
    entry.gen_contract_digest,
    entry.gen_spec_digest,
    entry.child_dedup,
    child_issue,
    state
  )
  if built == nil then
    error("github-devloop-workflow: materialization-marker-build-failed: materialization marker build failed: " .. tostring(err and err.code or "unknown"))
  end
  return visible_materialization_line(entry, state, child_issue) .. "\n\n" .. built
end

function M.terminal_request(repo, issue_number, origin, state, reason_code)
  local built, err = marker.build_terminal_marker(origin, state, reason_code)
  if built == nil then
    error("github-devloop-workflow: terminal-marker-build-failed: terminal marker build failed: " .. tostring(err and err.code or "unknown"))
  end
  local body = visible_terminal_line(state, reason_code) .. "\n\n" .. built
  return build_comment_request(repo, issue_number, origin, body, {
    "terminal",
    tostring(state),
    tostring(reason_code),
  })
end

function M.materialization_comment_request(repo, issue_number, origin, entry, state, child_issue)
  return build_comment_request(
    repo,
    issue_number,
    origin,
    M.materialization_marker_body(origin, entry, state, child_issue),
    {
      "materialization",
      tostring(entry.slot),
      tostring(entry.predecessor_ref_digest),
      tostring(entry.gen_spec_digest),
      tostring(state),
      tostring(child_issue or ""),
    }
  )
end

local function workflow_step_source_ref(repo, origin_issue_number, slot_id)
  return {
    kind = "external",
    ref = tostring(repo) .. "#workflow-step/" .. tostring(origin_issue_number) .. "/" .. tostring(slot_id),
  }
end

function M.issue_create_request(repo, issue_number, origin, blueprint_digest, slot_id, entry, generated_spec)
  local lineage, err = marker.build_lineage_header(origin, blueprint_digest, slot_id)
  if lineage == nil then
    error("github-devloop-workflow: lineage-marker-build-failed: lineage marker build failed: " .. tostring(err and err.code or "unknown"))
  end
  return {
    schema = "github-proxy.issue-create.v1",
    repo = repo,
    title = generated_spec.title,
    body = lineage .. "\n\n" .. generated_spec.body,
    dedup_key = entry.child_dedup,
    source_ref = workflow_step_source_ref(repo, issue_number, slot_id),
    parent = tonumber(issue_number),
    parent_comment_target = {
      repo = repo,
      issue_number = tonumber(issue_number),
    },
  }
end

function M.raise_request(proposal_id, queue, request)
  devloop_logging.log_raise(M.DEPT, proposal_id, queue, request)
end

function M.trusted_issue_created_number(core, current, child_dedup_key, trusted_comments)
  local pattern = "<!%-%- fkst:github%-proxy:issue%-created:v1.-%-%->"
  for _, comment in ipairs(trusted_comments(core, current and current.comments)) do
    for found in parsers_misc.comment_body(comment):gmatch(pattern) do
      if attr(found, "dedup") == tostring(child_dedup_key) then
        local issue = attr(found, "issue")
        if issue ~= nil and tostring(issue):match("^%d+$") and tonumber(issue) > 0 then
          return tostring(math.floor(tonumber(issue)))
        end
      end
    end
  end
  return nil
end

function M.has_trusted_issue_create_intent(core, current, child_dedup_key, trusted_comments)
  local pattern = "<!%-%- fkst:github%-proxy:issue%-create%-intent:v1.-%-%->"
  for _, comment in ipairs(trusted_comments(core, current and current.comments)) do
    for found in parsers_misc.comment_body(comment):gmatch(pattern) do
      if attr(found, "dedup") == tostring(child_dedup_key) then
        return true
      end
    end
  end
  return false
end

function M.spec_from_created_issue(issue, origin, blueprint_digest, slot_id, child_dedup)
  if type(issue) ~= "table" then
    return nil
  end
  local title = issue.title
  local body = issue.body
  if title == nil or body == nil then
    return nil
  end
  local before_proxy = strip_after_marker(body, issue_create_marker(child_dedup))
  if before_proxy == nil then
    return nil
  end
  local spec_body = strip_lineage_header(before_proxy, origin, blueprint_digest, slot_id)
  if spec_body == nil then
    return nil
  end
  return {
    title = tostring(title),
    body = spec_body,
  }
end

local function searched_issue_number(issue)
  return issue_number_or_nil(type(issue) == "table" and issue.number or nil)
end

function M.find_created_issue_by_dedup(repo, child_dedup, deps)
  if type(deps) == "table" and type(deps.search_created_issue) == "function" then
    return deps.search_created_issue(repo, child_dedup)
  end
  if type(exec_argv) ~= "function" then
    return nil
  end
  local result = require("forge.github").new(exec_argv).issue_search(
    repo,
    issue_create_marker(child_dedup),
    "number,title,state,author,body,url",
    30
  )
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("github-devloop-workflow: materialization-child-search-failed: child issue search failed: " .. tostring(result and result.stderr or "nil result"))
  end
  local ok, decoded = pcall(json.decode, result.stdout or "[]")
  if not ok or type(decoded) ~= "table" then
    error("github-devloop-workflow: materialization-child-search-malformed: child issue search returned malformed JSON")
  end
  local trusted = devloop_base.trusted_bot_login()
  for _, issue in ipairs(decoded) do
    local number = searched_issue_number(issue)
    if number ~= nil
      and issue_author_login(issue) == trusted
      and tostring(issue.body or ""):find(issue_create_marker(child_dedup), 1, true) ~= nil then
      return {
        number = number,
        title = issue.title,
        body = issue.body,
        state = issue.state,
        author_login = issue_author_login(issue),
        url = issue.url,
      }
    end
  end
  return nil
end

function M.read_created_issue_by_number(repo, issue_number, deps)
  local number = issue_number_or_nil(issue_number)
  if number == nil then
    return nil
  end
  if type(deps) == "table" and type(deps.read_created_issue) == "function" then
    return deps.read_created_issue(repo, number)
  end
  if type(exec_argv) ~= "function" then
    return nil
  end
  local result = require("forge.github").new(exec_argv).issue_view(
    repo,
    number,
    "number,title,state,author,body,url",
    30
  )
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("github-devloop-workflow: materialization-child-view-failed: child issue view failed: " .. tostring(result and result.stderr or "nil result"))
  end
  local ok, decoded = pcall(json.decode, result.stdout or "{}")
  if not ok or type(decoded) ~= "table" then
    error("github-devloop-workflow: materialization-child-view-malformed: child issue view returned malformed JSON")
  end
  return {
    number = searched_issue_number(decoded) or number,
    title = decoded.title,
    body = decoded.body,
    state = decoded.state,
    author_login = issue_author_login(decoded),
    url = decoded.url,
  }
end

function M.created_entry_from_issue(origin, blueprint_digest, slot, predecessor_ref_digest, child_dedup, issue)
  local spec = M.spec_from_created_issue(issue, origin, blueprint_digest, slot.id or slot, child_dedup)
  if spec == nil then
    return nil
  end
  local entry = materialization.created_entry(origin, blueprint_digest, slot, predecessor_ref_digest, spec, issue.number)
  if entry == nil or entry.child_dedup ~= tostring(child_dedup) then
    return nil
  end
  return entry
end

local function generated_fact_for_child(facts, child_dedup)
  for _, fact in ipairs(facts or {}) do
    if fact.state == "generated" and tostring(fact.child_dedup or "") == tostring(child_dedup or "") then
      return fact
    end
  end
  return nil
end

function M.record_existing_child_or_created_marker(core, deps, repo, issue_number, origin, blueprint_digest, slot, predecessor_ref_digest, child_dedup, facts, current, trusted_comments, log_decision)
  local child_issue = M.trusted_issue_created_number(core, current, child_dedup, trusted_comments)
  local generated_fact = generated_fact_for_child(facts, child_dedup)
  local found = nil
  local source = nil
  if child_issue ~= nil then
    if generated_fact ~= nil then
      found = { number = child_issue }
    else
      found = M.read_created_issue_by_number(repo, child_issue, deps)
    end
    source = "parent-created-marker"
  else
    found = M.find_created_issue_by_dedup(repo, child_dedup, deps)
    source = "existing-child-search"
  end
  if found == nil then
    if M.has_trusted_issue_create_intent(core, current, child_dedup, trusted_comments) then
      log_decision(origin, "materialization", "created", "skip-existing-child-inflight", "trusted github-proxy issue-create intent marker is visible")
      return "wait", nil
    end
    if child_issue == nil then
      return false, nil
    end
    found = { number = child_issue }
  end
  local created_entry = M.created_entry_from_issue(origin, blueprint_digest, slot, predecessor_ref_digest, child_dedup, found)
  if created_entry == nil then
    if generated_fact == nil then
      if child_issue ~= nil then
        log_decision(origin, "materialization", "created", "skip-existing-child-unreadable", "trusted github-proxy issue-created marker is visible but child issue is not readable yet")
        return "wait", nil
      end
      return nil, "existing-child-malformed"
    end
    created_entry = {
      origin = generated_fact.origin,
      blueprint_digest = generated_fact.blueprint_digest,
      slot = generated_fact.slot,
      predecessor_ref_digest = generated_fact.predecessor_ref_digest,
      gen_contract_digest = generated_fact.gen_contract_digest,
      gen_spec_digest = generated_fact.gen_spec_digest,
      child_dedup = generated_fact.child_dedup,
      state = "created",
      child_issue = tostring(found.number or child_issue),
    }
  end
  local outcome = source == "parent-created-marker" and "applied(parent-ledger-created)" or "applied(existing-child-created)"
  local reason = source == "parent-created-marker"
    and "trusted github-proxy issue-created marker is visible"
    or "trusted github-proxy issue-create marker is visible on child"
  log_decision(origin, "materialization", "created", outcome, reason)
  M.raise_request(
    origin,
    "github-proxy.github_issue_comment_request",
    M.materialization_comment_request(repo, issue_number, origin, created_entry, "created", created_entry.child_issue)
  )
  return true, nil
end

function M.maybe_write_created_from_existing_child(core, deps, repo, issue_number, origin, blueprint_fact, record, facts, current, trusted_comments, log_decision)
  -- A slot whose "created" ledger fact already exists must NOT be re-derived from
  -- its "generated" fact on every tick: the generated marker stays visible next to
  -- the created marker, so re-writing "created" and returning true here forever
  -- starves frontier advancement (compute_frontier is never reached, the next slot
  -- never materializes). Skip a generated fact once its slot already has a created
  -- fact. Found by real supervise dogfood 2026-07-03: a merged scaffold child never
  -- advanced to the implement slot because this returned true every 5m tick.
  local already_created = {}
  for _, fact in ipairs(facts or {}) do
    if fact.state == "created" and fact.child_dedup ~= nil then
      already_created[fact.child_dedup] = true
    end
  end
  for _, fact in ipairs(facts or {}) do
    if fact.state == "generated" and not already_created[fact.child_dedup] then
      local slot = M.find_step(record and record.blueprint, fact.slot) or { id = fact.slot }
      local wrote, reason = M.record_existing_child_or_created_marker(
        core,
        deps,
        repo,
        issue_number,
        origin,
        fact.blueprint_digest,
        slot,
        fact.predecessor_ref_digest,
        fact.child_dedup,
        facts,
        current,
        trusted_comments,
        log_decision
      )
      if wrote == "wait" then
        return "wait"
      end
      if wrote == nil then
        error("github-devloop-workflow: materialization-existing-child-invalid: " .. tostring(reason or "existing-child-malformed"))
      end
      if wrote then
        return true
      end
    end
  end
  return false
end

function M.record_created_or_raise_create(core, deps, repo, issue_number, origin, blueprint_fact, current, trusted_comments, facts, blueprint_digest, slot, predecessor_ref_digest, generated_spec, log_decision)
  local entry = materialization.write_generated_entry(origin, blueprint_digest, slot, predecessor_ref_digest, generated_spec)
  if entry == nil then
    return nil, "invalid-materialization-entry"
  end
  local wrote, reason = M.record_existing_child_or_created_marker(
    core,
    deps,
    repo,
    issue_number,
    origin,
    blueprint_digest,
    slot,
    predecessor_ref_digest,
    entry.child_dedup,
    facts,
    current,
    trusted_comments,
    log_decision
  )
  if wrote == "wait" then
    return "wait", nil
  end
  if wrote == nil then
    return nil, reason
  end
  if wrote then
    return true, nil
  end
  log_decision(origin, "materialization", "create", "applied(proceed-create)", "generated spec digest is ready and no child ledger is visible")
  M.raise_request(
    origin,
    "github-proxy.github_issue_create_request",
    M.issue_create_request(repo, issue_number, origin, blueprint_digest, slot.id, entry, generated_spec)
  )
  return true, nil
end

function M.facts_for_key(facts, key)
  local matched = {}
  for _, fact in ipairs(facts or {}) do
    if materialization.fact_key(fact) == key then
      matched[#matched + 1] = fact
    end
  end
  return matched
end

function M.best_fact_for_key(facts, key)
  local best = nil
  local best_rank = -1
  for _, fact in ipairs(M.facts_for_key(facts, key)) do
    local rank = marker.MATERIALIZATION_STATE_RANK[fact.state] or 0
    if best == nil or rank >= best_rank then
      best = fact
      best_rank = rank
    end
  end
  return best
end

return M
