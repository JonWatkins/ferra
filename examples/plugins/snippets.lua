-- snippets.lua: an example of asking the user from a ferra plugin, with a
-- picker (ctx:pick) and an input box (ctx:input).
--
-- <Space>i picks a snippet to insert at the cursor (type to filter).
-- <Space>w asks what to wrap the word under the cursor in (`**`, `"`...).
--
-- Install it by copying (or symlinking) this file into
-- ~/.config/ferra/plugins/.

local snippets = {
  { label = "todo", detail = "a TODO comment", text = "// TODO: " },
  { label = "fn", detail = "a Rust function", text = "fn name() {\n}" },
  { label = "test", detail = "a Rust test", text = "#[test]\nfn it_works() {\n}" },
  { label = "date", detail = "today's date", text = function() return os.date("%Y-%m-%d") end },
}

editor.command("snippet", "Pick a snippet to insert at the cursor", function(ctx)
  local items = {}
  for _, snippet in ipairs(snippets) do
    -- `value` is what the chosen item hands to on_choose.
    table.insert(items, { label = snippet.label, detail = snippet.detail, value = snippet.label })
  end
  ctx:pick({ title = "Snippets", items = items, on_choose = "snippet_insert" })
end)

editor.command("snippet_insert", "Insert the snippet named", function(ctx)
  for _, snippet in ipairs(snippets) do
    if snippet.label == ctx.args[1] then
      local text = snippet.text
      if type(text) == "function" then text = text() end
      ctx:insert(text)
      return
    end
  end
  error("no snippet named " .. tostring(ctx.args[1]), 0)
end)

editor.command("wrap_word", "Wrap the word under the cursor in some text", function(ctx)
  ctx:input({
    title = "Wrap the word in",
    value = '"',
    placeholder = "e.g. ** or `",
    on_submit = "wrap_word_in",
  })
end)

editor.command("wrap_word_in", "Wrap the word under the cursor in the text given", function(ctx)
  local wrap = ctx.args[1] or ""
  if wrap == "" then return end
  local row, col = ctx:cursor()
  local line = ctx:line(row)
  -- The word around the cursor (columns start at 1).
  local first, last = col, col
  while first > 1 and line:sub(first - 1, first - 1):match("[%w_]") do first = first - 1 end
  while last < #line and line:sub(last + 1, last + 1):match("[%w_]") do last = last + 1 end
  if not line:sub(first, last):match("^[%w_]+$") then
    error("no word under the cursor", 0)
  end
  ctx:set_line(row, line:sub(1, first - 1) .. wrap .. line:sub(first, last) .. wrap .. line:sub(last + 1))
  ctx:set_cursor(row, last + #wrap * 2)
end)

editor.map("normal", "<Space>i", "snippet")
editor.map("normal", "<Space>w", "wrap_word")
