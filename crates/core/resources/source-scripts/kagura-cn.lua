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
        local code = detail:match("(source_http_%d+)")
            or detail:match("(discovery_structure_changed)")
            or "source_refresh_failed"
        return {code = code, detail = detail}
    end
    function u.failed_index(failure)
        return {entries = ctx.array(), pending = rule.pages, complete = false, diagnostics = {u.diagnostic(failure)}}
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

local function parse_detail(ctx, u, root, page, fallback)
    local heading = u.first(root, function(node)
        return (node:tag() == "h1" or node:tag() == "h2") and u.has(node, "entry-title")
    end)
    local title = heading and heading:text() or fallback or ""
    local body = u.first(root, function(node)
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
    return {
        title = title,
        page = ctx.canonical_url(page),
        steam_ids = ctx.steam_ids(u.link_urls(body)),
        aliases = ctx.array(),
        mirrors = u.mirrors(body, page),
        text = body:text(),
        published_at = published,
        body_links = body_links,
    }
end

local function parse_index(ctx, u, root, page)
    local found = ctx.array()
    local function add(title, url, step_data)
        found[#found + 1] = {
            title = title,
            page = ctx.canonical_url(url),
            current_step = 1,
            total_steps = 2,
            step_data = step_data,
        }
    end
    for _, article in ipairs(root:select("article")) do
        local title = u.first(article, function(node) return u.has(node, "entry-title") end)
        if title and title:text() ~= "" then
            local links = u.links(title, page, function(_, url) return u.internal(url) end)
            if #links > 0 then add(title:text(), links[1].url, nil) end
        end
    end
    for _, link in ipairs(u.links(root, page, function(_, url)
        return ctx.url_parts(url).path:find("/category/", 1, true)
    end)) do
        if link.label ~= "" then add(link.label, link.url, {kind = "category"}) end
    end
    return found, u.next_page(root, page)
end

local function parse_category_page(ctx, u, root, page, details, seen_pages, pending_links, seen_links)
    for _, article in ipairs(root:select("article")) do
        local title = u.first(article, function(node) return u.has(node, "entry-title") end)
        if title then
            local urls = u.links(title, page, function(_, url) return u.internal(url) end)
            if #urls > 0 then
                local url = ctx.canonical_url(urls[1].url)
                if u.first(article, function(node) return u.has(node, "entry-content") end) then
                    if not seen_pages[url] then
                        seen_pages[url] = true
                        details[#details + 1] = parse_detail(ctx, u, article, url, title:text())
                    end
                elseif not seen_pages[url] and not seen_links[url] then
                    seen_links[url] = true
                    pending_links[#pending_links + 1] = {title = title:text(), page = url}
                end
            end
        end
    end
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
            local found, next_page = parse_index(ctx, u, root, final_url)
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
    resolve_detail = function(ctx, rule, state)
        local u = library(ctx, rule)
        local details = state.details or ctx.array()
        local diagnostics = ctx.array()
        local seen_pages = {}
        for _, detail in ipairs(details) do seen_pages[ctx.canonical_url(detail.page)] = true end

        if state.current_step == 1 then
            local page = ctx.canonical_url(state.page)
            local root, final_url = u.fetch(page)
            local step_kind = state.step_data and state.step_data ~= ctx.null and state.step_data.kind
            local is_category = step_kind == "category" or ctx.url_parts(final_url).path:find("/category/", 1, true)
            if not is_category then
                return {
                    current_step = 2,
                    total_steps = 2,
                    details = {parse_detail(ctx, u, root, final_url, state.title)},
                }
            end
            local pending_links, seen_links = ctx.array(), {}
            parse_category_page(ctx, u, root, final_url, details, seen_pages, pending_links, seen_links)
            local canon_final = ctx.canonical_url(final_url)
            local visited_pages = {canon_final}
            local pending_pages = ctx.array()
            local next_page = u.next_page(root, final_url)
            if next_page then
                local canon_next = ctx.canonical_url(next_page)
                if canon_next == canon_final then
                    diagnostics[#diagnostics + 1] = {code = "discovery_pagination_cycle"}
                else
                    pending_pages[#pending_pages + 1] = canon_next
                end
            end
            if #pending_links == 0 and #pending_pages == 0 then
                return {
                    current_step = 2,
                    total_steps = 2,
                    details = details,
                    diagnostics = diagnostics,
                }
            end
            return {
                current_step = 2,
                total_steps = 3,
                step_data = {
                    kind = "category",
                    pending_pages = pending_pages,
                    visited_pages = visited_pages,
                    pending_links = pending_links,
                },
                details = details,
                diagnostics = diagnostics,
            }
        end

        local data = state.step_data and state.step_data ~= ctx.null and state.step_data or {}
        local pending_pages = data.pending_pages or ctx.array()
        local visited_pages = data.visited_pages or ctx.array()
        local pending_links = data.pending_links or ctx.array()
        local visited, seen_links = {}, {}
        for _, page in ipairs(visited_pages) do visited[ctx.canonical_url(page)] = true end
        for _, link in ipairs(pending_links) do seen_links[ctx.canonical_url(link.page)] = true end

        local function incomplete_step(failure)
            diagnostics[#diagnostics + 1] = u.diagnostic(failure)
            return {
                current_step = state.current_step,
                total_steps = 3,
                step_data = {
                    kind = "category",
                    pending_pages = pending_pages,
                    visited_pages = visited_pages,
                    pending_links = pending_links,
                },
                details = details,
                diagnostics = diagnostics,
            }
        end

        while #pending_pages > 0 do
            local cat_page = ctx.canonical_url(pending_pages[1])
            if visited[cat_page] then
                table.remove(pending_pages, 1)
                diagnostics[#diagnostics + 1] = {code = "discovery_pagination_cycle"}
            else
                ctx.check_cancel()
                local ok, err = pcall(function()
                    local root, final_url = u.fetch(cat_page)
                    visited[cat_page] = true
                    visited_pages[#visited_pages + 1] = cat_page
                    local canon_final = ctx.canonical_url(final_url)
                    visited[canon_final] = true
                    parse_category_page(ctx, u, root, final_url, details, seen_pages, pending_links, seen_links)
                    local next_page = u.next_page(root, final_url)
                    table.remove(pending_pages, 1)
                    if next_page then
                        local canon_next = ctx.canonical_url(next_page)
                        if visited[canon_next] then
                            diagnostics[#diagnostics + 1] = {code = "discovery_pagination_cycle"}
                        else
                            local queued = false
                            for _, queued_page in ipairs(pending_pages) do
                                if ctx.canonical_url(queued_page) == canon_next then queued = true; break end
                            end
                            if not queued then pending_pages[#pending_pages + 1] = canon_next end
                        end
                    end
                end)
                if not ok then return incomplete_step(err) end
            end
        end

        while #pending_links > 0 do
            local link = pending_links[1]
            local link_page = ctx.canonical_url(link.page)
            if seen_pages[link_page] then
                table.remove(pending_links, 1)
            else
                ctx.check_cancel()
                local ok, err = pcall(function()
                    local root, final_url = u.fetch(link_page)
                    local detail = parse_detail(ctx, u, root, final_url, link.title)
                    table.remove(pending_links, 1)
                    seen_pages[detail.page] = true
                    details[#details + 1] = detail
                end)
                if not ok then return incomplete_step(err) end
            end
        end

        return {
            current_step = 3,
            total_steps = 3,
            details = details,
            diagnostics = diagnostics,
        }
    end
}
