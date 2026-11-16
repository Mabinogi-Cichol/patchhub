return {
    collect_index = function(ctx, rule)
        local entries = ctx.array()
        for _, page in ipairs(rule.pages) do
            local response = ctx.get(page)
            for _, row in ipairs(ctx.html(response.text):select("tr")) do
                local cells = row:select("td")
                local columns = rule.columns
                local appid_cell = cells[type(columns.appid) == "number" and columns.appid + 1 or 3]
                local appid = appid_cell and tonumber(appid_cell:text())
                if appid_cell and not appid then error("invalid_source_appid", 0) end
                local dl = ctx.url_join(response.url, cells[2]:select("a")[1]:attr("href"))
                local lang = type(columns.language) == "number" and cells[columns.language + 1]:text() or nil
                local published = type(columns.date) == "number" and cells[columns.date + 1]:text() or nil
                entries[#entries + 1] = {
                    title = cells[1]:text(), page = response.url, appid = appid,
                    current_step = 1, total_steps = 1,
                    details = {{
                        title = cells[1]:text(),
                        page = response.url,
                        steam_ids = appid and {appid} or ctx.array(),
                        mirrors = {{url = dl, format = "zip"}},
                        languages = lang and lang ~= "" and {lang} or ctx.array(),
                        published_at = published,
                        reported_format = "zip",
                    }}
                }
            end
        end
        return {entries = entries, complete = true}
    end
}
