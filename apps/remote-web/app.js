const el = (id) => document.getElementById(id);
const notice = el("notice");
const workspace = el("workspace");
const editor = el("editor");
const code = el("code");
let key = "";
let editing = null;
let readonly = false;
let saving = false;
let editorVersion = 0;
let listRequest = 0;
let writes = Promise.resolve();
const tasks = new Map();
function message(text, error = false) {
    notice.className = `alert mt-3 ${error ? "alert-danger" : "alert-success"}`;
    notice.textContent = text;
}
function errorText(error) {
    return error instanceof Error ? error.message : String(error);
}
async function api(path, method = "GET", body) {
    const request = async () => {
        const response = await fetch(`/api${path}`, {
            method,
            headers: {
                Authorization: `Bearer ${key}`,
                ...(body ? { "Content-Type": "application/json" } : {}),
            },
            ...(body ? { body: JSON.stringify(body) } : {}),
        });
        const data = await response.json();
        if (!response.ok) {
            throw new Error(response.status === 401 ? "访问密钥错误" : `${data.code}: ${data.detail || "操作失败"}`);
        }
        return data;
    };
    if (method === "GET")
        return request();
    const result = writes.then(request);
    writes = result.catch(() => { });
    return result;
}
function button(text, action, style = "btn") {
    const b = document.createElement("button");
    b.type = "button";
    b.className = style;
    b.textContent = text;
    b.addEventListener("click", action);
    return b;
}
function label(text, input) {
    const l = document.createElement("label");
    l.className = "form-label mt-3";
    l.textContent = text;
    l.append(input);
    return l;
}
async function refresh() {
    const request = ++listRequest;
    const data = await api("/sources");
    if (request !== listRequest)
        return;
    const container = el("sources");
    container.replaceChildren();
    for (const [role, title] of [["publisher", "索引源"], ["fallback_indexer", "补查源"]]) {
        const group = document.createElement("section");
        group.className = "mb-4";
        const heading = document.createElement("h2");
        heading.className = "h3";
        heading.textContent = title;
        group.append(heading);
        const entries = data.sources.filter(s => s.role === role);
        if (!entries.length) {
            const empty = document.createElement("p");
            empty.className = "text-secondary";
            empty.textContent = "暂无来源";
            group.append(empty);
        }
        entries.forEach(source => group.append(sourceCard(source)));
        container.append(group);
    }
}
async function mutate(path, method, body) {
    await api(path, method, body);
    try {
        await refresh();
        message("已保存");
    }
    catch (error) {
        message(`已保存，但刷新失败：${errorText(error)}。请点击刷新列表。`, true);
    }
}
function sourceCard(source) {
    const id = encodeURIComponent(source.id), path = `/sources/${id}`;
    const card = document.createElement("article");
    card.className = "card mb-2";
    const details = document.createElement("details");
    const summary = document.createElement("summary");
    summary.className = "card-body d-flex justify-content-between align-items-center gap-2";
    const name = document.createElement("strong");
    name.textContent = source.name;
    const status = document.createElement("span");
    status.className = "badge bg-blue-lt";
    status.textContent = `${source.enabled ? "已启用" : "已停用"} · ${source.status}`;
    summary.append(name, status);
    details.append(summary);
    const body = document.createElement("div");
    body.className = "card-body source-details";
    const description = document.createElement("p");
    description.className = "text-secondary source-domains";
    description.textContent = `页面域名：${source.page_hosts.join("、")}；下载域名：${source.download_hosts.join("、") || "无"} · 条目：${source.entry_count} · ${source.origin === "builtin" ? "内置" : "自定义"}`;
    body.append(description);
    const enabled = document.createElement("input");
    enabled.type = "checkbox";
    enabled.className = "form-check-input me-2";
    enabled.checked = source.enabled;
    const toggle = document.createElement("label");
    toggle.className = "form-check form-switch";
    toggle.append(enabled, document.createTextNode("启用来源"));
    body.append(toggle);
    enabled.addEventListener("change", async () => {
        try {
            await mutate(`${path}/enabled`, "PUT", { enabled: enabled.checked });
        }
        catch (error) {
            enabled.checked = !enabled.checked;
            message(errorText(error), true);
        }
    });
    const nameInput = document.createElement("input");
    nameInput.className = "form-control mt-1";
    nameInput.value = source.name;
    const nameForm = document.createElement("form");
    nameForm.append(label("名称", nameInput), button("保存名称", async () => {
        try {
            await mutate(`${path}/name`, "PUT", { name: nameInput.value });
        }
        catch (error) {
            message(errorText(error), true);
        }
    }, "btn btn-outline-primary mt-2"));
    nameForm.addEventListener("submit", e => e.preventDefault());
    body.append(nameForm);
    const interval = document.createElement("input");
    interval.className = "form-control mt-1";
    interval.type = "number";
    interval.min = "0";
    interval.step = "1";
    interval.value = String(source.interval_hours);
    if (source.role === "publisher") {
        body.append(label("自动更新间隔（小时，0 为关闭）", interval), button("保存间隔", async () => {
            if (!interval.reportValidity())
                return;
            try {
                await mutate(`${path}/interval`, "PUT", { interval_hours: Number(interval.value) });
            }
            catch (error) {
                message(errorText(error), true);
            }
        }, "btn btn-outline-primary mt-2"));
    }
    const actions = document.createElement("div");
    actions.className = "d-flex flex-wrap gap-2 mt-3";
    if (source.script) {
        actions.append(button(source.origin === "builtin" ? "查看脚本" : "编辑脚本", async () => {
            try {
                await openScript(source);
            }
            catch (error) {
                message(errorText(error), true);
            }
        }));
    }
    if (source.role === "publisher") {
        actions.append(button("更新索引", async () => {
            try {
                await startUpdate(`${path}/updates`);
            }
            catch (error) {
                message(errorText(error), true);
            }
        }));
    }
    if (source.origin !== "builtin") {
        actions.append(button("删除", async () => {
            if (!confirm(`确定删除来源“${source.name}”？`))
                return;
            try {
                await mutate(path, "DELETE");
            }
            catch (error) {
                message(errorText(error), true);
            }
        }, "btn btn-outline-danger"));
    }
    body.append(actions);
    details.append(body);
    card.append(details);
    return card;
}
async function openScript(source) {
    if (saving)
        return;
    const version = ++editorVersion;
    const doc = await api(`/sources/${encodeURIComponent(source.id)}/script`);
    if (version !== editorVersion || saving)
        return;
    editing = source.id;
    readonly = source.origin === "builtin";
    code.value = doc.code;
    code.readOnly = readonly;
    el("editor-title").textContent = readonly ? `查看脚本：${source.name}` : `编辑脚本：${source.name}`;
    el("import-tools").hidden = true;
    el("save").hidden = readonly;
    editor.hidden = false;
    editor.scrollIntoView();
}
async function startUpdate(path) {
    const result = await api(path, "POST", { force: false });
    for (const id of result.task_ids || (result.task_id ? [result.task_id] : []))
        trackTask(id);
    if (!result.task_id && !result.task_ids?.length)
        message("没有需要更新的来源");
}
function trackTask(id) {
    if (tasks.has(id))
        return;
    const box = document.createElement("div");
    box.className = "alert alert-info";
    el("tasks").append(box);
    tasks.set(id, box);
    void poll(id);
}
function taskMessage(task) {
    if (task.error) {
        if (["cancelled", "task_cancelled", "user_cancelled"].includes(task.error.code))
            return "更新已取消";
        return `任务失败：${task.error.code} ${task.error.detail || ""}`;
    }
    if (task.control_error)
        return `任务错误：${task.control_error.code} ${task.control_error.detail || ""}`;
    if (!task.running)
        return task.cancel_requested && !task.result ? "更新已取消" : "更新完成";
    const progress = task.progress;
    return `正在更新 ${progress ? `${progress.source_id} ${progress.completed}/${progress.total}` : ""}${task.cancel_requested ? "（正在取消）" : ""}`;
}
async function poll(id) {
    const box = tasks.get(id);
    try {
        const task = await api(`/tasks/${encodeURIComponent(id)}`);
        box.replaceChildren();
        const text = document.createElement("span");
        text.textContent = taskMessage(task);
        box.append(text);
        if (task.running) {
            const cancel = button("取消", () => { void api(`/tasks/${encodeURIComponent(id)}/cancel`, "POST").catch(e => message(errorText(e), true)); }, "btn btn-sm ms-3");
            cancel.disabled = task.cancel_requested;
            box.append(cancel);
            window.setTimeout(() => void poll(id), 1000);
        }
        else {
            tasks.delete(id);
            try {
                await refresh();
            }
            catch (error) {
                message(`任务已结束，但刷新来源失败：${errorText(error)}。请点击刷新列表。`, true);
            }
        }
    }
    catch (error) {
        box.textContent = `任务状态读取失败：${errorText(error)}`;
        box.append(button("重查", () => { void poll(id); }, "btn btn-sm ms-3"));
    }
}
el("login-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    key = el("key").value;
    try {
        await refresh();
        el("login").hidden = true;
        workspace.hidden = false;
        notice.textContent = "";
        notice.className = "";
    }
    catch (error) {
        key = "";
        message(errorText(error), true);
    }
});
el("refresh").addEventListener("click", async () => {
    try {
        await refresh();
        message("列表已刷新");
    }
    catch (error) {
        message(errorText(error), true);
    }
});
el("import").addEventListener("click", () => {
    if (saving)
        return;
    ++editorVersion;
    editing = null;
    readonly = false;
    code.readOnly = false;
    code.value = "";
    el("file").value = "";
    el("editor-title").textContent = "导入脚本来源";
    el("import-tools").hidden = false;
    el("prompt-box").hidden = true;
    el("save").hidden = false;
    editor.hidden = false;
    editor.scrollIntoView();
});
el("file").addEventListener("change", async (event) => {
    const file = event.target.files?.[0];
    if (!file)
        return;
    const version = ++editorVersion;
    try {
        const text = await file.text();
        if (version === editorVersion)
            code.value = text;
    }
    catch (error) {
        message(errorText(error), true);
    }
});
el("prompt-load").addEventListener("click", async () => {
    try {
        const result = await api("/script-prompt");
        el("prompt").value = result.prompt;
        el("prompt-box").hidden = false;
    }
    catch (error) {
        message(errorText(error), true);
    }
});
el("close-editor").addEventListener("click", () => {
    if (saving)
        return;
    ++editorVersion;
    editor.hidden = true;
});
el("save").addEventListener("click", async () => {
    if (saving)
        return;
    ++editorVersion;
    saving = true;
    const save = el("save");
    save.disabled = true;
    const path = editing ? `/sources/${encodeURIComponent(editing)}/script` : "/sources";
    try {
        const result = await api(path, editing ? "PUT" : "POST", { code: code.value });
        if (result.task_id)
            trackTask(result.task_id);
        editor.hidden = true;
        try {
            await refresh();
            if (result.task_error) {
                message(`脚本已保存，但首次更新未启动：${result.task_error.code} ${result.task_error.detail || ""}`, true);
            }
            else {
                message("脚本已保存");
            }
        }
        catch (error) {
            message(`脚本已保存，但刷新失败：${errorText(error)}。请点击刷新列表。`, true);
        }
    }
    catch (error) {
        message(errorText(error), true);
    }
    finally {
        saving = false;
        save.disabled = false;
    }
});
el("update-all").addEventListener("click", async () => {
    try {
        await startUpdate("/updates");
    }
    catch (error) {
        message(errorText(error), true);
    }
});
export {};
