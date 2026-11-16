-- Example HTML: <tr data-appid="123"><td class="title">Game</td>
-- <td><a href="https://downloads.vendor.example/game.zip">Patch</a></td></tr>
return {
    collect_index = function(ctx, rule, previous)
        local entries = ctx.array()
        for _, page in ipairs(rule.pages) do
            local response = ctx.get(page)
            if response.status ~= 200 then error("source_http_" .. response.status) end
            local root = ctx.html(response.text)
            for _, row in ipairs(root:select("tr[data-appid]")) do
                local title = row:select(".title")[1]
                local link = row:select("a[href]")[1]
                if not title or not link then error("missing_title_or_download") end
                local appid = tonumber(row:attr("data-appid"))
                if not appid then error("invalid_appid") end
                local href = ctx.url_join(response.url, link:attr("href"))
                entries[#entries + 1] = {
                    title = title:text(), appid = appid,
                    page = response.url,
                    current_step = 1, total_steps = 1,
                    details = {{
                        title = title:text(),
                        page = response.url,
                        steam_ids = {appid},
                        mirrors = {{url = href}},
                        reported_format = "zip",
                    }},
                }
            end
        end
        return {entries = entries, pending = ctx.array(), complete = true}
    end
}
