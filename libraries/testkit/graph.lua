local M = {}

local t = fkst.test

local function contains(value, expected)
  return tostring(value or ""):find(tostring(expected or ""), 1, true) ~= nil
end

function M.run(event_or_source, opts)
  return t.run_graph(event_or_source, opts or {})
end

function M.find_delivery(trace, expected)
  for index, step in ipairs((trace and trace.steps) or {}) do
    local queue_ok = expected.queue == nil or step.queue == expected.queue
    local consumer_ok = expected.consumer == nil or step.consumer == expected.consumer
    if queue_ok and consumer_ok then
      if expected.predicate == nil or expected.predicate(step, index) then
        return step, index
      end
    end
  end
  return nil
end

function M.require_delivery(trace, expected)
  local step, index = M.find_delivery(trace, expected)
  if step == nil then
    error(
      "missing delivery queue="
        .. tostring(expected.queue)
        .. " consumer="
        .. tostring(expected.consumer),
      2
    )
  end
  return step, index
end

function M.find_raise(trace, queue, predicate)
  for step_index, step in ipairs((trace and trace.steps) or {}) do
    for raise_index, raised in ipairs(step.raises or {}) do
      if raised.queue == queue and (predicate == nil or predicate(raised, step, step_index)) then
        return raised, step, step_index, raise_index
      end
    end
  end
  return nil
end

function M.require_raise(trace, queue, predicate)
  local raised, step, step_index, raise_index = M.find_raise(trace, queue, predicate)
  if raised == nil then
    error("missing raised queue=" .. tostring(queue), 2)
  end
  return raised, step, step_index, raise_index
end

function M.require_quiescent(trace)
  t.eq(trace.status, "quiescent")
  t.eq(trace.final.dead_letters, 0)
  for index, step in ipairs((trace and trace.steps) or {}) do
    if step.status ~= "accepted" or step.exit_code ~= 0 then
      error(
        "delivery step failed at index="
          .. tostring(index)
          .. " queue="
          .. tostring(step.queue)
          .. " consumer="
          .. tostring(step.consumer)
          .. " error="
          .. tostring(step.error),
        2
      )
    end
  end
  return trace
end

function M.signature(trace)
  local rows = {}
  for index, step in ipairs((trace and trace.steps) or {}) do
    local raises = {}
    for raise_index, raised in ipairs(step.raises or {}) do
      local payload = raised.payload or {}
      raises[raise_index] = table.concat({
        tostring(raised.queue or ""),
        tostring(payload.schema or ""),
        tostring(payload.proposal_id or ""),
        tostring(payload.decision or ""),
        tostring(payload.dedup_key or ""),
      }, ":")
    end
    rows[index] = table.concat({
      tostring(step.delivery_id or ""),
      tostring(step.queue or ""),
      tostring(step.consumer or ""),
      tostring(step.status or ""),
      table.concat(raises, ","),
    }, ">")
  end
  return table.concat(rows, "|")
end

function M.signature_without_payload_identity(trace)
  local rows = {}
  for index, step in ipairs((trace and trace.steps) or {}) do
    local raises = {}
    for raise_index, raised in ipairs(step.raises or {}) do
      local payload = raised.payload or {}
      raises[raise_index] = table.concat({
        tostring(raised.queue or ""),
        tostring(payload.schema or ""),
        tostring(payload.decision or ""),
      }, ":")
    end
    rows[index] = table.concat({
      tostring(step.queue or ""),
      tostring(step.consumer or ""),
      tostring(step.status or ""),
      table.concat(raises, ","),
    }, ">")
  end
  return table.concat(rows, "|")
end

function M.payload_contains(raised, fragment)
  local payload = raised and raised.payload or {}
  return contains(payload.body, fragment)
    or contains(payload.dedup_key, fragment)
    or contains(payload.proposal_id, fragment)
end

return M
