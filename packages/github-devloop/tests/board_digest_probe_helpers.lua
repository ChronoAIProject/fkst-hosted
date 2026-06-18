local core = require("core")

local M = {}

M.spec = {
  consumes = { "board_digest_probe" },
  produces = { "board_digest_result" },
}

local function lua_literal(value)
  local kind = type(value)
  if kind == "string" then
    return string.format("%q", value)
  end
  if kind == "number" or kind == "boolean" then
    return tostring(value)
  end
  if kind == "nil" then
    return "nil"
  end
  if kind == "table" then
    local parts = {}
    for key, field in pairs(value) do
      table.insert(parts, "[" .. lua_literal(key) .. "]=" .. lua_literal(field))
    end
    return "{" .. table.concat(parts, ",") .. "}"
  end
  error("unsupported result value type: " .. kind)
end

local function write_file(path, content)
  local dir = tostring(path):match("^(.*)/[^/]+$")
  if dir ~= nil then
    os.execute("mkdir -p " .. string.format("%q", dir))
  end
  local handle = assert(io.open(path, "w"))
  handle:write(content)
  handle:close()
end

function M.run(payload)
  if payload.mode == "block" then
    return {
      body = core.board_digest_block(payload.repo, payload.tick),
    }
  end

  if payload.mode == "append" then
    return {
      proposal = core.append_board_digest_to_proposal(payload.proposal, payload.repo, payload.tick),
    }
  end

  if payload.mode == "board_loop" then
    return {
      proposal = core.build_board_loop_proposal(
        payload.repo,
        payload.issue_number,
        payload.current,
        payload.source_ref,
        payload.n,
        payload.converge,
        payload.tick
      ),
    }
  end

  if payload.mode == "board_review" then
    return {
      proposal = core.build_board_pr_review_proposal(
        payload.repo,
        payload.issue_number,
        payload.pr_number,
        payload.version,
        payload.head_sha,
        payload.current,
        payload.source_ref,
        payload.tick
      ),
    }
  end

  if payload.mode == "board_review_loop" then
    return {
      proposal = core.build_board_pr_review_loop_proposal(
        payload.repo,
        payload.issue_number,
        payload.pr_number,
        payload.version,
        payload.head_sha,
        payload.current,
        payload.source_ref,
        payload.n,
        payload.converge,
        payload.tick
      ),
    }
  end

  error("github-devloop test probe: unknown mode")
end

function pipeline(event)
  local payload = event.payload or {}
  if payload.result_path == nil then
    error("board digest probe requires result_path")
  end
  write_file(payload.result_path, "return " .. lua_literal(M.run(payload)) .. "\n")
end

return M
