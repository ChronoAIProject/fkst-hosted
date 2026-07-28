local config = require("devloop.config")
local t = fkst.test

local function env_exec(value)
  return function(command)
    t.eq(command, config.read_env_command("FKST_SESSION_WORK_LABEL_MAP_JSON"))
    return {
      stdout = value or "",
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

return {
  test_unset_mapping_preserves_identity = function()
    t.eq(config.effective_work_label("fkst-dev", env_exec("")), "fkst-dev")
    local labels = config.effective_work_labels({ "fkst-dev", "fkst-dev", "fkst-dev:claimed" }, env_exec(""))
    t.eq(#labels, 2)
    t.eq(labels[1], "fkst-dev")
    t.eq(labels[2], "fkst-dev:claimed")
  end,

  test_valid_map_translates_multiple_exact_labels_only = function()
    local source = [[{"fkst-dev":"fkst-dev-chronoai-fkst","fkst-security":"fkst-security-chronoai-fkst"}]]
    local labels = config.effective_work_labels({
      "fkst-security",
      "fkst-dev",
      "fkst-dev:claimed",
      "fkst-security",
    }, env_exec(source))
    t.eq(#labels, 3)
    t.eq(labels[1], "fkst-security-chronoai-fkst")
    t.eq(labels[2], "fkst-dev-chronoai-fkst")
    t.eq(labels[3], "fkst-dev:claimed")
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
}
