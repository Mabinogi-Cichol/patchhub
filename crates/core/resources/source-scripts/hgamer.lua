return {
    discover = function(ctx, rule, query, previous)
        local u = library(ctx, rule)
        local diagnostics, games, patches = ctx.array(), {}, {}
        local function push(list, page, label)
            for _, item in ipairs(list) do if item.url == page then item.label = label; return end end
            list[#list + 1] = {url = page, label = label}
        end
        local function detail_path(url, kind)
            local path = ctx.url_parts(url).path
            return path:match("^/zh%-%u%u/" .. kind .. "/[^/]+/?$") ~= nil
        end
        local function result_count(text)
            local lower = text:lower()
            for _, marker in ipairs({"找到", "found"}) do
                local from = 1
                while true do
                    local _, finish = lower:find(marker, from, true)
                    if not finish then break end
                    local digits, after = lower:sub(finish + 1):match("^%s*(%d+)%s*(.*)")
                    if digits and (after:sub(1, #"笔") == "笔" or after:sub(1, #"筆") == "筆" or after:sub(1, 6) == "result") then
                        return tonumber(digits)
                    end
                    from = finish + 1
                end
            end
        end
        local function result_summary(root, term)
            for _, node in ipairs(root:select("p")) do
                local text = node:text()
                if text:find(term, 1, true) then
                    local count = result_count(text); if count then return count end
                end
            end
            local text = root:text()
            if text:find(term, 1, true) then return result_count(text) end
        end
        local function fetch(page)
            ctx.check_cancel()
            local ok, root, final_url = pcall(u.fetch, page)
            if not ok then diagnostics[#diagnostics + 1] = u.diagnostic(root); return end
            return root, final_url
        end
        local base = rule.url and rule.url ~= ctx.null and rule.url or rule.pages[1]
        for _, term in ipairs(query.terms) do
            local encoded = term:gsub("([^%w%*%.%-%_ ])", function(c) return string.format("%%%02X", c:byte()) end):gsub(" ", "+")
            local root, final_url = fetch(ctx.url_join(base, "/zh-CN/search") .. "?q=" .. encoded)
            if root then
                local count = result_summary(root, term)
                if not count then diagnostics[#diagnostics + 1] = {code = "discovery_search_structure_changed"}
                else
                    local identified = 0
                    for _, heading in ipairs(root:select("h2")) do
                        local title = heading:text()
                        local kind
                        if title == "游戏" or title == "遊戲" then kind = "games"
                        elseif title == "补丁" or title == "補丁" then kind = "patches"
                        elseif title == "文章" or title == "新聞" or title == "新闻" or title == "Articles" then kind = "posts" end
                        local parent = heading:parent()
                        local group = parent and parent:parent()
                        if kind and group then
                            for _, link in ipairs(u.links(group, final_url, function(_, url)
                                return u.internal(url) and detail_path(url, kind)
                            end)) do
                                identified = identified + 1
                                if kind == "games" then push(games, link.url, link.label)
                                elseif kind == "patches" then push(patches, link.url, link.label) end
                            end
                        end
                    end
                    if identified < count then diagnostics[#diagnostics + 1] = {code = "discovery_search_structure_changed"} end
                end
            end
        end
        local identities = ctx.array()
        for _, game in ipairs(games) do
            local root, page = fetch(game.url)
            if root then
                local body = u.first(root, function(node) return node:attr("data-game-body") ~= nil end)
                local heading = body and u.first(body, function(node) return node:tag() == "h1" end)
                local primary = heading and heading:parent()
                if not primary then diagnostics[#diagnostics + 1] = {code = "discovery_structure_changed"}
                else
                    local confirmed = false
                    for _, id in ipairs(ctx.steam_ids(u.link_urls(primary))) do if id == query.appid then confirmed = true end end
                    if confirmed then
                        identities[#identities + 1] = {appid = query.appid, page = page, aliases = ctx.aliases(heading:text(), u.texts(primary))}
                        for _, link in ipairs(u.links(body, page, function(_, url)
                            return u.internal(url) and detail_path(url, "patches")
                        end)) do push(patches, link.url, link.label) end
                    end
                end
            end
        end
        local pages = ctx.array()
        for _, patch in ipairs(patches) do
            local root, page = fetch(patch.url)
            if root then
                local body = u.first(root, function(node) return node:attr("data-patch-body") ~= nil end)
                local heading = body and u.first(body, function(node) return node:tag() == "h1" end)
                local primary = heading and heading:parent()
                if not primary then diagnostics[#diagnostics + 1] = {code = "discovery_structure_changed"}
                else
                    local urls = u.link_urls(primary)
                    for _, h2 in ipairs(body:select("h2")) do
                        local text = h2:text()
                        if text == "補丁說明" or text == "补丁说明" or text == "安裝教學" or text == "安装教学" then
                            local parent = h2:parent()
                            local description = parent and parent:parent()
                            if description then
                                for _, url in ipairs(u.link_urls(description)) do urls[#urls + 1] = url end
                                local column = description:parent()
                                if column then
                                    for _, child in ipairs(column:children()) do
                                        if child:tag() == "iframe" or child:tag() == "a" then
                                            for _, url in ipairs(u.link_urls(child)) do urls[#urls + 1] = url end
                                        end
                                    end
                                end
                            end
                        end
                    end
                    local ids = ctx.steam_ids(urls)
                    local aliases = ctx.aliases(heading:text(), u.texts(primary))
                    if ctx.game_title(heading:text()) == "" then
                        local found = false
                        for _, identity in ipairs(identities) do
                            for _, id in ipairs(ids) do
                                if identity.appid == id then aliases = identity.aliases; found = true; break end
                            end
                            if found then break end
                        end
                    end
                    local text, mirrors = primary:text(), u.mirrors(primary, page)
                    local reported = ctx.word_contains(text, "ZIP") and "zip" or ctx.word_contains(text, "EXE") and "exe" or "unknown"
                    local upstream
                    for _, mirror in ipairs(mirrors) do if not u.internal(mirror.url) then upstream = mirror.url; break end end
                    pages[#pages + 1] = {fact = {title = heading:text(), page = page, kind = "article",
                        text = text, steam_ids = ids, aliases = aliases, mirrors = mirrors},
                        reported_format = reported, upstream_page = upstream}
                end
            end
        end
        if #pages == 0 and #diagnostics > 0 then error(ctx.json_encode(diagnostics), 0) end
        return {pages = pages, identities = identities, diagnostics = diagnostics, complete = #diagnostics == 0}
    end
}
