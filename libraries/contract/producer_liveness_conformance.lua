-- contract.producer_liveness_conformance: deterministic positive-progress checks
-- for autonomous producer obligations.
local C = {}

local testing = require("contract.testing")

local function has_text(value)
  return type(value) == "string" and value ~= ""
end

local function declared_contracts(module)
  if type(module) ~= "table" or type(module.producer_liveness_contracts) ~= "function" then
    error("producer-liveness: package core must expose producer_liveness_contracts")
  end
  local contracts = module.producer_liveness_contracts()
  if type(contracts) ~= "table" then
    error("producer-liveness: producer_liveness_contracts must return a table")
  end
  return contracts
end

local function queue_set(list, field)
  local out = {}
  for _, value in ipairs(list or {}) do
    if not has_text(value) then
      error("producer-liveness: " .. tostring(field) .. " entries must be non-empty strings")
    end
    out[value] = true
  end
  return out
end

local function list_has(list, value)
  for _, item in ipairs(list or {}) do
    if item == value then
      return true
    end
  end
  return false
end

local function validate_contract(contract)
  if type(contract) ~= "table" then
    error("producer-liveness: contract must be a table")
  end
  for _, field in ipairs({ "producer_id", "trigger_source", "eligibility_predicate", "progress_output" }) do
    if not has_text(contract[field]) then
      error("producer-liveness: contract missing " .. field)
    end
  end
  if tonumber(contract.max_staleness_seconds) == nil or tonumber(contract.max_staleness_seconds) <= 0 then
    error("producer-liveness: max_staleness_seconds must be positive")
  end
  if tonumber(contract.max_silence_seconds) == nil or tonumber(contract.max_silence_seconds) <= 0 then
    error("producer-liveness: max_silence_seconds must be positive")
  end
  if tonumber(contract.max_skip_budget) == nil or tonumber(contract.max_skip_budget) < 0 then
    error("producer-liveness: max_skip_budget must be non-negative")
  end
  if type(contract.output_queues) ~= "table" or #contract.output_queues == 0 then
    error("producer-liveness: output_queues must be a non-empty list")
  end
  if contract.escalation_queues ~= nil and type(contract.escalation_queues) ~= "table" then
    error("producer-liveness: escalation_queues must be a list when declared")
  end
end

local function validate_department_binding(department, contract)
  if type(department) ~= "table" or type(department.spec) ~= "table" then
    error("producer-liveness: department must expose spec")
  end
  if not list_has(department.spec.consumes, contract.trigger_source) then
    error(
      "producer-liveness: "
        .. tostring(contract.producer_id)
        .. " trigger_source is not consumed by department: "
        .. tostring(contract.trigger_source)
    )
  end
  for _, queue in ipairs(contract.output_queues or {}) do
    if not list_has(department.spec.produces, queue) then
      error(
        "producer-liveness: "
          .. tostring(contract.producer_id)
          .. " output_queue is not produced by department: "
          .. tostring(queue)
      )
    end
  end
  for _, queue in ipairs(contract.escalation_queues or {}) do
    if not list_has(department.spec.produces, queue) then
      error(
        "producer-liveness: "
          .. tostring(contract.producer_id)
          .. " escalation_queue is not produced by department: "
          .. tostring(queue)
      )
    end
  end
end

local function produced_progress(raises, outputs, escalations)
  for _, item in ipairs(raises or {}) do
    local queue = item.queue
    if outputs[queue] == true then
      return true, "output", queue
    end
    if escalations[queue] == true then
      return true, "escalation", queue
    end
  end
  return false, nil, nil
end

local function assert_declared_producer_progress(config, contract)
  local t = assert(config.t, "producer-liveness: missing fkst.test handle")
  local department = config.department
  local department_for_delivery = config.department_for_delivery
  if department == nil and type(department_for_delivery) ~= "function" then
    error("producer-liveness: missing department or department_for_delivery")
  end
  local event_for_contract = assert(config.event_for_contract, "producer-liveness: missing event_for_contract")
  local before_delivery = config.before_delivery

  validate_contract(contract)
  local outputs = queue_set(contract.output_queues, "output_queues")
  local escalations = queue_set(contract.escalation_queues, "escalation_queues")
  local max_deliveries = math.floor(tonumber(contract.max_skip_budget)) + 1

  for attempt = 1, max_deliveries do
    if type(before_delivery) == "function" then
      before_delivery(contract, attempt)
    end
    local current_department = department
    if type(department_for_delivery) == "function" then
      current_department = department_for_delivery(contract, attempt)
    end
    validate_department_binding(current_department, contract)
    local result = testing.run_fake(current_department, event_for_contract(contract, attempt))
    local progressed, kind, queue = produced_progress(result.raises, outputs, escalations)
    if progressed then
      return {
        attempt = attempt,
        kind = kind,
        queue = queue,
      }
    end
  end

  error(
    "producer-liveness: "
      .. tostring(contract.producer_id)
      .. " produced no output or escalation within max_skip_budget="
      .. tostring(contract.max_skip_budget)
  )
end

function C.assert_declared_producer_progress(config)
  local package_core = assert(config.package_core, "producer-liveness: missing package_core")
  local contracts = declared_contracts(package_core)
  local only = config.producer_id
  local matched = false
  for _, contract in ipairs(contracts) do
    if only == nil or contract.producer_id == only then
      matched = true
      assert_declared_producer_progress(config, contract)
    end
  end
  if only ~= nil and matched ~= true then
    error("producer-liveness: no contract named " .. tostring(only))
  end
end

return C
