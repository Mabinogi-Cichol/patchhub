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
