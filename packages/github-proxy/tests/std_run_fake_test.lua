local run_fake = require("std.testing").run_fake
local gh_fake = require("std.github_fake")

local function make_test_department(ports)
  local function pipeline(event)
    local issue = ports.github.read_issue(event.payload.source_ref)
    if issue.state == "OPEN" then
      raise("demo.request", { dedup_key = "d:" .. issue.number })
    end
  end
  return { spec = { consumes = { "demo" } }, pipeline = pipeline, ports = ports }
end

return {
  test_run_fake_captures_raises_and_reads = function()
    local model = gh_fake.model({
      issues = {
        ["owner/repo#issue/42"] = { number = 42, state = "OPEN" },
      },
    })
    local dept = make_test_department({ github = gh_fake.new(model), git = nil })
    local result = run_fake(dept, {
      payload = {
        source_ref = { kind = "external", ref = "owner/repo#issue/42" },
      },
    })
    assert(result.result == nil)
    assert(result.failure == nil)
    assert(#result.raises == 1, "must capture the S2 raise")
    assert(result.raises[1].queue == "demo.request")
    assert(result.raises[1].payload.dedup_key == "d:42")
    assert(result.writes == model.writes)
  end,

  test_run_fake_returns_failure_shape_without_rethrowing = function()
    local dept = {
      spec = { consumes = { "demo" } },
      pipeline = function(_event)
        raise("demo.before-fail", { dedup_key = "before-fail" })
        error("forced fake failure")
      end,
    }
    local result = run_fake(dept, { payload = {} })
    assert(result.result == nil)
    assert(result.failure ~= nil)
    assert(tostring(result.failure.error):find("forced fake failure", 1, true) ~= nil)
    assert(#result.raises == 1)
    assert(result.raises[1].queue == "demo.before-fail")
    assert(type(result.writes) == "table")
  end,
}
