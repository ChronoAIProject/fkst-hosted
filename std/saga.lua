-- std.saga: shared department shape for event-level idempotent sagas.
local S = {}

local function always_accept(_event)
  return true
end

local function validate_consumes(consumes)
  if type(consumes) ~= "table" or #consumes == 0 then
    error("std.saga: department requires non-empty consumes")
  end
end

local function validate_opts(opts)
  if type(opts) ~= "table" then
    error("std.saga: department requires opts")
  end
  validate_consumes(opts.consumes)
  if type(opts.done) ~= "function" then
    error("std.saga: department requires done")
  end
  if type(opts.act) ~= "function" then
    error("std.saga: department requires act")
  end
end

local function spec_from_opts(opts)
  return {
    consumes = opts.consumes,
    produces = opts.produces,
    stall_window = opts.stall_window,
    retry = opts.retry,
    fanout = opts.fanout,
    ephemeral = opts.ephemeral,
  }
end

function S.department(opts)
  validate_opts(opts)

  local accept = opts.accept or always_accept
  local function raw(event)
    if not accept(event) then
      if type(opts.on_skip_foreign) == "function" then
        opts.on_skip_foreign(event)
      end
      return nil
    end
    if opts.done(event) then
      if type(opts.on_skip) == "function" then
        opts.on_skip(event)
      end
      return nil
    end
    return opts.act(event)
  end

  local name = opts.name or "std.saga"
  local wrapped = raw
  if type(opts.wrap) == "function" then
    wrapped = opts.wrap(name, raw)
  end
  _G.pipeline = wrapped

  return {
    spec = spec_from_opts(opts),
  }
end

return S
