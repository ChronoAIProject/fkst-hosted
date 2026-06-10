local t = fkst.test

local function package_root()
  local source = package.searchpath("tests.dead_letter_test", package.path)
  return source:match("(.+)/tests/dead_letter_test%.lua$")
end

local function capture_logs(event)
  local captured = {}
  local old_log = log

  log = {
    info = function(message)
      table.insert(captured, tostring(message))
    end,
    warn = function(message)
      table.insert(captured, tostring(message))
    end,
    error = function(message)
      table.insert(captured, tostring(message))
    end,
  }

  local ok, result = pcall(function()
    dofile(package_root() .. "/departments/dead_letter/main.lua")
    pipeline(event)
  end)

  log = old_log
  if not ok then
    error(result)
  end

  return captured
end

return {
  test_dead_letter_logs_delivery_identity = function()
    local logs = capture_logs({
      queue = "dead_letter",
      payload = {
        delivery_id = "delivery/v1/raised/queue/consensus.proposal/dept/consensus.decide/01HY",
        queue = "consensus.proposal",
        dept = "consensus.decide",
        attempt = 3,
        error = "codex timed out\nwhile running decide",
      },
    })

    t.eq(#logs, 1)
    t.eq(
      logs[1],
      "consensus dept=dead_letter tag=DEAD_LETTER"
        .. " delivery_id=delivery/v1/raised/queue/consensus.proposal/dept/consensus.decide/01HY"
        .. " queue=consensus.proposal"
        .. " dead_dept=consensus.decide"
        .. " attempt=3"
        .. " error=codex timed out while running decide"
    )
  end,
}
