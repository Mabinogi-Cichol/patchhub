-- P0 API experiment, not the complete maintained Kagura adapter.
-- Site logic stays here; the host supplies HTTP, DOM and URL primitives.
local function fetch(ctx, page)
    local response = ctx.get(page)
    if response.status ~= 200 then error("source_http_" .. response.status) end
    return ctx.html(response.text), response.url
end

local function next_page(ctx, root, page)
    local links = root:select("a[rel~=next], a.next, a.nextpostslink")
    if #links > 0 then return ctx.url_join(page, links[1]:attr("href")) end
end

local function ancestor_class(node, class)
    local parent = node:parent()
    while parent do
        local classes = " " .. (parent:attr("class") or "") .. " "
        if classes:find(" " .. class .. " ", 1, true) then return true end
        parent = parent:parent()
    end
    return false
end

local function collect_index(ctx, rule, previous)
    local entries, pending, seen, pages = {}, {}, {}, {}
    if previous and previous ~= ctx.null and previous.pending and #previous.pending > 0 then
        entries = previous.entries
        for _, page in ipairs(previous.pending) do
            pending[#pending + 1] = page; seen[page] = true
        end
    else
        for _, page in ipairs(rule.pages) do
            if not seen[page] then pending[#pending + 1] = page; seen[page] = true end
        end
    end
    for _, entry in ipairs(entries) do pages[entry.page] = true end
    while #pending > 0 do
        ctx.check_cancel()
        local page = pending[1]
        ctx.progress(page)
        local root, final_url = fetch(ctx, page)
        table.remove(pending, 1)
        seen[final_url] = true
        for _, card in ipairs(root:select(".product.type-product")) do
            if not ancestor_class(card, "widget") and not ancestor_class(card, "related") then
                local titles = card:select(".woocommerce-loop-product__title, .name")
                local links = card:select("a[href]")
                local page_url
                for _, link in ipairs(links) do
                    local url = ctx.url_join(final_url, link:attr("href"))
                    if url:find("/product/", 1, true) then page_url = url; break end
                end
                if #titles > 0 and page_url and not pages[page_url] then
                    entries[#entries + 1] = { title = titles[1]:text(), page = page_url, kind = "product" }
                    pages[page_url] = true
                end
            end
        end
        local next_url = next_page(ctx, root, final_url)
        if next_url and not seen[next_url] then
            seen[next_url] = true; pending[#pending + 1] = next_url
        end
    end
    return { entries = entries, pending = pending, complete = true }
end

local function resolve_page(ctx, rule, page)
    local root, final_url = fetch(ctx, page)
    local headings = root:select("h1, h2")
    local bodies = root:select("#tab-patch")
    assert(#headings > 0 and #bodies > 0, "discovery_structure_changed")
    local body = bodies[1]
    local mirrors, links, steam_ids, seen_mirrors, seen_ids = {}, {}, {}, {}, {}
    for _, link in ipairs(body:select("a[href], iframe[src]")) do
        local href = link:attr("href") or link:attr("src")
        local url = ctx.url_join(final_url, href)
        links[#links + 1] = url
        local appid = url:match("https?://store%.steampowered%.com/app/(%d+)")
        if appid and not seen_ids[appid] then
            seen_ids[appid] = true; steam_ids[#steam_ids + 1] = tonumber(appid)
        end
        local label = link:text()
        local lower = label:lower()
        if (lower:match("mirror%s*%d") or lower:find("download", 1, true))
            and not lower:find("instructions", 1, true) and not seen_mirrors[url] then
            local path = url:match("^[^?]+") or url
            local format = path:lower():match("%.(exe)$") or path:lower():match("%.(zip)$") or "web"
            mirrors[#mirrors + 1] = { url = url, label = label, format = format }
            seen_mirrors[url] = true
        end
    end
    return { title = headings[1]:text(), page = final_url, kind = "product", text = body:text(),
        mirrors = mirrors, steam_ids = steam_ids, body_links = links }
end

return { collect_index = collect_index, resolve_page = resolve_page }
