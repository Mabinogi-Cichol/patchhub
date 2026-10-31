return {
    collect_index = function(ctx, rule, previous)
        local u, entries = library(ctx, rule), ctx.array()
        if not rule.columns or rule.columns == ctx.null then error("invalid_source_columns", 0) end
        local validators = #rule.pages == 1 and previous ~= ctx.null and previous or nil
        local etag, modified
        for _, page in ipairs(rule.pages) do
            local ok, response = pcall(ctx.get, page, validators)
            if not ok then return u.failed_index(response) end
            if response.status == 304 then
                if not previous or previous == ctx.null or #previous.entries == 0 then error("source_cache_missing", 0) end
                return {not_modified = true, complete = true, etag = previous.etag, modified = previous.modified}
            end
            if response.status ~= 200 then return u.failed_index("source_http_" .. response.status) end
            local base = response.url ~= "" and response.url or page
            for _, row in ipairs(u.rows(ctx.html(response.text), rule.table_id ~= ctx.null and rule.table_id or nil)) do
                local entry = u.row_entry(row, rule.columns, base, rule.language ~= ctx.null and rule.language or "en")
                if entry then entries[#entries + 1] = entry end
            end
            if #rule.pages == 1 then etag = response.etag; modified = response.modified end
        end
        if #entries == 0 then error("empty_or_oversized_source", 0) end
        return {entries = entries, pending = ctx.array(), complete = true, etag = etag, modified = modified}
    end
}
