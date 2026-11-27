import type { AddSourceResult, SourceSummary, ScriptSourceDocument, ScriptSourcePromptResult, SourceProgressEvent, SourceUpdateResult } from "../../packages/contracts/generated/protocol";
type TaskRef = { source_id: string; task_id: string };
type Task = {
    task_id: string;
    running: boolean;
    cancel_requested: boolean;
    progress: SourceProgressEvent | null;
    result: SourceUpdateResult | null;
    error: {
        code: string;
        detail?: string | null;
    } | null;
    control_error: {
        code: string;
        detail?: string | null;
    } | null;
};
const el = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;
const notice = el<HTMLDivElement>("notice");
const workspace = el<HTMLDivElement>("workspace");
const editor = el<HTMLDialogElement>("editor");
const editorNotice = el<HTMLDivElement>("editor-notice");
const code = el<HTMLTextAreaElement>("code");
const prompt = el<HTMLTextAreaElement>("prompt");
const promptBox = el<HTMLDetailsElement>("prompt-box");
const copyPrompt = el<HTMLButtonElement>("prompt-copy");
const promptFeedback = el<HTMLDivElement>("prompt-feedback");
let key = "";
let editing: string | null = null;
let readonly = false;
let saving = false;
let editorVersion = 0;
let listRequest = 0;
let writes: Promise<unknown> = Promise.resolve();
type Feedback = {
    label: string;
    action: "更新" | "取消" | "重查";
    id?: string;
    busy?: boolean;
    locked?: boolean;
    detail?: string;
    timer?: number;
};
const tasks = new Map<string, Feedback>();
const cards = new Map<string, ReturnType<typeof sourceCard>>();
let copyTimer: number | undefined;
function message(text: string, error = false, target = notice) {
    target.className = `alert mt-3 ${error ? "alert-danger" : "alert-success"}`;
    target.textContent = text;
}
function errorText(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}
async function api<T>(path: string, method = "GET", body?: object): Promise<T> {
    const request = async (): Promise<T> => {
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
        return data as T;
    };
    if (method === "GET") return request();
    const result = writes.then(request);
    writes = result.catch(() => {});
    return result;
}
function button(text: string, action: () => void, style = "btn") {
    const b = document.createElement("button");
    b.type = "button";
    b.className = style;
    b.textContent = text;
    b.addEventListener("click", action);
    return b;
}
function label(text: string, input: HTMLElement) {
    const l = document.createElement("label");
    l.className = "form-label mt-3";
    l.textContent = text;
    l.append(input);
    return l;
}
async function refresh() {
    const request = ++listRequest;
    const data = await api<{
        sources: SourceSummary[];
        settings_revision: number;
    }>("/sources");
    if (request !== listRequest) return;
    const present = new Set(data.sources.map(source => source.id));
    for (const [id, card] of cards) {
        if (present.has(id)) continue;
        card.element.remove();
        cards.delete(id);
        clearFeedback(id);
    }
    for (const role of ["publisher", "fallback_indexer"]) {
        const group = el(role);
        const entries = data.sources.filter(source => source.role === role);
        el(role + "-empty").hidden = entries.length > 0;
        entries.forEach((source, index) => {
            let card = cards.get(source.id);
            if (!card || card.role !== role) {
                card?.element.remove();
                card = sourceCard(source);
                cards.set(source.id, card);
            }
            card.update(source);
            const position = group.children[index];
            if (position !== card.element) group.insertBefore(card.element, position ?? null);
        });
    }
}
async function mutate(path: string, method: string, body?: object) {
    await api(path, method, body);
    try {
        await refresh();
        message("已保存");
    }
    catch (error) {
        message(`已保存，但刷新失败：${errorText(error)}。请点击刷新列表。`, true);
    }
}
function sourceCard(source: SourceSummary) {
    const id = encodeURIComponent(source.id), path = `/sources/${id}`;
    const card = document.createElement("article");
    card.className = "card mb-2";
    card.dataset.source = source.id;
    const body = document.createElement("div");
    body.className = "card-body source-details";
    body.id = "details-" + id;
    body.hidden = true;
    const heading = button("", () => {
        body.hidden = !body.hidden;
        heading.setAttribute("aria-expanded", String(!body.hidden));
    }, "source-heading");
    heading.setAttribute("aria-expanded", "false");
    heading.setAttribute("aria-controls", body.id);
    const name = document.createElement("strong");
    const status = document.createElement("span");
    status.className = "source-enabled";
    heading.append(name, status);
    card.append(heading);
    const row = document.createElement("div");
    row.className = "source-status-row";
    const caption = document.createElement("span");
    caption.className = "source-caption";
    const announce = document.createElement("span");
    announce.className = "visually-hidden";
    announce.setAttribute("role", "status");
    const action = button("更新", () => void actOnSource(source.id), "btn btn-sm source-action");
    row.append(caption, announce, action);
    if (source.role === "publisher") card.append(row);
    const description = document.createElement("p");
    description.className = "text-secondary source-domains";
    const errorDetail = document.createElement("p");
    errorDetail.className = "text-danger source-domains";
    body.append(description, errorDetail);
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
        } catch (error) {
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
        } catch (error) {
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
            if (!interval.reportValidity()) return;
            try {
                await mutate(`${path}/interval`, "PUT", { interval_hours: Number(interval.value) });
            } catch (error) {
                message(errorText(error), true);
            }
        }, "btn btn-outline-primary mt-2"));
    }
    const actions = document.createElement("div");
    actions.className = "d-flex flex-wrap gap-2 mt-3";
    const script = button("", async () => {
        try { await openScript(source); } catch (error) { message(errorText(error), true); }
    });
    const remove = button("删除", async () => {
        if (!confirm("确定删除来源“" + source.name + "”？")) return;
        try {
            await mutate(path, "DELETE");
            clearFeedback(source.id);
        } catch (error) { message(errorText(error), true); }
    }, "btn btn-outline-danger");
    actions.append(script, remove);
    body.append(actions);
    card.append(body);

    function render() {
        const feedback = tasks.get(source.id);
        const warning = sourceWarning(source.status);
        const text = feedback?.label ?? (warning || lastCheck(source.checked_at));
        caption.textContent = text;
        caption.dataset.busy = String(!!feedback?.busy);
        caption.dataset.warning = String(!!(feedback?.detail || (!feedback && warning)));
        action.textContent = feedback?.action ?? "更新";
        action.setAttribute("aria-disabled", String(!!feedback?.locked));
        action.setAttribute("aria-label", action.textContent + "：" + source.name);
        const spoken = feedback?.label.replace(/ [0-9]+\/[0-9]+$/, "") ?? "";
        if (announce.textContent !== spoken) announce.textContent = spoken;
        errorDetail.textContent = feedback?.detail ?? "";
        errorDetail.hidden = !feedback?.detail;
    }
    return {
        element: card, role: source.role, render,
        update(next: SourceSummary) {
            if (nameInput.value === source.name) nameInput.value = next.name;
            if (interval.value === String(source.interval_hours)) interval.value = String(next.interval_hours);
            source = next;
            name.textContent = source.name;
            status.textContent = source.enabled ? "已启用" : "已停用";
            if (source.role !== "publisher") status.textContent += " · " + source.status;
            enabled.checked = source.enabled;
            script.hidden = !source.script;
            script.textContent = source.origin === "builtin" ? "查看脚本" : "编辑脚本";
            remove.hidden = source.origin === "builtin";
            description.textContent = "页面域名：" + source.page_hosts.join("、") + "；下载域名：" + (source.download_hosts.join("、") || "无") +
                " · 条目：" + source.entry_count + " · " + (remove.hidden ? "内置" : "自定义") + " · " + source.status;
            render();
        }
    };
}

async function openScript(source: SourceSummary) {
    if (saving)
        return;
    const version = ++editorVersion;
    const doc = await api<ScriptSourceDocument>(`/sources/${encodeURIComponent(source.id)}/script`);
    if (version !== editorVersion || saving) return;
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
const dateFormat = new Intl.DateTimeFormat("zh-CN", { year: "numeric", month: "numeric", day: "numeric" });
const timeFormat = new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false });
function lastCheck(seconds: number | null) {
    if (!seconds) return "尚未检查";
    const date = new Date(seconds * 1000);
    const day = date.toDateString() === new Date().toDateString() ? "" : dateFormat.format(date) + " ";
    return "最近检查 " + day + timeFormat.format(date);
}
function sourceWarning(status: string) {
    switch (status) {
        case "source_offline_cached": return "来源离线，已保留缓存";
        case "source_browser_verification_required":
        case "source_age_verification_required": return "需要站点验证";
        case "source_partial": return "索引更新不完整";
        case "failed":
        case "source_refresh_failed": return "更新失败";
        default: return /^discovery_http_\d{3}$/.test(status) ? "来源请求失败" : "";
    }
}
function progressLabel(progress: Task["progress"]) {
    switch (progress?.phase) {
        case "index": return "正在读取索引";
        case "details": return "准备匹配游戏";
        case "matched":
            return progress.total > 0 && progress.completed != null
                ? `匹配游戏 ${progress.completed}/${progress.total}` : "匹配游戏";
        case "browser_verification": return "等待站点验证";
        case "browser_collect": return "正在读取来源列表";
        default: return progress?.phase ? "正在更新" : "正在启动";
    }
}
function clearFeedback(sourceId: string) {
    window.clearTimeout(tasks.get(sourceId)?.timer);
    tasks.delete(sourceId);
    cards.get(sourceId)?.render();
}
function setFeedback(sourceId: string, state: Feedback) {
    window.clearTimeout(tasks.get(sourceId)?.timer);
    tasks.set(sourceId, state);
    cards.get(sourceId)?.render();
    return state;
}
function trackTask({ source_id, task_id }: TaskRef) {
    if (tasks.get(source_id)?.id === task_id) return;
    const state = setFeedback(source_id, { id: task_id, label: "正在启动", action: "取消", busy: true });
    void poll(source_id, state);
}
async function actOnSource(sourceId: string) {
    const current = tasks.get(sourceId);
    if (current?.locked) return;
    if (current?.action === "重查") {
        const state = setFeedback(sourceId, { ...current, label: "查询任务状态", locked: true });
        await poll(sourceId, state);
        return;
    }
    if (current?.action === "取消") {
        const state = setFeedback(sourceId, { ...current, label: "正在取消", detail: undefined, locked: true });
        try {
            const task = await api<Task>(`/tasks/${encodeURIComponent(current.id!)}/cancel`, "POST");
            if (tasks.get(sourceId) === state) await applySnapshot(sourceId, task);
        } catch (error) {
            if (tasks.get(sourceId) !== state) return;
            const running = setFeedback(sourceId, { ...current, label: "取消失败", detail: errorText(error) });
            schedulePoll(sourceId, running);
        }
        return;
    }
    const starting = setFeedback(sourceId, { action: "更新", label: "正在启动", busy: true, locked: true });
    try {
        const result = await api<{task_id: string}>(`/sources/${encodeURIComponent(sourceId)}/updates`, "POST", { force: false });
        if (tasks.get(sourceId) === starting) trackTask({ source_id: sourceId, task_id: result.task_id });
    } catch (error) {
        if (tasks.get(sourceId) === starting)
            setFeedback(sourceId, { action: "更新", label: "更新未启动", detail: errorText(error) });
    }
}
function schedulePoll(sourceId: string, state: Feedback) {
    state.timer = window.setTimeout(() => void poll(sourceId, state), 1000);
}
async function poll(sourceId: string, state: Feedback) {
    if (tasks.get(sourceId) !== state) return;
    let task: Task;
    try {
        task = await api<Task>(`/tasks/${encodeURIComponent(state.id!)}`);
    } catch (error) {
        if (tasks.get(sourceId) === state)
            setFeedback(sourceId, { id: state.id, label: "任务状态读取失败", action: "重查", detail: errorText(error) });
        return;
    }
    if (tasks.get(sourceId) === state) await applySnapshot(sourceId, task);
}
async function applySnapshot(sourceId: string, task: Task) {
    if (task.running) {
        const failed = task.control_error;
        const cancelling = task.cancel_requested && !failed;
        const state = setFeedback(sourceId, {
            id: task.task_id, action: "取消", busy: true, locked: cancelling,
            label: failed ? "取消失败" : cancelling ? "正在取消" : progressLabel(task.progress),
            detail: failed?.detail || failed?.code,
        });
        schedulePoll(sourceId, state);
        return;
    }
    const status = task.result?.status?.code ?? "";
    const cancelled = task.error ? ["cancelled", "task_cancelled", "user_cancelled"].includes(task.error.code)
        : task.cancel_requested && !task.result;
    const warning = cancelled ? "" : (task.error ? "更新失败" : sourceWarning(status));
    const label = warning || (cancelled ? "已取消" : status === "source_not_modified" ? "无变化" : "已更新");
    const finished = setFeedback(sourceId, {
        id: task.task_id, action: "更新", label, detail: warning ? task.error?.detail || task.error?.code || status : undefined,
    });
    if (!warning) finished.timer = window.setTimeout(() => {
        if (tasks.get(sourceId) === finished) clearFeedback(sourceId);
    }, 3000);
    try {
        await refresh();
    } catch (error) {
        if (tasks.get(sourceId) === finished)
            message("任务已结束，但刷新来源失败：" + errorText(error) + "。请点击刷新列表。", true);
    }
}
el<HTMLFormElement>("login-form").addEventListener("submit", async event => {
    event.preventDefault();
    key = el<HTMLInputElement>("key").value;
    try {
        await refresh();
        el("login").hidden = true;
        workspace.hidden = false;
        notice.textContent = "";
        notice.className = "";
    } catch (error) {
        key = "";
        message(errorText(error), true);
    }
});

el("refresh").addEventListener("click", async () => {
    try {
        await refresh();
        message("列表已刷新");
    } catch (error) {
        message(errorText(error), true);
    }
});

el("import").addEventListener("click", () => {
    if (saving) return;
    ++editorVersion;
    editing = null;
    readonly = false;
    code.readOnly = false;
    code.value = "";
    el<HTMLInputElement>("file").value = "";
    el("editor-title").textContent = "导入脚本来源";
    el("import-tools").hidden = false;
    promptBox.open = false;
    promptFeedback.replaceChildren();
    promptFeedback.className = "";
    el("save").hidden = false;
    showEditor();
    if (!prompt.value && !copyPrompt.disabled) void loadPrompt();
});

async function loadPrompt() {
    promptFeedback.replaceChildren();
    promptFeedback.className = "";
    promptBox.hidden = true;
    copyPrompt.disabled = true;
    copyPrompt.title = "";
    copyPrompt.textContent = "正在读取 AI 提示词…";
    try {
        const result = await api<ScriptSourcePromptResult>("/script-prompt");
        prompt.value = result.prompt;
        promptBox.hidden = false;
        copyPrompt.textContent = "复制 AI 提示词";
    } catch (error) {
        copyPrompt.textContent = "重试读取 AI 提示词";
        copyPrompt.title = errorText(error);
    } finally {
        copyPrompt.disabled = false;
    }
}

el<HTMLInputElement>("file").addEventListener("change", async event => {
    const file = (event.target as HTMLInputElement).files?.[0];
    if (!file) return;
    const version = ++editorVersion;
    try {
        const text = await file.text();
        if (version === editorVersion) code.value = text;
    } catch (error) {
        if (version === editorVersion) message(errorText(error), true, editorNotice);
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
            if (version === editorVersion) copiedPrompt();
            return;
        } catch {
            if (version !== editorVersion) return;
        }
    }
    // LAN HTTP pages cannot use the secure-context clipboard API.
    const wasOpen = promptBox.open;
    promptBox.open = true;
    prompt.focus();
    prompt.select();
    let copied = false;
    try {
        copied = document.execCommand("copy");
    } catch {
        // Keep the text selected when the browser blocks the clipboard command.
    }
    if (!copied) {
        window.clearTimeout(copyTimer);
        copyPrompt.textContent = "复制 AI 提示词";
        message("自动复制失败，已选中提示词，可手动复制。", true, promptFeedback);
        return;
    }
    promptBox.open = wasOpen;
    copyPrompt.focus();
    copiedPrompt();
});

function copiedPrompt() {
    promptFeedback.textContent = "";
    promptFeedback.className = "";
    copyPrompt.textContent = "✓ 已复制";
    window.clearTimeout(copyTimer);
    copyTimer = window.setTimeout(() => { copyPrompt.textContent = "复制 AI 提示词"; }, 2000);
}
function dismissEditor() {
    window.clearTimeout(copyTimer);
    if (prompt.value) copyPrompt.textContent = "复制 AI 提示词";
    editor.close();
}
function closeEditor() {
    if (saving) return;
    ++editorVersion;
    dismissEditor();
}
el("close-editor").addEventListener("click", closeEditor);
editor.addEventListener("cancel", event => {
    event.preventDefault();
    closeEditor();
});

el("save").addEventListener("click", async () => {
    if (saving) return;
    ++editorVersion;
    saving = true;
    const save = el<HTMLButtonElement>("save");
    save.disabled = true;
    const path = editing ? `/sources/${encodeURIComponent(editing)}/script` : "/sources";
    try {
        const result = await api<AddSourceResult & {
            task_id?: string;
            task_error?: { code: string; detail?: string | null };
        }>(path, editing ? "PUT" : "POST", { code: code.value });
        if (result.task_id) trackTask({ source_id: result.rule.id, task_id: result.task_id });
        if (result.task_error) {
            const { code, detail } = result.task_error;
            setFeedback(result.rule.id, { label: "首次更新未启动", action: "更新", detail: detail || code });
        }
        dismissEditor();
        try {
            await refresh();
            message("脚本已保存");
        } catch (error) {
            message(`脚本已保存，但刷新失败：${errorText(error)}。请点击刷新列表。`, true);
        }
    } catch (error) {
        message(errorText(error), true, editorNotice);
    } finally {
        saving = false;
        save.disabled = false;
    }
});

el("update-all").addEventListener("click", async () => {
    try {
        const result = await api<{ tasks: TaskRef[] }>("/updates", "POST", { force: false });
        result.tasks.forEach(trackTask);
        if (!result.tasks.length) message("没有需要更新的来源");
    } catch (error) {
        message(errorText(error), true);
    }
});
