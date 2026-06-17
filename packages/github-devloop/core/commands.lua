local S = {}

local modules = {
  "core.commands.support",
  "core.commands.validators",
  "core.commands.issue_reads",
  "core.commands.observe_lists",
  "core.commands.dashboard",
  "core.commands.labels",
  "core.commands.prs",
  "core.commands.git_ops",
}

function S.install(M)
  for _, module_name in ipairs(modules) do
    require(module_name).install(M)
  end
end

return S
