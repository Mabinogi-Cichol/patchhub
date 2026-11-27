const el = (id) => document.getElementById(id);
const notice = el("notice");
const workspace = el("workspace");
const editor = el("editor");
const editorNotice = el("editor-notice");
const code = el("code");
const prompt = el("prompt");
const promptBox = el("prompt-box");
const copyPrompt = el("prompt-copy");
const promptFeedback = el("prompt-feedback");
let key = "";
let editing = null;
let readonly = false;
let saving = false;
let editorVersion = 0;
let listRequest = 0;
let writes = Promise.resolve();
const tasks = new Map();
function message(text, error = false, target = notice) {
    target.className = `alert mt-3 ${error ? "alert-danger" : "alert-success"}`;
    target.textContent = text;
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
                const result = await api(`${path}/updates`, "POST", { force: false });
                trackTask({ source_id: source.id, task_id: result.task_id });
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
                tasks.get(source.id)?.box.remove();
                tasks.delete(source.id);
            }
            catch (error) {
                message(errorText(error), true);
            }
        }, "btn btn-outline-danger"));
    }
    body.append(actions);
    details.append(body);
    card.append(details);
    if (source.role === "publisher") {
        const taskSlot = document.createElement("div");
        taskSlot.dataset.sourceTask = source.id;
        const task = tasks.get(source.id);
        if (task)
            taskSlot.append(task.box);
        card.append(taskSlot);
    }
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
    showEditor();
}
function showEditor() {
    editorNotice.textContent = "";
    editorNotice.className = "";
    editor.showModal();
}
function showTaskStatus(sourceId, id, status, error = false) {
    const box = document.createElement("div");
    box.className = `source-task alert ${error ? "alert-danger" : "alert-info"}`;
    box.setAttribute("role", "status");
    box.textContent = status;
    tasks.set(sourceId, { id, box });
    const slot = document.querySelector(`[data-source-task="${CSS.escape(sourceId)}"]`);
    slot?.replaceChildren(box);
}
function trackTask({ source_id, task_id }) {
    if (tasks.get(source_id)?.id === task_id)
        return;
    showTaskStatus(source_id, task_id, "正在更新");
    void poll(source_id, task_id);
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
    return `正在更新${progress ? ` ${progress.completed}/${progress.total}` : ""}${task.cancel_requested ? "（正在取消）" : ""}`;
}
async function poll(sourceId, id) {
    const current = tasks.get(sourceId);
    if (current?.id !== id)
        return;
    const { box } = current;
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
            window.setTimeout(() => void poll(sourceId, id), 1000);
        }
        else {
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
        box.append(button("重查", () => { void poll(sourceId, id); }, "btn btn-sm ms-3"));
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
    promptBox.open = false;
    promptFeedback.replaceChildren();
    promptFeedback.className = "";
    el("save").hidden = false;
    showEditor();
    if (!prompt.value && !copyPrompt.disabled)
        void loadPrompt();
});
async function loadPrompt() {
    promptFeedback.replaceChildren();
    promptFeedback.className = "";
    promptBox.hidden = true;
    copyPrompt.disabled = true;
    copyPrompt.textContent = "正在读取 AI 提示词…";
    try {
        const result = await api("/script-prompt");
        prompt.value = result.prompt;
        promptBox.hidden = false;
        copyPrompt.textContent = "复制 AI 提示词";
    }
    catch (error) {
        copyPrompt.textContent = "重试读取 AI 提示词";
        if (editor.open && !el("import-tools").hidden) {
            message(errorText(error), true, promptFeedback);
        }
    }
    finally {
        copyPrompt.disabled = false;
    }
}
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
        if (version === editorVersion)
            message(errorText(error), true, editorNotice);
    }
});
copyPrompt.addEventListener("click", async () => {
    if (!prompt.value) {
        await loadPrompt();
        return;
    }
    const version = editorVersion;
    if (navigator.clipboard) {
        try {
            await navigator.clipboard.writeText(prompt.value);
            if (version === editorVersion)
                message("AI 提示词已复制", false, promptFeedback);
            return;
        }
        catch {
            if (version !== editorVersion)
                return;
        }
    }
    // LAN HTTP pages cannot use the secure-context clipboard API.
    const wasOpen = promptBox.open;
    promptBox.open = true;
    prompt.focus();
    prompt.select();
    if (!document.execCommand("copy")) {
        message("自动复制失败，已选中提示词，可手动复制。", true, promptFeedback);
        return;
    }
    promptBox.open = wasOpen;
    copyPrompt.focus();
    message("AI 提示词已复制", false, promptFeedback);
});
function closeEditor() {
    if (saving)
        return;
    ++editorVersion;
    editor.close();
}
el("close-editor").addEventListener("click", closeEditor);
editor.addEventListener("cancel", event => {
    event.preventDefault();
    closeEditor();
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
            trackTask({ source_id: result.rule.id, task_id: result.task_id });
        if (result.task_error) {
            const { code, detail } = result.task_error;
            showTaskStatus(result.rule.id, null, `首次更新未启动：${detail || code}`, true);
        }
        editor.close();
        try {
            await refresh();
            message("脚本已保存");
        }
        catch (error) {
            message(`脚本已保存，但刷新失败：${errorText(error)}。请点击刷新列表。`, true);
        }
    }
    catch (error) {
        message(errorText(error), true, editorNotice);
    }
    finally {
        saving = false;
        save.disabled = false;
    }
});
el("update-all").addEventListener("click", async () => {
    try {
        const result = await api("/updates", "POST", { force: false });
        result.tasks.forEach(trackTask);
        if (!result.tasks.length)
            message("没有需要更新的来源");
    }
    catch (error) {
        message(errorText(error), true);
    }
});
export {};
