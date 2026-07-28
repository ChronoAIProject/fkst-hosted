local config = require("devloop.config")
local devloop_base = require("devloop.base")
local devloop_state = require("devloop.state")
local t = fkst.test

local function env_exec(value)
  return function(command)
    return {
      stdout = command == config.read_env_command("FKST_SESSION_WORK_LABEL_MAP_JSON") and (value or "") or "",
      stderr = "",
      exit_code = 0,
    }
  end
end

local function env_values(values)
  return function(command)
    return {
      stdout = values[command] or "",
      stderr = "",
      exit_code = 0,
    }
  end
end

local function assert_invalid(source, expected)
  local ok, err = pcall(config.parse_work_label_map_json, source)
  t.eq(ok, false)
  t.is_true(tostring(err):find(expected, 1, true) ~= nil)
end

local function assert_rejected(source)
  local ok = pcall(config.parse_work_label_map_json, source)
  t.eq(ok, false)
end

local function mock_map(source, reads)
  for _ = 1, reads or 1 do
    t.mock_command(config.read_env_command("FKST_SESSION_WORK_LABEL_MAP_JSON"), {
      stdout = source,
      stderr = "",
      exit_code = 0,
    })
  end
end

return {
  test_unset_mapping_preserves_identity = function()
    t.eq(config.effective_work_label("fkst-dev", env_exec("")), "fkst-dev")
    local labels = config.effective_work_labels({ "fkst-dev", "fkst-dev", "fkst-dev:claimed" }, env_exec(""))
    t.eq(#labels, 2)
    t.eq(labels[1], "fkst-dev")
    t.eq(labels[2], "fkst-dev:claimed")
  end,

  test_valid_map_translates_complete_work_label_families = function()
    local source = [[{"fkst-dev":"fkst-dev-chronoai-fkst","fkst-security":"fkst-security-chronoai-fkst"}]]
    local labels = config.effective_work_labels({
      "fkst-security",
      "fkst-dev",
      "fkst-dev:claimed",
      "fkst-security:reviewing",
      "bug",
      "fkst-security",
    }, env_exec(source))
    t.eq(#labels, 5)
    t.eq(labels[1], "fkst-security-chronoai-fkst")
    t.eq(labels[2], "fkst-dev-chronoai-fkst")
    t.eq(labels[3], "fkst-dev-chronoai-fkst:claimed")
    t.eq(labels[4], "fkst-security-chronoai-fkst:reviewing")
    t.eq(labels[5], "bug")
  end,

  test_namespace_derives_arbitrary_work_label_families_without_explicit_map = function()
    local namespace = "chronoai-fkst-cloud-test"
    local exec = env_values({
      [config.read_env_command("FKST_WORK_LABEL_NAMESPACE")] = namespace,
      [config.read_env_command("FKST_SESSION_WORK_LABEL")] = table.concat({
        "fkst-dev-" .. namespace,
        "fkst-security-" .. namespace,
        "fkst-abcdefg-" .. namespace,
      }, ","),
    })

    t.eq(config.effective_work_label("fkst-dev:claimed", exec), "fkst-dev-" .. namespace .. ":claimed")
    t.eq(
      config.effective_work_label("fkst-security:reviewing", exec),
      "fkst-security-" .. namespace .. ":reviewing"
    )
    t.eq(
      config.effective_work_label("fkst-abcdefg:thinking", exec),
      "fkst-abcdefg-" .. namespace .. ":thinking"
    )
    local configured = config.session_work_labels(exec)
    t.eq(configured[1], "fkst-dev-" .. namespace)
    t.eq(configured[2], "fkst-security-" .. namespace)
    t.eq(configured[3], "fkst-abcdefg-" .. namespace)
  end,

  test_namespace_transforms_logical_session_work_labels_and_rejects_inconsistent_map = function()
    local namespace = "chronoai-fkst-cloud-test"
    local logical_exec = env_values({
      [config.read_env_command("FKST_WORK_LABEL_NAMESPACE")] = namespace,
      [config.read_env_command("FKST_SESSION_WORK_LABEL")] = "fkst-dev",
    })
    t.eq(config.session_work_labels(logical_exec)[1], "fkst-dev-" .. namespace)

    local inconsistent_exec = env_values({
      [config.read_env_command("FKST_WORK_LABEL_NAMESPACE")] = namespace,
      [config.read_env_command("FKST_SESSION_WORK_LABEL_MAP_JSON")] =
        [[{"fkst-dev":"fkst-dev-another-provider"}]],
    })
    local ok, err = pcall(config.effective_work_label, "fkst-dev:claimed", inconsistent_exec)
    t.eq(ok, false)
    t.is_true(tostring(err):find("does not match FKST_WORK_LABEL_NAMESPACE", 1, true) ~= nil)
  end,

  test_arbitrary_and_overlapping_roots_use_longest_family_match = function()
    local map = {
      ["fkst"] = "fkst-cloud",
      ["fkst-dev"] = "fkst-dev-cloud",
      ["fkst-abcdefg"] = "fkst-abcdefg-cloud",
    }
    t.eq(config.apply_work_label_map_to_label("fkst-dev:claimed", map), "fkst-dev-cloud:claimed")
    t.eq(config.apply_work_label_map_to_label("fkst-abcdefg:ready", map), "fkst-abcdefg-cloud:ready")
    t.eq(config.apply_work_label_map_to_label("fkst-security:ready", map), "fkst-security:ready")
    t.eq(config.apply_work_label_map_to_label("fkst-dev-extra:ready", map), "fkst-dev-extra:ready")
  end,

  test_label_color_keys_follow_the_same_family_mapping = function()
    local map = { ["fkst-dev"] = "fkst-dev-cloud" }
    local colors = config.effective_label_colors({
      ["fkst-dev:ready"] = "0E8A16",
      ["bug"] = "ffffff",
    }, map)
    t.eq(colors["fkst-dev-cloud:ready"], "0E8A16")
    t.eq(colors.bug, "ffffff")
    t.eq(colors["fkst-dev:ready"], nil)
  end,

  test_inbound_comparison_accepts_only_the_effective_family = function()
    local map = { ["fkst-dev"] = "fkst-dev-chronoai-fkst-cloud-test" }
    t.is_true(config.label_matches_effective(
      "fkst-dev-chronoai-fkst-cloud-test:claimed",
      "fkst-dev:claimed",
      nil,
      map
    ))
    t.is_true(not config.label_matches_effective("fkst-dev:claimed", "fkst-dev:claimed", nil, map))
    t.is_true(not config.label_matches_effective(
      "fkst-dev-another-provider:claimed",
      "fkst-dev:claimed",
      nil,
      map
    ))
  end,

  test_devloop_state_and_intake_hints_accept_only_namespaced_lifecycle_labels = function()
    local source = [[{"fkst-dev":"fkst-dev-chronoai-fkst-cloud-test"}]]
    local effective = "fkst-dev-chronoai-fkst-cloud-test"

    mock_map(source)
    t.is_true(devloop_state.has_label({ effective .. ":thinking" }, "fkst-dev:thinking"))
    mock_map(source)
    t.is_true(not devloop_state.has_label({ "fkst-dev:thinking" }, "fkst-dev:thinking"))
    mock_map(source)
    t.is_true(not devloop_state.has_label(
      { "fkst-dev-another-provider:thinking" },
      "fkst-dev:thinking"
    ))

    mock_map(source)
    t.is_true(devloop_base.is_opted_in({ effective .. ":enabled" }))
    mock_map(source)
    t.is_true(not devloop_base.is_opted_in({ "fkst-dev:enabled" }))

    mock_map(source)
    local add, remove = devloop_state.state_label_reconcile_changes({
      effective .. ":thinking",
      "fkst-dev:reviewing",
      "fkst-dev-another-provider:reviewing",
    }, "reviewing")
    t.eq(add[1], "fkst-dev:reviewing")
    t.eq(#remove, 1)
    t.eq(remove[1], effective .. ":thinking")
  end,

  test_overlong_lifecycle_suffix_is_deterministically_compacted_under_github_limit = function()
    local map = { ["fkst-dev"] = "fkst-dev-chronoai-fkst-cloud-test" }
    local first = config.apply_work_label_map_to_label("fkst-dev:blocked-on-dependency", map)
    local second = config.apply_work_label_map_to_label("fkst-dev:blocked-on-dependency", map)
    t.eq(first, second)
    t.eq(utf8.len(first), 50)
    t.is_true(first:find("fkst-dev-chronoai-fkst-cloud-test:block", 1, true) == 1)
    t.is_true(first ~= "fkst-dev-chronoai-fkst-cloud-test:blocked-on-dependency")
  end,

  test_malformed_or_non_object_json_fails_closed = function()
    assert_invalid("not-json", "expected a JSON object")
    assert_invalid("[]", "expected a JSON object")
    assert_invalid([[{"fkst-dev":}]], "malformed JSON object")
    assert_invalid([[{"fkst-dev":7}]], "effective label must be a non-empty string")
    assert_invalid([[{"fkst-dev":" fkst-dev-cloud"}]], "cannot have surrounding whitespace")
    assert_invalid([[{"fkst-dev":"fkst-dev,cloud"}]], "cannot contain a comma")
    assert_invalid([[{"fkst-dev":"fkst-dev\u0001cloud"}]], "cannot contain control characters")
    assert_invalid([[{"fkst-dev":"]] .. string.rep("x", 51) .. [["}]], "50-character limit")
    assert_rejected('{"fkst-dev":"fkst-' .. string.char(255) .. '"}')
  end,

  test_case_insensitive_effective_collision_fails_closed = function()
    assert_invalid(
      [[{"fkst-dev":"FKST-DEV-CLOUD","fkst-security":"fkst-dev-cloud"}]],
      "effective labels collide case-insensitively"
    )
  end,

  test_invalid_namespace_fails_closed = function()
    for _, invalid in ipairs({ "Cloud", "cloud--test", "-cloud", "cloud-", string.rep("a", 49) }) do
      local ok = pcall(config.parse_work_label_namespace, invalid)
      t.eq(ok, false)
    end
  end,
}
