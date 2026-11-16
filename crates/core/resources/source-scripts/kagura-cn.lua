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

-- WordPress and WooCommerce share pagination and PageFact construction.
local function kagura(overseas)
    local function parse_fact(ctx, u, root, page, fallback)
        local heading = u.first(root, function(node)
            return (node:tag() == "h1" or node:tag() == "h2") and (overseas or u.has(node, "entry-title"))
        end)
        local title = heading and heading:text() or fallback or ""
        local body = u.first(root, function(node)
            if overseas then return node:attr("id") == "tab-patch" end
            return u.has(node, "entry-content") and u.ancestor(node, function(parent) return parent:tag() == "article" end)
        end)
        if title == "" or not body then error("discovery_structure_changed", 0) end
        local body_links = ctx.array()
        for _, href in ipairs(u.link_urls(body)) do
            local ok, url = pcall(ctx.url_join, page, href)
            if ok then body_links[#body_links + 1] = url end
        end
        local published
        local article = u.ancestor(body, function(node) return node:tag() == "article" end)
        if article then
            local time = u.first(article, function(node)
                return node:tag() == "time" and (u.has(node, "published") or u.has(node, "entry-date"))
            end)
            if time then published = ctx.date(time:attr("datetime") or "") end
        end
        local next_page = u.next_page(root, page)
        return {title = title, page = ctx.canonical_url(page), kind = overseas and "product" or "article",
            steam_ids = ctx.steam_ids(u.link_urls(body)), aliases = ctx.array(),
            mirrors = u.mirrors(body, page), text = body:text(), published_at = published,
            body_links = body_links, article_links = ctx.array(), article_facts = ctx.array(),
            next_page = next_page and ctx.canonical_url(next_page)}
    end
    local function parse_index(ctx, rule, u, root, page)
        local found = ctx.array()
        local function add(title, url, kind)
            found[#found + 1] = {title = title, page = ctx.canonical_url(url), kind = kind}
        end
        if overseas then
            for _, card in ipairs(u.walk(root)) do
                if u.has(card, "product") and u.has(card, "type-product")
                    and not u.ancestor(card, function(node) return u.has(node, "widget") or u.has(node, "related") end) then
                    local title_node = u.first(card, function(node)
                        return u.has(node, "woocommerce-loop-product__title") or u.has(node, "name")
                    end)
                    local links = u.links(card, page, function(_, url)
                        return ctx.url_parts(url).path:find("/product/", 1, true)
                    end)
                    if #links > 0 then
                        local title = title_node and title_node:text() or ""
                        if title == "" then
                            for _, link in ipairs(links) do if link.label ~= "" then title = link.label; break end end
                        end
                        if title ~= "" then add(title, links[1].url, "product") end
                    end
                end
            end
        else
            for _, article in ipairs(root:select("article")) do
                local title = u.first(article, function(node) return u.has(node, "entry-title") end)
                if title and title:text() ~= "" then
                    local links = u.links(title, page, function(_, url) return u.internal(url) end)
                    if #links > 0 then add(title:text(), links[1].url, "article") end
                end
            end
            for _, link in ipairs(u.links(root, page, function(_, url)
                return ctx.url_parts(url).path:find("/category/", 1, true)
            end)) do
                if link.label ~= "" then add(link.label, link.url, "category") end
            end
        end
        return found, u.next_page(root, page)
    end
    return {
        collect_index = function(ctx, rule, previous)
            local u = library(ctx, rule)
            local entries, pending, diagnostics = ctx.array(), ctx.array(), ctx.array()
            local seen, incomplete = {}, false
            local function enqueue(page)
                page = ctx.canonical_url(page)
                if not seen[page] then pending[#pending + 1] = page; seen[page] = true; return true end
                return false
            end
            if previous and previous ~= ctx.null and #previous.pending > 0 then
                entries = previous.entries
                for _, page in ipairs(previous.pending) do enqueue(page) end
            else
                if rule.url and rule.url ~= ctx.null then enqueue(rule.url) end
                for _, page in ipairs(rule.pages) do enqueue(page) end
            end
            while #pending > 0 do
                ctx.check_cancel()
                local page = pending[1]
                ctx.progress(page)
                local ok, root, final_url = pcall(u.fetch, page)
                if not ok then diagnostics[#diagnostics + 1] = u.diagnostic(root); incomplete = true; break end
                table.remove(pending, 1); seen[ctx.canonical_url(final_url)] = true
                local found, next_page = parse_index(ctx, rule, u, root, final_url)
                if #found == 0 then
                    diagnostics[#diagnostics + 1] = {code = "discovery_structure_changed"}; incomplete = true
                end
                for _, entry in ipairs(found) do entries[#entries + 1] = entry end
                if next_page and not enqueue(next_page) then
                    diagnostics[#diagnostics + 1] = {code = "discovery_pagination_cycle"}
                end
            end
            local unique, pages = ctx.array(), {}
            for _, entry in ipairs(entries) do
                if not pages[entry.page] then unique[#unique + 1] = entry; pages[entry.page] = true end
            end
            return {entries = unique, pending = pending, complete = #pending == 0 and not incomplete, diagnostics = diagnostics}
        end,
        resolve_page = function(ctx, rule, page)
            local u = library(ctx, rule)
            page = ctx.canonical_url(page)
            local root, final_url = u.fetch(page)
            if overseas or not ctx.url_parts(final_url).path:find("/category/", 1, true) then
                return parse_fact(ctx, u, root, final_url)
            end
            local heading = u.first(root, function(node) return node:tag() == "h1" end)
                or u.first(root, function(node) return node:tag() == "title" end)
            local links, facts = ctx.array(), ctx.array()
            for _, article in ipairs(root:select("article")) do
                local title = u.first(article, function(node) return u.has(node, "entry-title") end)
                if title then
                    local urls = u.links(title, final_url, function(_, url) return u.internal(url) end)
                    if #urls > 0 then
                        local url = ctx.canonical_url(urls[1].url)
                        links[#links + 1] = {title = title:text(), page = url}
                        -- Listings without full bodies are links, not failed details.
                        if u.first(article, function(node) return u.has(node, "entry-content") end) then
                            facts[#facts + 1] = parse_fact(ctx, u, article, url, title:text())
                        end
                    end
                end
            end
            local next_page = u.next_page(root, final_url)
            return {title = heading and heading:text() or "", page = ctx.canonical_url(final_url), kind = "category",
                steam_ids = ctx.array(), aliases = ctx.array(), mirrors = ctx.array(), text = "",
                body_links = ctx.array(), article_links = links, article_facts = facts,
                next_page = next_page and ctx.canonical_url(next_page)}
        end
    }
end

return kagura(false)
