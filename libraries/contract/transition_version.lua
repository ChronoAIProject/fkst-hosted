-- contract.transition_version: dependency-free transition version normalization.
local V = {}

function V.strip_suffixes(version)
  local text = tostring(version or "")
  local previous = nil
  while previous ~= text do
    previous = text
    text = text
      :gsub("/rereview/%d+/[0-9A-Fa-f]+$", "")
      :gsub("%-rereview%-%d+%-[0-9A-Fa-f]+$", "")
      :gsub("/review%-meta/%d+$", "")
      :gsub("%-review%-meta%-%d+$", "")
      :gsub("/review%-meta%-action/%d+$", "")
      :gsub("%-review%-meta%-action%-%d+$", "")
      :gsub("/review%-loop/%d+$", "")
      :gsub("%-review%-loop%-%d+$", "")
      :gsub("/review/%d+$", "")
      :gsub("%-review%-%d+$", "")
      :gsub("/fix/%d+$", "")
      :gsub("%-fix%-%d+$", "")
      :gsub("/timeout%-reconcile/[%w%-]+/%d+$", "")
      :gsub("%-timeout%-reconcile%-[%w%-]+%-%d+$", "")
      :gsub("/timeout/[%w%-]+/%d+$", "")
      :gsub("%-timeout%-[%w%-]+%-%d+$", "")
      :gsub("/reimplement/%d+$", "")
      :gsub("%-reimplement%-%d+$", "")
      :gsub("/ready%-split/%d+$", "")
      :gsub("%-ready%-split%-%d+$", "")
      :gsub("/loop/%d+$", "")
      :gsub("%-loop%-%d+$", "")
  end
  return text
end

return V
