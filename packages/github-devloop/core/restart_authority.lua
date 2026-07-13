-- This grant-disabled shadow slice proves CAS-admission-composition parity only.
-- The loop department's thinking-to-blocked probe is an admission sentinel; the
-- real blocked transition belongs to reconcile, so this does not prove effect parity.
--
-- Scope of the cas_outcome claim (honest bound): parity holds for admission status
-- and reason_code (two independent reachability implementations), and for cas_outcome
-- within the tested version regime (incoming dedup_key == current marker version). The
-- loop stale cas_outcome version-branch is NOT yet modeled here: production loop emits
-- "skip-stale(incoming version < current marker version)" when the dedup_key is
-- ordered-older (devloop_state.cas_outcome passes dedup_key as incoming_version),
-- whereas the catalog's plain loop model + this shadow's version-less evidence always
-- yield "skip-advanced-or-diverged". This divergence is rooted in the (unchanged)
-- catalog plain modeling of loop cas_outcome; the shadow composes it faithfully. It is
-- a documented gap to close before any grant-enablement (add a stale older-dedup_key
-- fixture + thread the incoming version, or make cas.legacy_loop_plain_v1 model the
-- stale version-branch).

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
}

local function illegal(reason_code, outcome_reason)
  return {
    status = "illegal",
    reason_code = reason_code,
    cas_outcome = "illegal(" .. tostring(outcome_reason or reason_code) .. ")",
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
  if intent.incoming_version ~= nil
    and (type(intent.incoming_version) ~= "string" or intent.incoming_version == "") then
    return nil
  end
  if intent.target_version ~= nil
    and (type(intent.target_version) ~= "string" or intent.target_version == "") then
    return nil
  end
  return {
    semantic_variant = intent.semantic_variant,
    source_boundary = intent.source_boundary,
    target = intent.target,
    evidence_refs = intent.evidence_refs,
    incoming_version = intent.incoming_version,
    target_version = intent.target_version,
  }
end

local function select_edge(semantic_variant)
  local selected = nil
  local matches = 0
  for _, edge in ipairs(edges) do
    if edge.semantic_variant == semantic_variant then
      selected = edge
      matches = matches + 1
    end
  end
  return selected, matches
end

local function exact_source_state(source_states, source_state)
  if type(source_states) ~= "table" or #source_states ~= 1 or source_states[1] ~= source_state then
    return false
  end
  for key in pairs(source_states) do
    if key ~= 1 then
      return false
    end
  end
  return true
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
      version = current.version,
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

  local edge, matches = select_edge(normalized.semantic_variant)
  if matches == 0 then
    return illegal("unknown-variant")
  end
  if matches > 1 then
    return illegal("ambiguous-variant")
  end
  if edge.cas_policy_id ~= "cas.legacy_loop_plain_v1"
    and not (edge.cas_policy_id == "cas.legacy_consensus_result_v1"
      and edge.cas_variant == "thinking_to_ready") then
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
  if variant == nil
    or not exact_source_state(variant.source_states, edge.source.state)
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
