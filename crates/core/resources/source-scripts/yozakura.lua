local function steam_ids(ctx, u, body, page)
    local ids = ctx.steam_ids(u.link_urls(body))
    -- Post 150 currently links to the store page for post 161.
    if page == "https://yozakuragames.com/?p=150" and #ids == 1 and ids[1] == 4370020 then
        ids[1] = 4376090
    end
    return ids
end

local function downloads(ctx, u, body, page)
    return u.links(body, page, function(_, url)
        local parts = ctx.url_parts(url)
        local path = parts.path:lower()
        return parts.host == "www.zakuragames.com" and (path:match("%.zip$") or path:match("%.exe$"))
    end)
end

return {
    collect_index = function(ctx, rule)
        local u, entries = library(ctx, rule), ctx.array()
        local offset, page_size = 0, 100
        while true do
            ctx.check_cancel()
            local url = rule.pages[1] .. "&offset=" .. offset
            ctx.progress(url)
            local response = ctx.get(url)
            if response.status ~= 200 then return u.failed_index("source_http_" .. response.status) end
            if not response.text:match("^%s*%[") then error("discovery_structure_changed", 0) end
            local posts = ctx.json_decode(response.text)
            for _, post in ipairs(posts) do
                local page = ctx.canonical_url(post.link)
                local body = ctx.html(post.content.rendered)
                if #downloads(ctx, u, body, page) > 0 then
                    local ids = steam_ids(ctx, u, body, page)
                    entries[#entries + 1] = {title = ctx.html(post.title.rendered):text(),
                        page = page, kind = "article", appid = #ids == 1 and ids[1] or nil}
                end
            end
            if #posts < page_size then break end
            offset = offset + page_size
        end
        return {entries = entries, pending = ctx.array(), complete = true}
    end,
    resolve_page = function(ctx, rule, page)
        local u = library(ctx, rule)
        local root, final_url = u.fetch(page)
        local heading = root:select(".h-page-title__outer h1")[1]
        local body = root:select(".colibri-post-content")[1]
        if not heading or not body then error("discovery_structure_changed", 0) end
        page = ctx.canonical_url(final_url)
        return {title = heading:text(), page = page, kind = "article", text = body:text(),
            steam_ids = steam_ids(ctx, u, body, page), mirrors = downloads(ctx, u, body, page),
            single_download_group = true}
    end
}
