local h = require("tests.devloop_helpers")
local t = h.t

local function load_department(module_name)
  local old_pipeline = pipeline
  local ok, module = pcall(require, module_name)
  pipeline = old_pipeline
  if not ok then
    return nil, module
  end
  return module, nil
end

local function contains(values, expected)
  for _, value in ipairs(values or {}) do
    if value == expected then
      return true
    end
  end
  return false
end

return {
  test_merge_and_merge_queue_have_separate_srp_specs = function()
    local merge = assert(load_department("departments.merge.main"))
    local merge_queue, load_error = load_department("departments.merge_queue.main")

    t.is_true(merge_queue ~= nil, tostring(load_error or ""))
    t.is_true(contains(merge.spec.consumes, "devloop_merge_ready"))
    t.is_true(not contains(merge.spec.consumes, "devloop_merge_queue_tick"))
    t.is_true(contains(merge_queue.spec.consumes, "devloop_merge_queue_tick"))
    t.is_true(not contains(merge_queue.spec.consumes, "devloop_merge_ready"))
  end,
}
