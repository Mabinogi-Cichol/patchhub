-- Shared text/DOM helpers. No site routing or host permissions live here.
local function library(ctx, rule)
    local u = {}
    function u.walk(node)
        local nodes = {node}
        for _, child in ipairs(node:select("*")) do nodes[#nodes + 1] = child end
        return nodes
    end
    function u.first(node, predicate)
        for _, child in ipairs(u.walk(node)) do if predicate(child) then return child end end
    end
    function u.has(node, class)
        for token in (node:attr("class") or ""):gmatch("%S+") do
            if token == class then return true end
        end
        return false
    end
    function u.ancestor(node, predicate)
        local parent = node:parent()
        while parent do
            if predicate(parent) then return parent end
            parent = parent:parent()
        end
    end
    function u.url(base, href)
        local ok, value = pcall(ctx.url_join, base, href)
        if ok and ctx.navigation_url(value) then return value end
    end
    function u.internal(url)
        local host = ctx.url_parts(url).host
        for _, allowed in ipairs(rule.page_hosts) do if host == allowed then return true end end
        return false
    end
    function u.links(node, page, predicate)
        local links, seen = ctx.array(), {}
        for _, anchor in ipairs(node:select("a[href]")) do
            local url = u.url(page, anchor:attr("href"))
            local label = anchor:text()
            if url and not seen[url] and (not predicate or predicate(label, url)) then
                links[#links + 1] = {label = label, url = url}; seen[url] = true
            end
        end
        return links
    end
    function u.link_urls(node)
        local links = ctx.array()
        for _, child in ipairs(u.walk(node)) do
            local value
            if child:tag() == "a" then value = child:attr("href")
            elseif child:tag() == "iframe" then value = child:attr("src") end
            if value then links[#links + 1] = value end
        end
        return links
    end
    function u.texts(node)
        local texts = ctx.array()
        for _, child in ipairs(u.walk(node)) do texts[#texts + 1] = child:text() end
        return texts
    end
    function u.mirrors(node, page)
        return u.links(node, page, function(label)
            local lower = label:lower()
            local mirror = false
            local from = 1
            while true do
                local start, finish = lower:find("mirror%s*%d", from)
                if not start then break end
                local before = start > 1 and utf8.offset(label, -1, start) or start
                if ctx.word_contains(label:sub(before, start - 1) .. "mirror", "mirror") then mirror = true; break end
                from = finish + 1
            end
            local positive = mirror
                or label:find("下载", 1, true) or label:find("下載", 1, true)
                or label:find("载点", 1, true) or label:find("載点", 1, true)
                or label:find("载點", 1, true) or label:find("載點", 1, true)
                or lower:find("download", 1, true)
            return positive and not (label:find("安装说明", 1, true)
                or label:find("安裝說明", 1, true) or lower:find("instructions", 1, true))
        end)
    end
    function u.fetch(url, validators)
        local response = ctx.get(url, validators)
        if response.status ~= 200 then error("source_http_" .. response.status, 0) end
        return ctx.html(response.text), response.url ~= "" and response.url or url, response
    end
    function u.diagnostic(failure)
        local detail = tostring(failure)
        local code = detail:match("(source_http_%d+)") or "source_refresh_failed"
        return {code = code, detail = detail}
    end
    function u.failed_index(failure)
        return {entries = ctx.array(), pending = rule.pages, complete = false, diagnostics = {u.diagnostic(failure)}}
    end
    function u.rows(root, table_id)
        local tables, rows = {}, {}
        for _, node in ipairs(root:select("table")) do
            if table_id then
                if node:attr("id") == table_id then tables[1] = node; break end
            elseif not u.ancestor(node, function(parent) return parent:tag() == "table" end) then
                tables[#tables + 1] = node
            end
        end
        for _, node in ipairs(tables) do
            for _, row in ipairs(node:select("tr")) do
                local cells = row:select("td, th")
                if #cells > 0 then rows[#rows + 1] = cells end
            end
        end
        return rows
    end
    function u.row_entry(row, columns, page, language)
        local title = row[columns.title + 1]
        local download = row[columns.download + 1]
        local anchor = download and download:select("a[href]")[1]
        if not title or title:text() == "" or not anchor or anchor:attr("href") == "" then return end
        local ok, url = pcall(ctx.url_join, page, anchor:attr("href"))
        if not ok or not ctx.url_parts(url).path:lower():match("%.zip$") then return end
        local row_title = title:text()
        local published_at
        if columns.date and columns.date ~= ctx.null and row[columns.date + 1] then
            local date = row[columns.date + 1]:text()
            if date == "待定" or date == "即将推出" or date == "即將推出" then date = "COMING SOON" end
            if date ~= "" then published_at = ctx.normalize_date(date) end
        end
        local lang = language
        if columns.language and columns.language ~= ctx.null and row[columns.language + 1] then
            local value = row[columns.language + 1]:text()
            if value ~= "" then lang = value end
        end
        local appid
        if columns.appid and columns.appid ~= ctx.null and row[columns.appid + 1] then
            local value = row[columns.appid + 1]:text()
            local parsed = tonumber(value)
            if not value:match("^[1-9]%d*$") or not parsed or parsed > 4294967295 then
                error("invalid_source_appid", 0)
            end
            appid = parsed
        end
        return {
            title = row_title,
            aliases = ctx.array(),
            page = page,
            appid = appid,
            current_step = 1,
            total_steps = 1,
            details = {
                {
                    title = row_title,
                    page = page,
                    steam_ids = appid and {appid} or ctx.array(),
                    aliases = ctx.array(),
                    mirrors = {{url = url, format = "zip"}},
                    text = "",
                    languages = lang and lang ~= "" and {lang} or ctx.array(),
                    published_at = published_at,
                    body_links = ctx.array(),
                    reported_format = "zip",
                }
            },
        }
    end
    return u
end

local function verification(page)
    local text = page:lower()
    if text:find("please wait while your request is being verified", 1, true)
        or text:find("正在验证您的请求", 1, true) then
        return "source_browser_verification_required"
    end
    if text:find("age verification", 1, true) and text:find("18 years", 1, true) then
        return "source_age_verification_required"
    end
end

local function collect_index(ctx, rule, previous, get, submit)
    local u = library(ctx, rule)
    local ok, response = pcall(get, rule.pages[1], previous ~= ctx.null and previous or nil)
    if not ok then return u.failed_index(response) end
    if response.status == 304 then
        if not previous or previous == ctx.null or #previous.entries == 0 then error("source_cache_missing", 0) end
        return {not_modified = true, complete = true, etag = previous.etag, modified = previous.modified}
    end
    local function waiting(code)
        return {entries = ctx.array(), pending = rule.pages, complete = false,
            diagnostics = {{code = code}}}
    end
    local blocked = verification(response.text)
    if blocked then return waiting(blocked) end
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
                local fetched, localized = pcall(submit, url, fields)
                if not fetched then return u.failed_index(localized) end
                blocked = verification(localized.text)
                if blocked then return waiting(blocked) end
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
        local url = entry.details[1].mirrors[1].url
        if not groups[url] then groups[url] = {}; ordered[#ordered + 1] = url end
        local group = groups[url]; group[#group + 1] = entry
    end
    local merged = ctx.array()
    for _, url in ipairs(ordered) do
        local group, chosen = groups[url], groups[url][1]
        for _, entry in ipairs(group) do
            if entry.details[1].languages[1] == "zh-CN" then chosen = entry; break end
        end
        local aliases, seen = ctx.array(), {}
        for _, entry in ipairs(group) do
            if entry.title ~= chosen.title and not seen[entry.title] then
                aliases[#aliases + 1] = entry.title; seen[entry.title] = true
            end
        end
        table.sort(aliases)
        chosen.aliases = aliases
        chosen.details[1].aliases = aliases
        merged[#merged + 1] = chosen
    end
    return {entries = merged, pending = ctx.array(), complete = true, etag = response.etag, modified = response.modified}
end

return {
    meta = {
        name = "Shiravune",
        pages = {"https://www.patch.moe/?flag=1"},
        page_hosts = {"www.patch.moe"},
        download_hosts = {"download.patch.moe"},
        official_title_match = true,
    },
    collect_index = function(ctx, rule, previous)
        local result = collect_index(ctx, rule, previous, ctx.get, ctx.form)
        local diagnostic = result.diagnostics and result.diagnostics[1]
        if ctx.browser_available and diagnostic and diagnostic.code == "source_browser_verification_required" then
            local function browser_page(url)
                local response = ctx.browser_get(url)
                while verification(response.text) == "source_browser_verification_required" do
                    ctx.check_cancel()
                    response = ctx.browser_snapshot(response.url)
                end
                return response
            end
            return collect_index(ctx, rule, ctx.null, browser_page, ctx.browser_form)
        end
        return result
    end
}
