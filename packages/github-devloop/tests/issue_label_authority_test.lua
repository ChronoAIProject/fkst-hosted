local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local issue = h.issue
local run_observe = h.run_observe
local mock_issue_state = h.mock_issue_state
local mock_pr_origin_for = h.mock_pr_origin_for
local find_raise = h.find_raise
local count_calls = h.count_calls

local function package_root()
  local source = package.searchpath("tests.devloop_helpers", package.path)
  return source:match("(.+)/tests/devloop_helpers%.lua$")
end

local function read_source(path)
  local handle = assert(io.open(package_root() .. "/" .. path, "r"))
  local body = handle:read("*a")
  handle:close()
  return body
end

return {
  test_observe_issue_reconciles_pr_open_label_when_backing_pr_exists = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:implementing" }, "OPEN", {
      core.state_marker(proposal_id, "pr-open", impl_version),
      core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", impl_version, "dev"),
    })
    mock_pr_origin_for({
      comments = {
        core.pr_origin_marker(proposal_id, "42", "devloop-owner-repo-42-01HY", impl_version, "dev"),
      },
      times = 2,
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:implementing" } }), opts("observe-pr-open-label-authority"))
    t.eq(result.exit_code, 0)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request", function(payload)
      return tostring(payload.target_kind or "issue") == "issue"
    end)
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:pr-open")
    t.eq(label_raise.payload.remove_labels[1], "fkst-dev:implementing")
    t.eq(count_calls("--json body"), 0)
  end,

  test_pr_open_issue_state_label_authority_stays_in_observe_issue = function()
    local open_pr_body = read_source("departments/open_pr/main.lua")
    t.eq(open_pr_body:find("github-proxy.github_issue_label_request", 1, true), nil)
    t.eq(open_pr_body:find("build_state_label_request", 1, true), nil)

    local requests_body = read_source("core/requests.lua")
    t.eq(requests_body:find("build_pr_open_label_request", 1, true), nil)
    t.eq(requests_body:find("issue_label_add = add_labels", 1, true), nil)
    t.eq(requests_body:find("issue_label_remove = remove_labels", 1, true), nil)

    local observe_body = read_source("departments/observe_issue/main.lua")
    t.is_true(observe_body:find("state_label_reconcile_changes", 1, true) ~= nil)
    t.is_true(observe_body:find("github-proxy.github_issue_label_request", 1, true) ~= nil)
  end,
}
