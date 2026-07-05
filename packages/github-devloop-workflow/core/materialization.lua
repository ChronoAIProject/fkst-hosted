local digest = require("core.digest")
local marker = require("core.marker")
local strings = require("contract.strings")

local M = {}

M.EMPTY_PREDECESSOR_REF_DIGEST = "d-0000000000"

local function digest_string(prefix, value)
  return "d-" .. strings.decimal_checksum(prefix .. "\n" .. tostring(value or ""))
end

local function append_field(parts, name, value)
  table.insert(parts, name)
  table.insert(parts, tostring(#tostring(value or "")))
  table.insert(parts, tostring(value or ""))
end

function M.generated_spec_digest(generated_spec)
  if type(generated_spec) ~= "table" then
    return nil
  end
  local parts = {}
  append_field(parts, "title", generated_spec.title)
  append_field(parts, "body", generated_spec.body)
  return digest_string("generated-spec:v1", table.concat(parts, "\n"))
end

function M.generator_contract_digest(slot)
  if type(slot) ~= "table" or type(slot.content) ~= "table" then
    return nil
  end
  local content = slot.content
  local value = content.kind == "static" and content.intent or content.generator
  local parts = {}
  append_field(parts, "slot", slot.id)
  append_field(parts, "kind", content.kind)
  append_field(parts, "value", value)
  return digest_string("generator-contract:v1", table.concat(parts, "\n"))
end

function M.materialization_key(origin, blueprint_digest, slot_id, predecessor_ref_digest)
  return table.concat({
    tostring(origin or ""),
    tostring(blueprint_digest or ""),
    tostring(slot_id or ""),
    tostring(predecessor_ref_digest or M.EMPTY_PREDECESSOR_REF_DIGEST),
  }, "|")
end

function M.child_dedup_key(origin, slot_id, predecessor_ref_digest)
  return "workflow/materialize/" .. strings.sanitize_key(origin, false)
    .. "/" .. strings.sanitize_key(slot_id, false)
    .. "/" .. tostring(predecessor_ref_digest or M.EMPTY_PREDECESSOR_REF_DIGEST)
end

function M.key_from_parts(parts)
  if type(parts) ~= "table" then
    return nil
  end
  if parts.origin == nil or parts.blueprint_digest == nil
    or (parts.slot_id == nil and parts.slot == nil)
    or parts.predecessor_ref_digest == nil then
    return nil
  end
  return M.materialization_key(
    parts.origin,
    parts.blueprint_digest,
    parts.slot_id or parts.slot,
    parts.predecessor_ref_digest
  )
end

local function fact_key(fact)
  if type(fact) ~= "table" then
    return nil
  end
  return M.key_from_parts(fact)
end

M.fact_key = fact_key

local function matching_facts(ledger_facts, key)
  local facts = {}
  if type(ledger_facts) ~= "table" then
    return facts
  end
  local source = ledger_facts.facts or ledger_facts
  for _, fact in ipairs(source) do
    if fact_key(fact) == key then
      table.insert(facts, fact)
    end
  end
  return facts
end

local function best_fact(facts)
  local best = nil
  local best_rank = -1
  for _, fact in ipairs(facts) do
    local rank = marker.MATERIALIZATION_STATE_RANK[fact.state] or 0
    if best == nil or rank >= best_rank then
      best = fact
      best_rank = rank
    end
  end
  return best
end

local function same_key_facts_have_conflict(facts, gen_spec_digest)
  for _, fact in ipairs(facts) do
    if (fact.state == "generated" or fact.state == "created")
      and fact.gen_spec_digest ~= gen_spec_digest then
      return fact
    end
  end
  return nil
end

function M.latch_generated(ledger_facts, key, generated_spec)
  local gen_spec_digest = M.generated_spec_digest(generated_spec)
  if gen_spec_digest == nil then
    return {
      action = "error",
      reason_code = "invalid-generated-spec",
    }
  end

  local same_key_facts = matching_facts(ledger_facts, key)
  local conflict = same_key_facts_have_conflict(same_key_facts, gen_spec_digest)
  if conflict ~= nil then
    return {
      action = "error",
      reason_code = "generated-spec-digest-conflict",
      generated_spec_digest = gen_spec_digest,
      existing_generated_spec_digest = conflict.gen_spec_digest,
    }
  end

  local existing = best_fact(same_key_facts)
  if existing ~= nil and existing.state == "created" then
    return {
      action = "noop",
      reason_code = "already-created",
      generated_spec_digest = existing.gen_spec_digest,
      child_ref = existing.child_ref,
      child_issue = existing.child_issue,
    }
  end
  if existing ~= nil and existing.state == "generated" then
    return {
      action = "proceed_create",
      generated_spec_digest = gen_spec_digest,
      child_dedup_key = existing.child_dedup,
      fact = existing,
    }
  end

  return {
    action = "proceed_create",
    generated_spec_digest = gen_spec_digest,
  }
end

function M.write_generated_entry(origin, blueprint_digest, slot, predecessor_ref_digest, generated_spec)
  local gen_contract_digest = M.generator_contract_digest(slot)
  local gen_spec_digest = M.generated_spec_digest(generated_spec)
  if blueprint_digest == nil or gen_contract_digest == nil or gen_spec_digest == nil then
    return nil
  end
  return {
    origin = origin,
    blueprint_digest = blueprint_digest,
    slot = slot.id,
    predecessor_ref_digest = predecessor_ref_digest or M.EMPTY_PREDECESSOR_REF_DIGEST,
    gen_contract_digest = gen_contract_digest,
    gen_spec_digest = gen_spec_digest,
    child_dedup = M.child_dedup_key(origin, slot.id, predecessor_ref_digest),
    state = "generated",
  }
end

function M.created_entry(origin, blueprint_digest, slot, predecessor_ref_digest, generated_spec, child_issue)
  local entry = M.write_generated_entry(origin, blueprint_digest, slot, predecessor_ref_digest, generated_spec)
  if entry == nil then
    return nil
  end
  entry.state = "created"
  entry.child_issue = tostring(child_issue or "")
  if entry.child_issue == "" then
    return nil
  end
  return entry
end

function M.blueprint_digest(plan)
  return digest.blueprint_digest(plan)
end

function M.install(target)
  target.materialization = M
end

return M
