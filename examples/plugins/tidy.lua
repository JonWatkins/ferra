-- tidy.lua: an example of decorations in a ferra plugin.
--
-- Marks trailing whitespace with a red background, and after any line
-- longer than your text-width option, says how far over it is (in grey,
-- after the line). Both follow your edits; :tidy turns the marks off and
-- on again.
--
-- Install it by copying (or symlinking) this file into
-- ~/.config/ferra/plugins/.

local enabled = true

-- Puts the marks on the buffer (after taking the old ones away).
local function mark(ctx)
  ctx:clear_decorations("tidy")
  if not enabled then
    return
  end
  local limit = ctx:option("text-width")
  for row = 1, ctx:line_count() do
    local line = ctx:line(row)
    -- Trailing spaces and tabs: a highlighted range, columns as
    -- string.find gives them (the last one included).
    local from, to = line:find("[ \t]+$")
    if from and from > 1 then
      ctx:decorate({ group = "tidy", line = row, col = from, to_col = to, style = "ui.decoration.error" })
    end
    -- Too long: text after the line.
    local length = utf8.len(line) or #line
    if length > limit then
      ctx:decorate({
        group = "tidy",
        line = row,
        text = string.format("← %d over %d", length - limit, limit),
        end_of_line = true,
        style = "ui.decoration.text",
      })
    end
  end
end

editor.on("opened", mark)
editor.on("buffer-changed", mark)

editor.command("tidy", "Show or hide trailing whitespace and long lines", function(ctx)
  enabled = not enabled
  mark(ctx)
  ctx:message(enabled and "Tidy marks on" or "Tidy marks off")
end)
