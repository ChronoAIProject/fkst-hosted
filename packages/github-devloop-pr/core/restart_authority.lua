-- This grant-disabled shadow slice proves CAS-admission-composition parity only.
-- It models selected review_result and fix edges while preserving the raw snapshot
-- fact for each policy profile to project independently.
-- No live effect is authorized or emitted by this module.

local core = require("core")
local owner = core.restart_package_name
local rows = core.restart_transition_table()
local inventories = {
  canonicalization = require("core.restart.canonicalization_inventory"),
  entry = require("core.restart.entry_inventory"),
  operator_reentry = require("core.restart.operator_reentry_inventory"),
}
local owner_pending_projection = require("devloop.restart_owner_pending_projection")
local edges = owner_pending_projection.edges(owner, rows, inventories)
local projection = owner_pending_projection.derive(owner, rows, inventories)
local catalog = require("devloop.restart_cas_catalog")
local restart_effect_entitlements = require("devloop.restart_effect_entitlements")

local M = {}
local issued = setmetatable({}, { __mode = "k" })
local intent_fields = {
  semantic_variant = true,
  source_boundary = true,
  target = true,
  evidence_refs = true,
  incoming_version = true,
  target_version = true,
  overlay_version = true,
}

local function illegal(reason_code, outcome_reason)
  local detail = outcome_reason or reason_code
  return {
    status = "illegal",
    reason_code = reason_code,
    cas_outcome = "illegal(" .. tostring(detail) .. ")",
    grant = nil,
  }
end

local function normalize_intent(intent)
  if type(intent) ~= "table" then
    return nil
  end
  for field in pairs(intent) do
    if intent_fields[field] ~= true then
      return nil
    end
  end
  if type(intent.semantic_variant) ~= "string" or intent.semantic_variant == "" then
    return nil
  end
  for _, field in ipairs({ "incoming_version", "target_version", "overlay_version" }) do
    local value = intent[field]
    if value ~= nil and (type(value) ~= "string" or value == "") then
      return nil
    end
  end
  return {
    semantic_variant = intent.semantic_variant,
    source_boundary = intent.source_boundary,
    target = intent.target,
    evidence_refs = intent.evidence_refs,
    incoming_version = intent.incoming_version,
    target_version = intent.target_version,
    overlay_version = intent.overlay_version,
  }
end

function M.seal_snapshot(fields)
  if type(fields) ~= "table" or fields.owner ~= owner then
    error("restart-authority: snapshot-owner-mismatch: owner must be " .. tostring(owner))
  end
  local current = type(fields.current) == "table" and fields.current or {}
  local sealed = {
    owner = fields.owner,
    proposal_id = fields.proposal_id,
    current = {
      state = current.state,
      version = current.version or "",
    },
  }
  issued[sealed] = true
  return sealed
end

function M.decide_transition(sealed_snapshot, intent)
  if issued[sealed_snapshot] ~= true or sealed_snapshot.owner ~= owner then
    return illegal("unsealed-or-foreign-snapshot", "unsealed")
  end

  local normalized = normalize_intent(intent)
  if normalized == nil then
    return illegal("malformed-intent")
  end

  local edge = nil
  local matches = 0
  for _, candidate in ipairs(edges) do
    if candidate.semantic_variant == normalized.semantic_variant then
      edge = candidate
      matches = matches + 1
    end
  end
  if matches == 0 then
    return illegal("unknown-variant")
  end
  if matches > 1 then
    return illegal("ambiguous-variant")
  end
  local supported_review_result = edge.cas_policy_id == "cas.legacy_review_result_v1"
    and (edge.cas_variant == "reviewing_to_merge_ready"
      or edge.cas_variant == "reviewing_to_fixing")
  local supported_fix = edge.cas_policy_id == "cas.legacy_fix_v1"
    and edge.cas_variant == "fixing_to_reviewing"
  if not supported_review_result and not supported_fix then
    return illegal("unsupported-shadow-edge")
  end
  if normalized.source_boundary ~= nil and normalized.source_boundary ~= edge.source.boundary then
    return illegal("source-boundary-mismatch")
  end
  if normalized.target ~= nil and normalized.target ~= edge.target then
    return illegal("target-mismatch")
  end

  local definition = catalog.definition(edge.cas_policy_id)
  local variant = definition
    and type(definition.variants) == "table"
    and definition.variants[edge.cas_variant]
    or nil
  local source_states = variant and variant.source_states
  local source_shape_exact = type(source_states) == "table"
    and #source_states == 1
    and source_states[1] == edge.source.state
  if source_shape_exact then
    for key in pairs(source_states) do
      if key ~= 1 then
        source_shape_exact = false
      end
    end
  end
  if variant == nil
    or not source_shape_exact
    or variant.target_state ~= edge.target then
    return illegal("policy-variant-shape-mismatch")
  end
  local cas_base = variant.base or definition.base
  if (cas_base == "versioned" or cas_base == "cyclic")
    and normalized.incoming_version == nil then
    return illegal("incoming-version-required")
  end

  local current = type(sealed_snapshot.current) == "table" and sealed_snapshot.current or {}
  local evidence = {
    current = {
      state = current.state,
      version = current.version,
    },
    variant = edge.cas_variant,
    incoming_version = normalized.incoming_version,
    target_version = normalized.target_version,
    overlay_version = normalized.overlay_version,
  }
  local resolved = catalog.resolve(edge.cas_policy_id, evidence, projection)
  local disposition = ({
    apply = "apply",
    idempotent = "idempotent",
  })[resolved.status]
  local effect_entitlement_id = nil
  local granted_effect_ids = nil
  if disposition ~= nil and edge.transition_effect_entitlements ~= nil then
    local entitlement = restart_effect_entitlements.resolve(edge, disposition)
    effect_entitlement_id = entitlement.id
    granted_effect_ids = entitlement.effect_ids
  end
  return {
    status = resolved.status,
    reason_code = resolved.reason_code,
    cas_outcome = resolved.cas_outcome,
    edge_id = edge.id,
    cas_policy_id = edge.cas_policy_id,
    effect_entitlement_id = effect_entitlement_id,
    granted_effect_ids = granted_effect_ids,
    evidence = {
      status = "complete",
      refs = normalized.evidence_refs or {},
      facts = {
        source = edge.source.state,
        target = edge.target,
      },
    },
    grant = nil,
  }
end

return M
