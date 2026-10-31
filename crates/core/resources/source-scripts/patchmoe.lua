return {
    collect_index = function(ctx, rule, previous)
        local u = library(ctx, rule)
        local ok, response = pcall(ctx.get, rule.pages[1], previous ~= ctx.null and previous or nil)
        if not ok then return u.failed_index(response) end
        if response.status == 304 then
            if not previous or previous == ctx.null or #previous.entries == 0 then error("source_cache_missing", 0) end
            return {not_modified = true, complete = true, etag = previous.etag, modified = previous.modified}
        end
        if response.status ~= 200 then return u.failed_index("source_http_" .. response.status) end
        local root = ctx.html(response.text)
        local entries, forms = {}, 0
        for _, form in ipairs(root:select("form")) do
            local name = form:attr("name")
            if name == "form_en" or name == "form_zh_cn" or name == "form_zh_tw" then
                forms = forms + 1
                local fields, lang_index = ctx.array(), nil
                for _, input in ipairs(form:select("input[name]")) do
                    local field = input:attr("name")
                    if field ~= "" and (input:attr("type") or "hidden"):lower() == "hidden" then
                        fields[#fields + 1] = {field, input:attr("value") or ""}
                        if field == "lang" and not lang_index then lang_index = #fields end
                    end
                end
                if (form:attr("method") or "get"):lower() ~= "post" or not lang_index then
                    error("patchmoe_language_form_invalid", 0)
                end
                local lang = fields[lang_index][2]
                if lang == "" then
                    local quote, assigned = response.text:match(name .. "%.lang%.value%s*=%s*(['\"])(.-)%1")
                    if quote and assigned ~= "" and not assigned:find("[\"'<>]") then lang = assigned end
                end
                if lang ~= "" then
                    fields[lang_index][2] = lang
                    local action = form:attr("action") or ""
                    if action == "" then action = "/" end
                    local url = ctx.url_join(response.url ~= "" and response.url or rule.pages[1], action)
                    local fetched, localized = pcall(ctx.form, url, fields)
                    if not fetched then return u.failed_index(localized) end
                    if localized.status ~= 200 then return u.failed_index("source_http_" .. localized.status) end
                    local page = localized.url ~= "" and localized.url or url
                    local language = name == "form_en" and "en" or name == "form_zh_cn" and "zh-CN" or "zh-TW"
                    for _, row in ipairs(u.rows(ctx.html(localized.text))) do
                        local columns = {}
                        for index, cell in ipairs(row) do
                            local class = (cell:attr("class") or ""):match("%S+")
                            if class then columns[class] = index - 1 end
                        end
                        if columns.title and columns.release and columns.patch then
                            local entry = u.row_entry(row, {title = columns.title, date = columns.release, download = columns.patch}, page, language)
                            if entry then entries[#entries + 1] = entry end
                        end
                    end
                end
            end
        end
        if forms == 0 then error("patchmoe_language_forms_missing", 0) end
        if #entries == 0 then error("patchmoe_rows_missing", 0) end
        local groups, ordered = {}, {}
        for _, entry in ipairs(entries) do
            if not groups[entry.url] then groups[entry.url] = {}; ordered[#ordered + 1] = entry.url end
            local group = groups[entry.url]; group[#group + 1] = entry
        end
        local merged = ctx.array()
        for _, url in ipairs(ordered) do
            local group, chosen = groups[url], groups[url][1]
            for _, entry in ipairs(group) do if entry.language == "zh-CN" then chosen = entry; break end end
            local aliases, seen = ctx.array(), {}
            for _, entry in ipairs(group) do
                if entry.title ~= chosen.title and not seen[entry.title] then
                    aliases[#aliases + 1] = entry.title; seen[entry.title] = true
                end
            end
            table.sort(aliases); chosen.aliases = aliases; merged[#merged + 1] = chosen
        end
        return {entries = merged, pending = ctx.array(), complete = true, etag = response.etag, modified = response.modified}
    end
}
