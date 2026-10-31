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
        local entry = {title = title:text(), aliases = ctx.array(), page = page,
            kind = "entry", url = url, language = language}
        if columns.date and columns.date ~= ctx.null and row[columns.date + 1] then
            local date = row[columns.date + 1]:text()
            if date == "待定" or date == "即将推出" or date == "即將推出" then date = "COMING SOON" end
            if date ~= "" then entry.date = ctx.normalize_date(date) end
        end
        if columns.language and columns.language ~= ctx.null and row[columns.language + 1] then
            local value = row[columns.language + 1]:text()
            if value ~= "" then entry.language = value end
        end
        if columns.appid and columns.appid ~= ctx.null and row[columns.appid + 1] then
            local value = row[columns.appid + 1]:text()
            local appid = tonumber(value)
            if not value:match("^[1-9]%d*$") or not appid or appid > 4294967295 then
                error("invalid_source_appid", 0)
            end
            entry.appid = appid
        end
        return entry
    end
    function u.next_page(root, page)
        for _, anchor in ipairs(root:select("a")) do
            local next_rel = false
            for token in (anchor:attr("rel") or ""):gmatch("%S+") do if token == "next" then next_rel = true end end
            if next_rel or u.has(anchor, "next") or u.has(anchor, "nextpostslink") then
                return ctx.url_join(page, anchor:attr("href") or "")
            end
        end
    end
    return u
end
