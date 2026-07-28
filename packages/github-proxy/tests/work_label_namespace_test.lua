local h = require("tests.proxy_integration_helpers")
local config = require("devloop.config")
local t = h.t

local function has_arg_pair(rendered, flag, value)
  local text = tostring(rendered or "")
  local quoted = tostring(flag) .. " '" .. tostring(value) .. "'"
  if text:find(quoted, 1, true) ~= nil then
    return true
  end
  local plain = tostring(flag) .. " " .. tostring(value)
  local start_at, end_at = text:find(plain, 1, true)
  return start_at ~= nil and (end_at == #text or text:sub(end_at + 1, end_at + 1) == " ")
end

return {
  test_label_request_translates_work_labels_before_create_and_mutation = function()
    local event = {
      queue = "github_issue_label_request",
      payload = {
        schema = "github-proxy.label.v1",
        repo = "owner/x",
        issue_number = 42,
        add_labels = { "fkst-dev", "fkst-dev:claimed" },
        remove_labels = { "fkst-security" },
        label_colors = { ["fkst-dev:claimed"] = "0E8A16" },
        dedup_key = "provider-work-label-map",
        source_ref = { kind = "external", ref = "owner/x#issue/42" },
      },
    }
    local map_json = [[{"fkst-dev":"fkst-dev-chronoai-fkst","fkst-security":"fkst-security-chronoai-fkst"}]]
    t.mock_command(config.read_env_command("FKST_SESSION_WORK_LABEL_MAP_JSON"), {
      stdout = map_json,
      stderr = "",
      exit_code = 0,
    })
    h.mock_write_env("1")
    h.mock_repo_label_list({ "fkst-security-chronoai-fkst", "fkst-dev-chronoai-fkst:claimed" })
    h.mock_label_create()
    t.mock_command("gh issue edit", { stdout = "", exit_code = 0 })

    local result = t.run_department("departments/github_issue_label/main.lua", event, h.opts("label-provider-map", {
      FKST_GITHUB_WRITE = "1",
      FKST_SESSION_WORK_LABEL_MAP_JSON = map_json,
    }))

    t.eq(result.exit_code, 0, tostring(result.error or result.stderr or "namespaced label mutation failed"))
    local create = h.calls_matching("gh label create")[1]
    t.is_true(create.rendered:find("fkst-dev-chronoai-fkst", 1, true) ~= nil, create.rendered)
    t.is_true(create.rendered:find("fkst-dev:claimed", 1, true) == nil, create.rendered)
    local edit = h.calls_matching("gh issue edit")[1]
    t.is_true(has_arg_pair(edit.rendered, "--add-label", "fkst-dev-chronoai-fkst"), edit.rendered)
    t.is_true(has_arg_pair(edit.rendered, "--add-label", "fkst-dev-chronoai-fkst:claimed"), edit.rendered)
    t.is_true(has_arg_pair(edit.rendered, "--remove-label", "fkst-security-chronoai-fkst"), edit.rendered)
    t.is_true(not has_arg_pair(edit.rendered, "--add-label", "fkst-dev"), edit.rendered)
  end,

  test_namespace_and_effective_session_roots_are_sufficient_without_explicit_map = function()
    local namespace = "chronoai-fkst-cloud-test"
    local base = "fkst-security-" .. namespace
    local event = {
      queue = "github_issue_label_request",
      payload = {
        schema = "github-proxy.label.v1",
        repo = "owner/x",
        issue_number = 42,
        add_labels = { "fkst-security:claimed" },
        remove_labels = { "fkst-security:thinking" },
        dedup_key = "provider-work-label-namespace-fallback",
        source_ref = { kind = "external", ref = "owner/x#issue/42" },
      },
    }
    t.mock_command(config.read_env_command("FKST_SESSION_WORK_LABEL_MAP_JSON"), {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(config.read_env_command("FKST_WORK_LABEL_NAMESPACE"), {
      stdout = namespace,
      stderr = "",
      exit_code = 0,
    })
    t.mock_command(config.read_env_command("FKST_SESSION_WORK_LABEL"), {
      stdout = base,
      stderr = "",
      exit_code = 0,
    })
    h.mock_write_env("1")
    h.mock_repo_label_list({ base .. ":claimed", base .. ":thinking" })
    t.mock_command("gh issue edit", { stdout = "", exit_code = 0 })

    local result = t.run_department("departments/github_issue_label/main.lua", event, h.opts("label-provider-namespace-fallback", {
      FKST_GITHUB_WRITE = "1",
      FKST_SESSION_WORK_LABEL = base,
      FKST_WORK_LABEL_NAMESPACE = namespace,
    }))

    t.eq(result.exit_code, 0, tostring(result.error or result.stderr or "namespace fallback label mutation failed"))
    local edit = h.calls_matching("gh issue edit")[1]
    t.is_true(has_arg_pair(edit.rendered, "--add-label", base .. ":claimed"), edit.rendered)
    t.is_true(has_arg_pair(edit.rendered, "--remove-label", base .. ":thinking"), edit.rendered)
    t.is_true(not has_arg_pair(edit.rendered, "--add-label", "fkst-security:claimed"), edit.rendered)
  end,
}
