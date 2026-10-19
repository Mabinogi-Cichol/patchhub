import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { downloadFileName, candidateMatched, displayPatches, filterInstalls, gameDisplayName, isExternal, patchState, reason, installationResult, progressStageLabel, progressPercent, workflowState, type Filter, type Install, type Preview, type ScanResult, type SourceCandidate, type Transaction, type InstallerOutcome } from "@steam-patch/ui";
import {
  actionBlockers,
  primaryInstallAction,
  candidateConditions,
  candidateFormat,
  candidateLanguage,
  candidateSourceLabel,
  chipLabel,
  fallbackDisplayStatus,
  gameMatchStatusHint,
  matchBasisLabel,
  patchChip,
  pendingMatchInstances,
  rowSummary,
  safeSourceUrl,
  sourceChip,
  sourceIntervalLabel,
  statusLabel,
  statusTone,
  taskForInstance,
  taskKindLabel,
  taskProgressText,
  unqueriedCount,
  versionEvidence,
  type Capabilities,
  type GameMatch,
  type GameSourceSubscription,
  type MatchGamesResult,
  type MatchProgressEvent,
  type MatchUpdateEvent,
  type ProjectInstancesResult,
  type SourceProgressEvent,
  type SourceSummary,
  type SourceTaskStatus,
  type SourceUpdateEvent,
  type SourceUpdateResult,
} from "./sources";
import { SourcePanel } from "./SourcePanel";
import "./style.css";

type CoreOperation = "scan" | "preview" | "list_transactions" | "recover";
type InstallStatus = {
  running: boolean;
  cancelled: boolean;
  progress: Omit<import("../../../packages/contracts/generated/protocol").InstallProgressEvent, "event" | "protocol_version" | "request_id" | "sequence" | "instance_id" | "selection_id"> | null;
};

function call<T>(operation: CoreOperation, args: { instanceId?: string; releaseId?: string; selectionId?: string } = {}): Promise<T> {
  return invoke<T>("core_call", { operation, args });
}

/** Turn a raw host error (`code` or `code: detail`) into a real, readable message. */
function describeError(value: unknown): string {
  if (value && typeof value === "object" && "code" in value && typeof value.code === "string") {
    const detail = "detail" in value && typeof value.detail === "string" ? value.detail : null;
    return detail ? `${reason(value.code)}（${detail}）` : reason(value.code);
  }
  return typeof value === "string" ? value : String(value);
}

function Chip({ tone, label }: { tone: string; label: string }) {
  return <span className={`chip ${tone}`}>{label}</span>;
}

function CandidateCard({ candidate, onOpen }: { candidate: SourceCandidate; onOpen: (url: string) => void }) {
  const matched = candidateMatched(candidate);
  const format = candidateFormat(candidate);
  const evidence = versionEvidence(candidate);
  const warnings = (candidate.warnings ?? []).filter(warning => !(format.conflict && warning === "artifact_format_conflict"));
  return <section className="card candidate">
    <div className="card-head">
      <h3>{candidate.patch_title}</h3>
      <Chip tone={matched ? "ok" : "warn"} label={matched ? "身份已核实" : "身份待确认"} />
    </div>
    <div className="facts">
      <div><label>来源</label><strong>{candidateSourceLabel(candidate)}</strong></div>
      <div><label>补丁语言</label><strong>{candidateLanguage(candidate)}</strong></div>
      <div><label>适用版本</label><strong>{candidateConditions(candidate)}</strong></div>
      <div><label>补丁版本</label><strong>{candidate.version_raw || "待确认"}</strong></div>
    </div>
    <p><b>文件格式：</b>{format.label}{format.conflict ? `（页面标示 ${candidate.format_reported}，上游为 ${candidate.format_observed}）` : ""}</p>
    {!!evidence.length && <p><b>版本依据：</b>{evidence.join("、")}</p>}
    <p><b>身份依据：</b>{matchBasisLabel(candidate)}</p>
    {candidate.cache_status && <p><b>来源记录：</b>{reason(candidate.cache_status)}</p>}
    <p className="capability-hint">此条目是来源线索，用于核对游戏语言、版本与 DLC 条件；本平台不会据此启用安装，安装入口只对已审核的补丁版本开放。</p>
    {!matched && <p className="warning">此线索的游戏身份尚待确认，请核对来源中的游戏信息。</p>}
    {warnings.map((warning, index) => <p className="warning" key={index}>{reason(warning)}</p>)}
    <div className="actions">
      <button onClick={() => onOpen(candidate.source_page)}>{candidate.source_id === "hgamer" ? "打开补丁详情 ↗" : "打开补丁来源页 ↗"}</button>
      {candidate.upstream_page && candidate.upstream_page !== candidate.source_page && <button onClick={() => onOpen(candidate.upstream_page!)}>打开上游发布页 ↗</button>}
    </div>
  </section>;
}

function App() {
  const [scan, setScan] = useState<ScanResult | null>(null);
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [selectedDownload, setSelectedDownload] = useState<string | null>(null);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [preferredLanguage, setPreferredLanguage] = useState<string | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [manualConfirmation, setManualConfirmation] = useState<string | null>(null);
  const [archive, setArchive] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [job, setJob] = useState<InstallStatus | null>(null);
  const [installing, setInstalling] = useState(false);
  const [sourceTasks, setSourceTasks] = useState<SourceTaskStatus[]>([]);
  const [sourceNotice, setSourceNotice] = useState<string | null>(null);
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const [globalSources, setGlobalSources] = useState<SourceSummary[] | null>(null);
  const [gameSources, setGameSources] = useState<GameSourceSubscription[] | null>(null);
  const [sourcesLoading, setSourcesLoading] = useState(false);

  const requestIds = useRef(new Map<string, string>());
  const scanRef = useRef<ScanResult | null>(null);
  const selectedRef = useRef<string | null>(null);
  const capabilitiesPromise = useRef<Promise<Capabilities | null> | null>(null);
  const capabilityErrorRef = useRef<string | null>(null);
  const finishedTasksRef = useRef(new Set<string>());
  const sourcesOpenRef = useRef(false);

  const game = scan?.installs.find(x => x.game.instance_id === selected) ?? null;
  const entries = useMemo(() => (game ? displayPatches(game, preferredLanguage) : []), [game, preferredLanguage]);
  const entry = entries.find(item => item.key === selectedKey) ?? entries[0] ?? null;
  const patch = entry?.kind === "installable" ? entry.choices.find(p => (p.selection_id ?? p.release_id) === selectedDownload) ?? entry.patch : null;
  const shown = useMemo(() => filterInstalls(scan?.installs ?? [], filter, query), [scan, filter, query]);
  const owned = transactions.filter(x => x.instance_id === selected);
  const scanBusy = busy?.includes("扫描") ?? false;
  const tasksRunning = sourceTasks.some(task => task.running);
  const unqueried = useMemo(() => unqueriedCount(scan?.installs ?? []), [scan]);
  const settingsRevision = scan?.settings_revision ?? null;

  function instanceMatching(instanceId: string): boolean {
    return sourceTasks.some(task => task.running
      && ((task.kind === "match_games" && task.key === "match")
        || (task.kind === "rematch_game" && task.key === `instance:${instanceId}`)));
  }

  useEffect(() => { scanRef.current = scan; }, [scan]);
  useEffect(() => { selectedRef.current = selected; }, [selected]);
  useEffect(() => {
    if (entries.some(item => item.key === selectedKey)) return;
    setSelectedKey(entries[0]?.key ?? null);
  }, [entries, selectedKey]);

  // Capability negotiation is the single version/availability gate. A failure
  // is a real contract error: show the host diagnostic instead of falling back.
  function loadCapabilities(): Promise<Capabilities | null> {
    if (!capabilitiesPromise.current) {
      capabilitiesPromise.current = invoke<Capabilities>("get_capabilities")
        .then(caps => { capabilityErrorRef.current = null; return caps; })
        .catch(e => { const message = describeError(e); capabilityErrorRef.current = message; return null; });
    }
    return capabilitiesPromise.current;
  }

  async function run<T>(label: string, task: () => Promise<T>, done: (result: T) => void | Promise<void>) {
    setBusy(label); setError(null); setNotice(null);
    try { await done(await task()); }
    catch (e) { setError(describeError(e)); }
    finally { setBusy(null); }
  }

  // ---- source tasks: polling, live match rows and result merging ---------
  function upsertTask(snapshot: SourceTaskStatus) {
    setSourceTasks(prev => [snapshot, ...prev.filter(task => task.task_id !== snapshot.task_id)]);
  }

  function applyGameMatches(matches: GameMatch[]) {
    if (!matches.length) return;
    setScan(prev => {
      if (!prev) return prev;
      const byInstance = new Map<string, GameMatch>();
      const byAppid = new Map<number, GameMatch>();
      for (const match of matches) {
        if (match.instance_id) byInstance.set(match.instance_id, match);
        byAppid.set(match.appid, match);
      }
      const installs = prev.installs.map(row => {
        const match = byInstance.get(row.game.instance_id) ?? byAppid.get(row.game.appid);
        if (!match) return row;
        return { ...row, discovery: { status: match.status, candidates: match.candidates ?? [], diagnostics: match.diagnostics ?? [] } };
      });
      return { ...prev, installs };
    });
  }

  function applyProjectedInstances(result: ProjectInstancesResult) {
    setScan(prev => {
      if (!prev) return prev;
      const byInstance = new Map(result.instances.map(row => [row.game.instance_id, row]));
      const installs = prev.installs.map(row => byInstance.get(row.game.instance_id) ?? row);
      return { ...prev, installs, settings_revision: result.settings_revision ?? prev.settings_revision };
    });
  }

  /** Refresh install actions for affected games without a library-wide scan. */
  async function refreshProjectedMatches(appids: number[]) {
    const current = scanRef.current;
    if (!current || !appids.length) return;
    const appidSet = new Set(appids);
    const instanceIds = current.installs.filter(row => appidSet.has(row.game.appid)).map(row => row.game.instance_id);
    if (!instanceIds.length) return;
    const projected = await invoke<ProjectInstancesResult>("project_instances", { instanceIds });
    applyProjectedInstances(projected);
  }

  async function completeTask(task: SourceTaskStatus) {
    if (finishedTasksRef.current.has(task.task_id)) return;
    finishedTasksRef.current.add(task.task_id);
    if (task.error) {
      const code = task.error.code;
      setSourceNotice(code === "cancelled" || code === "task_cancelled" || code === "download_cancelled"
        ? "已取消来源任务，已保留取得的结果。"
        : `来源任务未完成：${describeError(task.error)}`);
      return;
    }
    const result = task.result;
    if (!result) return;
    if (task.kind === "update_source") {
      const update = result as SourceUpdateResult;
      setSourceNotice(`来源 ${update.source_id} 已更新${update.complete ? "" : "（索引未完整）"}，影响 ${update.affected_appids?.length ?? 0} 款游戏。`);
      await refreshProjectedMatches(update.affected_appids ?? []);
      if (sourcesOpenRef.current) await loadSources(false);
      return;
    }
    const matched = result as MatchGamesResult;
    applyGameMatches(matched.results ?? []);
    setScan(prev => prev ? { ...prev, settings_revision: matched.settings_revision ?? prev.settings_revision } : prev);
    await refreshProjectedMatches((matched.results ?? []).map(match => match.appid));
  }

  async function pollTasks() {
    try {
      const list = await invoke<SourceTaskStatus[]>("source_task_status");
      setSourceTasks(list);
      for (const task of list) {
        if (!task.running) await completeTask(task);
      }
    } catch (e) {
      setSourceNotice(describeError(e));
    }
  }

  async function startSourceTask(command: string, args: Record<string, unknown>) {
    try {
      const snapshot = await invoke<SourceTaskStatus>(command, args);
      upsertTask(snapshot);
      setSourceNotice(null);
      return snapshot;
    } catch (e) {
      setError(describeError(e));
      return null;
    }
  }

  function updateSource(sourceId: string) {
    void startSourceTask("start_source_update", { sourceId, force: false });
  }

  function matchAllGames() {
    void startSourceTask("start_match_games", { preferredLanguage: preferredLanguage ?? undefined });
  }

  function rematchGame(instanceId: string) {
    void startSourceTask("start_rematch_game", { instanceId, preferredLanguage: preferredLanguage ?? undefined });
  }

  async function cancelTask(taskId: string) {
    try {
      await invoke<boolean>("cancel_source_task", { taskId });
      await pollTasks();
    } catch (e) {
      setError(describeError(e));
    }
  }

  /**
   * Newly discovered games enter an incremental library match automatically;
   * the scan result has already rendered, so this never blocks or re-scans.
   */
  async function maybeAutoMatch(rows: Install[]) {
    try {
      const sources = await invoke<SourceSummary[]>("list_sources");
      setGlobalSources(sources);
      const tasks = await invoke<SourceTaskStatus[]>("source_task_status");
      setSourceTasks(tasks);
      // Reconcile tasks the automatic scheduler already finished while the
      // window was still starting, without waiting for another scan.
      for (const task of tasks) {
        if (!task.running) await completeTask(task);
      }
      const instances = pendingMatchInstances(rows, sources, tasks);
      if (!instances.length) return;
      const snapshot = await invoke<SourceTaskStatus>("start_match_games", { instances });
      upsertTask(snapshot);
    } catch (e) {
      setSourceNotice(describeError(e));
    }
  }

  useEffect(() => {
    sourcesOpenRef.current = sourcesOpen;
  }, [sourcesOpen]);

  useEffect(() => {
    if (!tasksRunning) return;
    let active = true;
    const poll = async () => { if (active) await pollTasks(); };
    void poll();
    const timer = window.setInterval(() => { void poll(); }, 500);
    return () => { active = false; window.clearInterval(timer); };
  }, [tasksRunning]);

  // Host events keep the UI in sync with tasks started elsewhere (for example
  // the automatic update scheduler) without re-scanning the library.
  useEffect(() => {
    let disposed = false;
    const unlisten: Array<() => void> = [];
    const schedulePoll = () => { void pollTasks(); };
    void (async () => {
      const handlers = await Promise.all([
        listen<MatchUpdateEvent>("match_update", event => {
          const payload = event.payload;
          applyGameMatches([{
            appid: payload.appid,
            instance_id: payload.instance_id,
            status: payload.status as GameMatch["status"],
            candidates: payload.candidates,
            diagnostics: payload.diagnostics,
            source_revisions: {},
          }]);
          schedulePoll();
        }),
        listen<SourceUpdateEvent>("source_update", () => {
          if (sourcesOpenRef.current) void loadSources(false);
          schedulePoll();
        }),
        listen<SourceProgressEvent>("source_progress", schedulePoll),
        listen<MatchProgressEvent>("match_progress", schedulePoll),
      ]);
      if (disposed) handlers.forEach(off => off()); else unlisten.push(...handlers);
    })();
    return () => { disposed = true; unlisten.forEach(off => off()); };
  }, []);

  async function refresh() {
    const caps = await loadCapabilities();
    if (!caps) { setError(capabilityErrorRef.current ?? "核心契约不可用"); return; }
    let nextScan: ScanResult | null = null;
    await run("正在扫描 Steam 库…", async () => {
      // Both core calls open the same journal; finish one before starting the other.
      const value = await call<ScanResult>("scan");
      const nextTransactions = await call<Transaction[]>("list_transactions");
      return { nextScan: value, nextTransactions };
    }, ({ nextScan: value, nextTransactions }) => {
      nextScan = value;
      scanRef.current = value;
      setScan(value);
      setTransactions(nextTransactions);
      if (!value.installs.some(x => x.game.instance_id === selectedRef.current)) { setSelected(null); setSelectedKey(null); }
    });
    if (capabilityErrorRef.current) setError(capabilityErrorRef.current);
    // The scan result is already rendered. Newly found games enter an
    // incremental match task; this never re-scans and never blocks the UI.
    if (nextScan) void maybeAutoMatch((nextScan as ScanResult).installs);
  }

  useEffect(() => { void refresh(); }, []);

  useEffect(() => {
    if (!sourcesOpen) return;
    void loadSources();
  }, [sourcesOpen, selected]);

  useEffect(() => { setPreview(null); setArchive(null); setManualConfirmation(null); }, [selected, selectedKey, patch?.selection_id, patch?.release_id]);

  useEffect(() => {
    if (!installing) return;
    let active = true;
    const poll = async () => {
      try {
        const status = await invoke<InstallStatus>("install_status");
        if (active) setJob(status);
      } catch { /* the install result remains authoritative */ }
    };
    void poll();
    const timer = window.setInterval(() => { void poll(); }, 500);
    return () => { active = false; window.clearInterval(timer); };
  }, [installing]);

  // ---- source management -------------------------------------------------
  async function loadSources(showLoading = true) {
    if (showLoading) setSourcesLoading(true);
    try {
      // The source drawer reads the authoritative list explicitly; `scan.sources`
      // is not used for schedule/revision fields.
      const list = await invoke<SourceSummary[]>("list_sources");
      setGlobalSources(list);
      const instanceId = selectedRef.current;
      if (instanceId) setGameSources(await invoke<GameSourceSubscription[]>("list_game_sources", { instanceId }));
      else setGameSources(null);
      setSourceTasks(await invoke<SourceTaskStatus[]>("source_task_status"));
    } catch (e) {
      setError(describeError(e));
    } finally {
      if (showLoading) setSourcesLoading(false);
    }
  }

  function reloadSources() {
    void loadSources();
  }

  /** Settings writes return the new revision; keep the drawer header accurate. */
  function applySettingsRevision(result: { settings_revision?: number | null }) {
    if (result.settings_revision != null) {
      setScan(prev => prev ? { ...prev, settings_revision: result.settings_revision ?? prev.settings_revision } : prev);
    }
  }

  function toggleSource(sourceId: string, enabled: boolean) {
    void run(enabled ? "正在启用来源…" : "正在停用来源…", async () => {
      applySettingsRevision(await invoke<{ settings_revision: number }>("set_source_enabled", { sourceId, enabled }));
      await loadSources(false);
    }, () => setNotice(`已${enabled ? "启用" : "停用"}来源。`));
  }

  function toggleGameSource(sourceId: string, subscribed: boolean) {
    if (!selected) return;
    void run("正在更新此游戏的订阅…", async () => {
      applySettingsRevision(await invoke<{ settings_revision: number }>("set_game_source", { instanceId: selected, sourceId, subscribed }));
      await loadSources(false);
    }, () => setNotice("此游戏的订阅已更新，可点击“重新匹配”生效。"));
  }

  function setSourceInterval(sourceId: string, intervalHours: number) {
    void run("正在保存自动更新间隔…", async () => {
      applySettingsRevision(await invoke<{ settings_revision: number }>("set_source_update_interval", { sourceId, intervalHours }));
      await loadSources(false);
    }, () => setNotice(`自动更新间隔已设置为：${sourceIntervalLabel(intervalHours)}。`));
  }

  function addSource(rule: string) {
    let parsed: unknown;
    try { parsed = JSON.parse(rule); } catch { setError("来源规则不是有效的 JSON，请检查后重试。"); return; }
    void run("正在添加来源…", async () => {
      applySettingsRevision(await invoke<{ settings_revision: number }>("add_source", { rule: parsed }));
      await loadSources(false);
    }, () => setNotice("已添加来源。"));
  }

  function openUrl(url: string) {
    if (!safeSourceUrl(url)) { setError("来源地址不是可用的公开 HTTPS 网页，已阻止打开。"); return; }
    void invoke("open_source", { url }).catch(e => setError(describeError(e)));
  }

  // ---- detail / install --------------------------------------------------
  function detail(row: Install) {
    setSelected(row.game.instance_id);
    setSelectedKey(displayPatches(row, preferredLanguage)[0]?.key ?? null);
  }
  function viewPreview() {
    if (!selected || !patch) return;
    const args = patch.selection_id ? { instanceId: selected, selectionId: patch.selection_id } : { instanceId: selected, releaseId: patch.release_id };
    void run("正在检查文件变更…", () => call<Preview>("preview", args), setPreview);
  }
  function selectedInstallPatch() {
    const row = scanRef.current?.installs.find(item => item.game.instance_id === selected);
    const selectedEntry = row && displayPatches(row, preferredLanguage).find(item => item.key === selectedKey);
    return selectedEntry?.kind === "installable" ? selectedEntry.choices.find(p => (p.selection_id ?? p.release_id) === selectedDownload) ?? selectedEntry.patch : null;
  }
  function chooseArchive() {
    const latest = selectedInstallPatch();
    if (!latest?.actions.local_archive.available) return;
    const instanceId = selected;
    const identity = latest.selection_id ?? latest.release_id;
    void run("正在选择 ZIP 文件…", async () => {
      const current = selectedInstallPatch();
      if (selected !== instanceId || !current || (current.selection_id ?? current.release_id) !== identity || !current.actions.local_archive.available) return null;
      return invoke<string | null>("choose_archive");
    }, value => {
      const current = selectedInstallPatch();
      if (selected === instanceId && current && (current.selection_id ?? current.release_id) === identity && current.actions.local_archive.available) setArchive(value);
    });
  }
  async function startInstall(route: "primary" | "local_archive" = "primary") {
    if (!selected || !patch || installing || busy) return;
    const latest = selectedInstallPatch();
    if (!latest || (latest.selection_id ?? latest.release_id) !== (patch.selection_id ?? patch.release_id)) return;
    const action = primaryInstallAction(latest, !!archive, route);
    if (!action.available || (latest.compatibility === "manual_selection_required" && manualConfirmation !== latest.selection_id)) return;
    const selectedArchive = action.input === "local_archive" ? archive ?? "" : "";
    const operation = action.operation;
    const wasInstalled = action.label === "overwrite";
    const identity = latest.selection_id ?? latest.release_id;
    const key = `${selected}|${identity}|${operation}|${selectedArchive}`;
    const requestId = requestIds.current.get(key) ?? crypto.randomUUID();
    requestIds.current.set(key, requestId);
    setInstalling(true); setJob(null); setBusy("正在安装…"); setError(null); setNotice(null);
    try {
      const outcome = await invoke<InstallerOutcome>("start_install", {
        instanceId: selected,
        ...(latest.selection_id ? { selectionId: latest.selection_id } : { releaseId: latest.release_id }),
        operation,
        requestId,
        archive: action.input === "local_archive" ? archive ?? undefined : undefined,
      });
      const message = installationResult(outcome, wasInstalled);
      requestIds.current.delete(key);
      setInstalling(false); setJob(null); setBusy(null);
      await refresh();
      setNotice(message);
    } catch (e) {
      const message = describeError(e);
      requestIds.current.delete(key);
      setInstalling(false); setJob(null); setBusy(null);
      await refresh();
      setError(message);
    }
  }
  function recover() {
    void run("正在检查并恢复未完成事务…", () => call<Transaction[]>("recover"), async result => {
      await refresh();
      const pending = result.filter(tx => tx.state === "needs_recovery");
      if (pending.length) {
        setError(`有 ${pending.length} 项操作仍需恢复，已保留冲突文件和恢复材料。${pending.map(tx => tx.error ? reason(tx.error) : "").filter(Boolean).join("；")}`);
      } else {
        setNotice(result.length ? `恢复检查完成，已处理 ${result.length} 项操作。` : "恢复检查完成，没有待恢复操作。");
      }
    });
  }

  const progress = job?.progress;
  const progressWidth = progress ? progressPercent(progress.bytes_done, progress.total_bytes) : null;
  const progressIndeterminate = progressWidth == null;
  const detailChip = game ? sourceChip(game, instanceMatching(game.game.instance_id)) : null;

  return <div className="shell">
    <header><div><h1>Steam 黄油补丁助手</h1><p>一站式查找、下载并安装 Steam 黄油的成人内容补丁。</p></div><div className="header-actions"><button onClick={() => setSourcesOpen(true)}>来源管理</button><button onClick={recover} disabled={!!busy}>尝试恢复未完成操作</button><button className="primary" onClick={() => void refresh()} disabled={!!busy}>重新扫描</button></div></header>
    {busy && !installing && <div role="status" className="status">{busy} 任务完成后会显示结果；当前没有百分比进度。</div>}
    {error && <div role="alert" className="error">{error}</div>}{notice && <div role="status" className="notice">{notice}</div>}
    {installing && <section className="progress" aria-live="polite"><h3>{progress ? progressStageLabel(progress.stage, progress.substage) : "正在准备安装"}</h3><div className={`progress-bar${progressIndeterminate ? " indeterminate" : ""}`}><div className="progress-fill" style={progressIndeterminate ? undefined : { width: `${progressWidth}%` }} /></div>{progress && <div className="progress-meta"><span>{progress.total_bytes ? `${progress.bytes_done} / ${progress.total_bytes} bytes` : "正在处理"}</span>{progress.total_files != null && <span>{progress.files_done} / {progress.total_files} 个文件</span>}</div>}{progress?.current_file && <p className="path">{progress.current_file}</p>}<button className="cancel" onClick={() => void invoke("cancel_install")} disabled={job?.cancelled === true}>{job?.cancelled ? "正在取消…" : "取消安装"}</button></section>}
    {tasksRunning && <section className="progress source-progress" aria-live="polite"><h3>正在处理补丁来源</h3>{sourceTasks.filter(task => task.running).map(task => {
      const width = task.progress && task.progress.total > 0 ? Math.min(100, task.progress.completed / task.progress.total * 100) : null;
      return <div className="task-progress" key={task.task_id}><h4>{taskKindLabel(task.kind)}{task.kind === "update_source" ? ` · ${task.key}` : task.kind === "match_games" ? " · 全部游戏" : ""}</h4><div className={`progress-bar${width == null ? " indeterminate" : ""}`}><div className="progress-fill" style={width == null ? undefined : { width: `${width}%` }} /></div><div className="progress-meta"><span>{taskProgressText(task)}</span>{settingsRevision != null && <span>来源设置版本 {settingsRevision}</span>}<button type="button" className="link" onClick={() => void cancelTask(task.task_id)} disabled={task.cancelled}>{task.cancelled ? "正在取消…" : "取消"}</button></div></div>;
    })}</section>}
    {!tasksRunning && sourceNotice && <div role="status" className="status">{sourceNotice}</div>}
    <div className="workspace"><aside><div className="search"><input aria-label="搜索游戏" placeholder="搜索游戏或 AppID" value={query} onChange={e => setQuery(e.target.value)} /></div>
      <div className="filters">{(["all", "matched", "installed", "attention"] as Filter[]).map((key, i) => <button key={key} className={filter === key ? "active" : ""} onClick={() => setFilter(key)}>{["全部", "有补丁", "已安装", "需处理"][i]}</button>)}</div>
      <div className="count">{shown.length} 个安装实例</div>
      {filter === "matched" && unqueried > 0 && <div className="count hint-row">还有 {unqueried} 款游戏尚未匹配来源。<button type="button" className="link" onClick={() => { setFilter("all"); setQuery(""); }}>查看全部</button></div>}
      <div className="game-list">{shown.map(row => {
        const rowBusy = scanBusy || instanceMatching(row.game.instance_id);
        const chip = patchChip(row, rowBusy);
        return <button key={row.game.instance_id} className={`game ${selected === row.game.instance_id ? "selected" : ""}`} onClick={() => detail(row)} disabled={!!busy}><span className="game-head"><strong>{gameDisplayName(row.game)}</strong>{chip && <Chip tone={chip.tone} label={chip.label} />}</span><small>AppID {row.game.appid} · {rowSummary(row, rowBusy)}</small><small>{row.game.availability === "installed" ? row.game.library_path : `不可用：${row.game.availability}`}</small></button>;
      })}</div>
      {scan && !scan.installs.length && <p className="empty">未找到已安装游戏。检查 Steam 是否安装，或确认库磁盘已挂载。</p>}
    </aside><main>{!game ? <section className="welcome"><h2>选择一个游戏</h2><p>扫描会列出可访问 Steam 库中的所有安装实例，目录未收录的游戏也会显示。</p><p>目录版本：{scan?.catalog_revision ?? "—"} · 信任状态：{workflowState(scan?.catalog_trust ?? "未加载")}</p>{scan?.diagnostics.map((d, i) => <p key={i} className="warning">{reason(d.code)} {d.path}</p>)}</section> : <>
      <div className="detail-head"><div><h2>{gameDisplayName(game.game)}</h2><p>AppID {game.game.appid} · build {game.game.buildid ?? "未知版本"}</p><p className="path">{game.game.install_path}</p></div><div className="detail-badges"><span className="badge">{rowSummary(game, scanBusy || instanceMatching(game.game.instance_id))}</span>{detailChip && <Chip tone={detailChip.tone} label={detailChip.label} />}</div></div>
      {!entries.length ? <section className="card"><h3>暂无可用补丁</h3><p>当前目录没有这款游戏的匹配条目，也没有已核实身份的来源候选。这不代表游戏不需要补丁，也不说明其补丁安装状态。</p></section> : <>
        <div className="tabs">{entries.map(item => <button key={item.key} className={`${selectedKey === item.key ? "active" : ""}${item.kind === "discovery" ? " tab-source" : ""}`} onClick={() => {setSelectedKey(item.key); setSelectedDownload(null);setManualConfirmation(null);setPreview(null);setArchive(null);}} disabled={!!busy}>{item.kind === "installable" ? item.patch.name : item.candidate.patch_title}{item.kind === "discovery" && <span className="tab-tag">来源</span>}</button>)}</div>
        {entry?.kind === "installable" && (() => {
          const current = patch!;
          const dynamic = current.support_level === "publisher_zip" || current.support_level === "publisher_exe";
          const primaryAction = primaryInstallAction(current, !!archive);
          const blockers = actionBlockers(current);
          return <>
            <section className="card"><h3>{current.name}</h3><div className="facts"><div><label>检测状态</label><strong>{patchState(current.detection.status)}</strong></div><div><label>来源能力</label><strong>{workflowState(current.support_level)}</strong></div><div><label>兼容性</label><strong>{workflowState(current.compatibility)}</strong></div><div><label>游戏版本</label><strong>{game.game.buildid ?? "未知，可手动选择"}</strong></div></div><p>补丁标识：{current.selection_id ?? current.release_id}</p><button onClick={() => openUrl(current.source_page)}>在外部浏览器打开官方来源 ↗</button>{current.support_level === "publisher_exe" && <p className="hint">安装时自动检查条件；下载后会先分析安装包，条件不满足不会写入文件。</p>}{blockers.map((b, i) => <p className="warning" key={i}>{reason(b.code)}{b.detail ? ` · ${b.detail}` : ""}</p>)}{isExternal(current.detection.status) && <p className="warning">检测到外部安装；没有本工具的原版备份，不能由此卸载。</p>}{dynamic && current.compatibility !== "manual_selection_required" && current.actions.automatic.available && <p className="hint">可自动安装；点击安装后会从来源下载并核对内容后写入。</p>}</section>
            <section className="card"><h3>安装与预览</h3>{entry.kind === "installable" && entry.choices.length > 1 && <label>下载文件与镜像<select value={current.selection_id ?? current.release_id} onChange={e => {setSelectedDownload(e.target.value);setManualConfirmation(null);setPreview(null);setArchive(null);}}>{entry.choices.map(p => <option key={p.selection_id ?? p.release_id} value={p.selection_id ?? p.release_id}>{p.download_label || "来源下载"} · {p.download_url ? downloadFileName(p.download_url) : p.name}</option>)}</select></label>}{current.download_url && <><p><b>下载项：</b>{current.download_label || downloadFileName(current.download_url)}</p><p className="path">{current.download_url}</p></>}{current.compatibility === "manual_selection_required" && game.discovery?.candidates.filter(c => c.candidate_id === current.candidate_id).map(c => <div key={c.candidate_id}><p>版本：{c.version_raw || "未标明"} · 类型：{c.variant} · 语言：{c.languages.join("、") || "未标明"}</p><p>需要 DLC：{c.requires_dlc?.join("、") || "未标明"} · 排除 DLC：{c.excludes_dlc?.join("、") || "未标明"}</p></div>)}{current.compatibility === "manual_selection_required" && <label><input type="checkbox" checked={manualConfirmation === current.selection_id} onChange={e => setManualConfirmation(e.target.checked ? current.selection_id ?? null : null)} />我选择将此候选安装到当前游戏（请核对来源、语言和 DLC；未标明的条件由你选择）</label>}<p>{primaryAction.input === "local_archive" ? "此操作需要导入本地 ZIP；查看文件变更仅供只读核对。" : "安装会按核心提供的主要操作路线核对补丁后写入游戏文件；查看文件变更仅供只读核对。"}</p><div className="actions"><button onClick={viewPreview} disabled={!!busy}>查看文件变更（只读）</button>{current.actions.local_archive.available && <button onClick={chooseArchive} disabled={!!busy}>导入本地 ZIP</button>}{archive && current.actions.local_archive.available && <button onClick={() => void startInstall("local_archive")} disabled={!!busy || installing || !primaryInstallAction(current, true, "local_archive").available}>使用本地 ZIP 安装</button>}<button className="primary" onClick={() => void startInstall()} disabled={!!busy || installing || !primaryAction.available || (current.compatibility === "manual_selection_required" && manualConfirmation !== current.selection_id)}>{primaryAction.label === "overwrite" ? "覆盖安装补丁" : "安装补丁"}</button></div>{archive && <p className="path">已选择：{archive}</p>}{preview && <ChangeList title="只读文件变更" value={preview} />}</section>
          </>;
        })()}
        {entry?.kind === "discovery" && <CandidateCard candidate={entry.candidate} onOpen={openUrl} />}
      </>}
      {(() => {
        const rematch = taskForInstance(sourceTasks, game.game.instance_id);
        const libraryMatch = sourceTasks.find(task => task.kind === "match_games" && task.key === "match");
        const active = rematch?.running ? rematch : libraryMatch?.running ? libraryMatch : undefined;
        const status = game.discovery?.status;
        const matchHint = gameMatchStatusHint(status);
        const fallbackText = fallbackDisplayStatus(status);
        const width = active?.progress && active.progress.total > 0 ? Math.min(100, active.progress.completed / active.progress.total * 100) : null;
        return <section className="card">
          <div className="card-head"><h3>来源匹配</h3><Chip tone={statusTone(status)} label={chipLabel(status, active != null)} /></div>
          <p>{statusLabel(status, active != null)}{game.discovery?.candidates?.length ? ` · 共 ${game.discovery.candidates.length} 条候选，其中 ${game.discovery.candidates.filter(candidateMatched).length} 条身份已核实` : ""}</p>
          {matchHint && <p className={status === "incomplete" ? "warning" : "hint"}>{matchHint}</p>}
          {fallbackText && status !== "no_match" && <p className="hint">{fallbackText}</p>}
          {active && <div className="source-progress-detail"><div className={`progress-bar${width == null ? " indeterminate" : ""}`}><div className="progress-fill" style={width == null ? undefined : { width: `${width}%` }} /></div><p className="hint">{taskProgressText(active)}</p></div>}
          <div className="actions"><label className="field-label">偏好的补丁语言<select value={preferredLanguage ?? ""} onChange={e => setPreferredLanguage(e.target.value || null)} disabled={!!busy}><option value="">未指定</option><option value="zh-CN">简体中文</option><option value="zh-TW">繁体中文</option><option value="en">英文</option><option value="ja">日文</option></select></label><button onClick={() => rematchGame(game.game.instance_id)} disabled={!!busy || active != null}>{active ? "正在重新匹配…" : "重新匹配"}</button>{active && <button className="cancel" onClick={() => void cancelTask(active.task_id)} disabled={active.cancelled}>{active.cancelled ? "正在取消…" : "取消匹配"}</button>}</div>
          <p className="hint">重新匹配只使用已缓存的普通来源索引，不会刷新来源；语言偏好用于优先核对，不改变游戏身份。</p>
          {game.discovery?.diagnostics.map((d, i) => <p className="warning" key={i}>{d.source_id ? `（${d.source_id}）` : ""}{reason(d.code)}{d.detail && d.detail !== d.code ? ` · ${reason(d.detail)}` : ""}</p>)}
        </section>;
      })()}
      <section className="card"><h3>本工具事务</h3>{owned.length ? owned.map(tx => <div className="transaction" key={tx.transaction_id}><span>{tx.release_id || tx.selection_id || tx.content_id || tx.transaction_id} · {workflowState(tx.state)}{tx.error && ` · ${reason(tx.error)}`}</span></div>) : <p>没有托管安装记录。外部安装不能由本工具卸载。</p>}</section>
    </>}</main></div>
    <SourcePanel
      open={sourcesOpen}
      onClose={() => setSourcesOpen(false)}
      globalSources={globalSources}
      gameSources={gameSources}
      gameName={game ? gameDisplayName(game.game) : null}
      gameInstanceId={game ? game.game.instance_id : null}
      preferredLanguage={preferredLanguage}
      settingsRevision={settingsRevision}
      loading={sourcesLoading}
      busy={busy}
      error={error}
      notice={notice}
      tasks={sourceTasks}
      onReload={reloadSources}
      onMatchAllGames={matchAllGames}
      onUpdateSource={updateSource}
      onSetInterval={setSourceInterval}
      onCancelTask={taskId => void cancelTask(taskId)}
      onRematchGame={() => { if (game) rematchGame(game.game.instance_id); }}
      onLanguageChange={setPreferredLanguage}
      onToggleSource={toggleSource}
      onToggleGameSource={toggleGameSource}
      onAddSource={addSource}
      onOpenUrl={openUrl}
    />
  </div>;
}

function ChangeList({ title, value }: { title: string; value: Preview }) {
  return <div className="change-list"><h4>{title} · {value.changes.length} 项变更</h4>{value.blockers.length > 0 && value.blockers.map((b, i) => <p className="warning" key={i}>{reason(b)}</p>)}{value.changes.map((c, i) => <div key={i}><b>{workflowState(c.action)}</b><code>{c.path}</code>{c.baseline_matches === false && <span>基线不匹配</span>}</div>)}</div>;
}

createRoot(document.getElementById("root")!).render(<App />);
