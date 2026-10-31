-- This source covers exactly the product pages explicitly listed in the rule.
-- It does not claim to index the complete publisher catalog.
local function load(ctx, page)
    local response = ctx.get(page)
    if response.status ~= 200 then error("source_http_" .. response.status) end
    local doc = ctx.html(response.text)
    local heading = doc:select("h1")[1]
    local body = doc:select("#tab-patch")[1]
    if not heading or not body then error("product_structure_changed") end
    return response.url, heading:text(), body
end
return {
    collect_index = function(ctx, rule, previous)
        local entries = ctx.array()
        for _, page in ipairs(rule.pages) do
            local url, title = load(ctx, page)
            entries[#entries + 1] = {title = title, page = url, kind = "product"}
        end
        return {entries = entries, pending = ctx.array(), complete = true}
    end,
    resolve_page = function(ctx, rule, page)
        local url, title, body = load(ctx, page)
        local mirrors, links = ctx.array(), ctx.array()
        for _, anchor in ipairs(body:select("a[href]")) do
            local href = ctx.url_join(url, anchor:attr("href"))
            if ctx.navigation_url(href) then
                links[#links + 1] = href
                local label = anchor:text()
                if label:lower():find("mirror", 1, true) or label:lower():find("download", 1, true) then
                    mirrors[#mirrors + 1] = {url = href, label = label}
                end
            end
        end
        return {title = title, page = url, kind = "product", text = body:text(),
            steam_ids = ctx.steam_ids(links), mirrors = mirrors}
    end
}
