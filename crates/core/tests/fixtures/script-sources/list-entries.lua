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
                entries[#entries + 1] = {
                    title = cells[1]:text(), page = response.url, kind = "entry",
                    url = cells[2]:select("a")[1]:attr("href"), appid = appid,
                    date = type(columns.date) == "number" and cells[columns.date + 1]:text() or nil,
                    language = type(columns.language) == "number" and cells[columns.language + 1]:text() or nil
                }
            end
        end
        return {entries = entries, complete = true}
    end
}
