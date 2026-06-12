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
        delivery_id = "delivery/v1/raised/queue/github-devloop.devloop_decompose/dept/github-devloop.decompose/01HY",
        queue = "github-devloop.devloop_decompose",
        dept = "github-devloop.decompose",
        source_ref = {
          kind = "external",
          ref = "owner/repo#issue/140",
        },
        dedup_key = "github-devloop/issue/owner/repo/140/2026-06-10T08-46-17Z",
        attempt = 3,
        error = "gh pr decomposed marker comment failed\nwhile writing marker",
      },
    })

    t.eq(#logs, 1)
    t.eq(
      logs[1],
      "github-devloop dept=dead_letter tag=DEAD_LETTER"
        .. " delivery_id=delivery/v1/raised/queue/github-devloop.devloop_decompose/dept/github-devloop.decompose/01HY"
        .. " queue=github-devloop.devloop_decompose"
        .. " dead_dept=github-devloop.decompose"
        .. " source_ref=external:owner/repo#issue/140"
        .. " dedup_key=github-devloop/issue/owner/repo/140/2026-06-10T08-46-17Z"
        .. " attempt=3"
        .. " error=gh pr decomposed marker comment failed while writing marker"
    )
  end,
}
