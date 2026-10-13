import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { candidateMatched, displayPatches, filterInstalls, gameDisplayName, isExternal, isManaged, patchState, reason, installationResult, progressStageLabel, progressPercent, workflowState, type DiscoveryResult, type Filter, type GameSourceSubscription, type Install, type Preview, type ScanResult, type SourceCandidate, type Transaction, type InstallerOutcome } from "@steam-patch/ui";
import {
  actionBlockers,
  primaryInstallAction,
  candidateConditions,
  candidateFormat,
  candidateLanguage,
  candidateSourceLabel,
  chipLabel,
  matchBasisLabel,
  patchChip,
  rowSummary,
  safeSourceUrl,
  sourceChip,
  statusLabel,
  statusTone,
  unqueriedCount,
  versionEvidence,
  type Capabilities,
  type SourceEvent,
  type SourceScanItem,
  type SourceScanResult,
  type SourceSummary,
} from "./sources";
import { SourcePanel } from "./SourcePanel";
import "./style.css";

type CoreOperation = "scan" | "preview" | "list_transactions" | "recover";
type InstallStatus = {
  running: boolean;
  cancelled: boolean;
  progress: Omit<import("../../../packages/contracts/generated/protocol").InstallProgressEvent, "event" | "protocol_version" | "request_id" | "sequence" | "instance_id" | "selection_id"> | null;
};

type SourceScanState = {
  started: boolean;
  running: boolean;
  cancelled: boolean;
  completed: number;
  total: number;
  settingsRevision: number | null;
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


const EMPTY_SOURCE_SCAN: SourceScanState = { started: false, running: false, cancelled: false, completed: 0, total: 0, settingsRevision: null };

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
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [preferredLanguage, setPreferredLanguage] = useState<string | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [archive, setArchive] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [job, setJob] = useState<InstallStatus | null>(null);
  const [installing, setInstalling] = useState(false);
  const [sourceScan, setSourceScan] = useState<SourceScanState>(EMPTY_SOURCE_SCAN);
  const [sourceNotice, setSourceNotice] = useState<string | null>(null);
  const [capabilities, setCapabilities] = useState<Capabilities | null>(null);
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const [globalSources, setGlobalSources] = useState<SourceSummary[] | null>(null);
  const [gameSources, setGameSources] = useState<GameSourceSubscription[] | null>(null);
  const [sourcesLoading, setSourcesLoading] = useState(false);
  const [discoveryBusy, setDiscoveryBusy] = useState<string | null>(null);

  const requestIds = useRef(new Map<string, string>());
  const scanRef = useRef<ScanResult | null>(null);
  const selectedRef = useRef<string | null>(null);
  const capabilitiesPromise = useRef<Promise<Capabilities | null> | null>(null);
  const capabilityErrorRef = useRef<string | null>(null);
  const sourceRunningRef = useRef(false);
  const cancelledRef = useRef(false);
  const scanTokenRef = useRef(0);
  const activeEventRef = useRef<{ token: number; id: string | null; seq: number }>({ token: 0, id: null, seq: -1 });

  const game = scan?.installs.find(x => x.game.instance_id === selected) ?? null;
  const entries = useMemo(() => (game ? displayPatches(game, preferredLanguage) : []), [game, preferredLanguage]);
  const entry = entries.find(item => item.key === selectedKey) ?? entries[0] ?? null;
  const patch = entry?.kind === "installable" ? entry.patch : null;
  const shown = useMemo(() => filterInstalls(scan?.installs ?? [], filter, query), [scan, filter, query]);
  const owned = transactions.filter(x => x.instance_id === selected);
  const scanBusy = busy?.includes("扫描") ?? false;
  const settling = scanBusy || sourceScan.running;
  const unqueried = useMemo(() => unqueriedCount(scan?.installs ?? []), [scan]);
  const settingsRevision = sourceScan.settingsRevision ?? scan?.settings_revision ?? null;

  useEffect(() => { scanRef.current = scan; }, [scan]);
  useEffect(() => { selectedRef.current = selected; }, [selected]);
  useEffect(() => {
    if (entries.some(item => item.key === selectedKey)) return;
    setSelectedKey(entries[0]?.key ?? null);
  }, [entries, selectedKey]);

  function supportsSourceScan(caps: Capabilities | null): boolean {
    return !!caps && Array.isArray(caps.methods) && caps.methods.includes("scan_sources");
  }

  // Capability negotiation is the single version/availability gate. A failure
  // is a real contract error: show the host diagnostic instead of falling back.
  function loadCapabilities(): Promise<Capabilities | null> {
    if (!capabilitiesPromise.current) {
      capabilitiesPromise.current = invoke<Capabilities>("get_capabilities")
        .then(caps => { setCapabilities(caps); capabilityErrorRef.current = null; return caps; })
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

  // ---- source discovery: live scan events --------------------------------
  function handleSourceEvent(payload: SourceEvent) {
    if (!payload || typeof payload.request_id !== "string" || typeof payload.sequence !== "number") return;
    const active = activeEventRef.current;
    if (active.token === 0) return;
    if (active.id === null) { active.id = payload.request_id; active.seq = payload.sequence; }
    else if (active.id !== payload.request_id || payload.sequence <= active.seq) return;
    else active.seq = payload.sequence;

    if (payload.event === "source_progress") {
      setSourceScan(prev => ({
        ...prev,
        running: payload.running ?? prev.running,
        completed: payload.completed ?? prev.completed,
        total: payload.total ?? prev.total,
        settingsRevision: payload.settings_revision ?? prev.settingsRevision,
      }));
      return;
    }
    if (payload.event === "source_update" && "discovery" in payload) {
      const discovery = payload.discovery;
      const instanceId = payload.instance_id ?? null;
      const appid = payload.appid ?? null;
      setScan(prev => {
        if (!prev) return prev;
        let changed = false;
        const installs = prev.installs.map(row => {
          const match = instanceId ? row.game.instance_id === instanceId : appid != null && row.game.appid === appid;
          if (!match) return row;
          changed = true;
          return { ...row, discovery };
        });
        return changed ? { ...prev, installs } : prev;
      });
    }
  }

  useEffect(() => {
    let disposed = false;
    const unlisten: Array<() => void> = [];
    void (async () => {
      const progress = await listen<SourceEvent>("source_progress", event => handleSourceEvent(event.payload));
      const update = await listen<SourceEvent>("source_update", event => handleSourceEvent(event.payload));
      if (disposed) { progress(); update(); } else { unlisten.push(progress, update); }
    })();
    return () => { disposed = true; unlisten.forEach(off => off()); };
  }, []);

  function applyScanResults(result: SourceScanResult) {
    setScan(prev => {
      if (!prev) return prev;
      const byInstance = new Map<string, SourceScanItem>();
      for (const item of result.results ?? []) if (item.instance_id) byInstance.set(item.instance_id, item);
      const installs = prev.installs.map(row => {
        const item = byInstance.get(row.game.instance_id);
        if (!item) return row;
        return { ...row, discovery: { status: item.status, candidates: item.candidates ?? [], diagnostics: item.diagnostics ?? [] } };
      });
      return { ...prev, installs, settings_revision: result.settings_revision ?? prev.settings_revision };
    });
    if (result.settings_revision != null) setSourceScan(prev => ({ ...prev, settingsRevision: result.settings_revision ?? prev.settingsRevision }));
  }

  async function startSourceScan(force: boolean) {
    if (sourceRunningRef.current) return;
    const token = ++scanTokenRef.current;
    activeEventRef.current = { token, id: null, seq: -1 };
    cancelledRef.current = false;
    sourceRunningRef.current = true;
    setSourceNotice(null);
    setSourceScan({ started: true, running: true, cancelled: false, completed: 0, total: 0, settingsRevision: null });
    try {
      const result = await invoke<SourceScanResult>("start_source_scan", { force });
      if (scanTokenRef.current === token) applyScanResults(result);
      if (cancelledRef.current) setSourceNotice("已取消查找，已保留取得的结果；可稍后重新查找。");
    } catch (e) {
      if (!(e && typeof e === "object" && "code" in e && e.code === "source_scan_in_progress")) setSourceNotice(describeError(e));
    } finally {
      if (scanTokenRef.current === token) {
        activeEventRef.current = { token: 0, id: null, seq: -1 };
        sourceRunningRef.current = false;
        setSourceScan(prev => ({ ...prev, running: false, cancelled: cancelledRef.current }));
      }
    }
  }

  async function cancelSourceScan() {
    cancelledRef.current = true;
    setSourceScan(prev => ({ ...prev, cancelled: true }));
    try {
      const accepted = await invoke<boolean>("cancel_source_scan");
      // The task may have finished between the click and the request; only keep
      // the cancelled state when something was actually still running.
      if (!accepted && !sourceRunningRef.current) {
        cancelledRef.current = false;
        setSourceScan(prev => ({ ...prev, cancelled: false }));
      }
    } catch (e) {
      setError(describeError(e));
    }
  }

  async function findGame(instanceId: string, force: boolean) {
    if (discoveryBusy) return;
    setDiscoveryBusy(instanceId); setError(null); setNotice(null);
    try {
      const args: Record<string, unknown> = { instanceId, force };
      if (preferredLanguage) args.preferredLanguage = preferredLanguage;
      const result = await invoke<DiscoveryResult>("discover_game", args);
      mergeDiscoveryResult(instanceId, result);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setDiscoveryBusy(null);
    }
  }

  function mergeDiscoveryResult(instanceId: string, result: DiscoveryResult) {
    const current = scanRef.current;
    const row = current?.installs.find(item => item.game.instance_id === instanceId);
    if (!current || !row) return;
    if (result.instance_id && result.instance_id !== instanceId) return;
    if (result.appid != null && result.appid !== row.game.appid) return;
    if (result.status === "stale") { setNotice("来源设置已变化，结果可能已过期，请重新查找。"); return; }
    if (current.settings_revision != null && result.settings_revision != null && current.settings_revision !== result.settings_revision) {
      setNotice("来源设置已变化，请重新查找。");
      return;
    }
    const discovery = { status: result.status, candidates: result.candidates ?? [], diagnostics: result.diagnostics ?? [] };
    setScan(prev => prev ? { ...prev, installs: prev.installs.map(item => item.game.instance_id === instanceId ? { ...item, discovery } : item) } : prev);
  }

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
      if (value.sources) setGlobalSources(value.sources);
      if (!value.installs.some(x => x.game.instance_id === selectedRef.current)) { setSelected(null); setSelectedKey(null); }
    });
    if (capabilityErrorRef.current) setError(capabilityErrorRef.current);
    const hasInstalled = nextScan ? (nextScan as ScanResult).installs.some(row => row.game.availability === "installed") : false;
    if (supportsSourceScan(caps) && hasInstalled) void startSourceScan(false);
  }

  useEffect(() => { void refresh(); }, []);

  useEffect(() => {
    if (!sourcesOpen) return;
    void loadSources();
  }, [sourcesOpen, selected]);

  useEffect(() => { setPreview(null); setArchive(null); }, [selected, selectedKey]);

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
  async function loadSources() {
    setSourcesLoading(true);
    try {
      const list = await invoke<SourceSummary[]>("list_sources");
      setGlobalSources(list);
      if (selected) setGameSources(await invoke<GameSourceSubscription[]>("list_game_sources", { instanceId: selected }));
      else setGameSources(null);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setSourcesLoading(false);
    }
  }

  function reloadSources() {
    void run("正在读取来源列表…", async () => {
      const list = await invoke<SourceSummary[]>("list_sources");
      const games = selected ? await invoke<GameSourceSubscription[]>("list_game_sources", { instanceId: selected }) : null;
      return { list, games };
    }, ({ list, games }) => { setGlobalSources(list); setGameSources(games); });
  }

  function toggleSource(sourceId: string, enabled: boolean) {
    void run(enabled ? "正在启用来源…" : "正在停用来源…", async () => {
      await invoke("set_source_enabled", { sourceId, enabled });
      const list = await invoke<SourceSummary[]>("list_sources");
      const games = selected ? await invoke<GameSourceSubscription[]>("list_game_sources", { instanceId: selected }) : null;
      return { list, games };
    }, ({ list, games }) => { setGlobalSources(list); setGameSources(games); setNotice(`已${enabled ? "启用" : "停用"}来源。`); });
  }

  function toggleGameSource(sourceId: string, subscribed: boolean) {
    if (!selected) return;
    void run("正在更新此游戏的订阅…", async () => {
      await invoke("set_game_source", { instanceId: selected, sourceId, subscribed });
      const list = await invoke<SourceSummary[]>("list_sources");
      const games = await invoke<GameSourceSubscription[]>("list_game_sources", { instanceId: selected });
      return { list, games };
    }, ({ list, games }) => { setGlobalSources(list); setGameSources(games); setNotice("此游戏的订阅已更新，可点击“重新查找”生效。"); });
  }

  function refreshSources(force: boolean) {
    void run(force ? "正在强制刷新来源索引…" : "正在刷新来源索引…", async () => {
      await invoke("refresh_sources", { force });
      return invoke<SourceSummary[]>("list_sources");
    }, list => { setGlobalSources(list); setNotice(force ? "已强制刷新来源索引。" : "已刷新来源索引。"); });
  }

  function addSource(rule: string) {
    let parsed: unknown;
    try { parsed = JSON.parse(rule); } catch { setError("来源规则不是有效的 JSON，请检查后重试。"); return; }
    void run("正在添加来源…", async () => {
      await invoke("add_source", { rule: parsed });
      return invoke<SourceSummary[]>("list_sources");
    }, list => { setGlobalSources(list); setNotice("已添加来源。"); });
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
    return selectedEntry?.kind === "installable" ? selectedEntry.patch : null;
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
    if (!action.available) return;
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
  const detailChip = game ? sourceChip(game, sourceScan.running) : null;

  return <div className="shell">
    <header><div><h1>Steam 补丁管理</h1><p>本地游戏 · 已审核来源 · 文件变更先预览</p></div><div className="header-actions"><button onClick={() => setSourcesOpen(true)}>来源管理</button><button onClick={recover} disabled={!!busy}>尝试恢复未完成操作</button><button className="primary" onClick={() => void refresh()} disabled={!!busy}>重新扫描</button></div></header>
    {busy && !installing && <div role="status" className="status">{busy} 任务完成后会显示结果；当前没有百分比进度。</div>}
    {error && <div role="alert" className="error">{error}</div>}{notice && <div role="status" className="notice">{notice}</div>}
    {installing && <section className="progress" aria-live="polite"><h3>{progress ? progressStageLabel(progress.stage, progress.substage) : "正在准备安装"}</h3><div className={`progress-bar${progressIndeterminate ? " indeterminate" : ""}`}><div className="progress-fill" style={progressIndeterminate ? undefined : { width: `${progressWidth}%` }} /></div>{progress && <div className="progress-meta"><span>{progress.total_bytes ? `${progress.bytes_done} / ${progress.total_bytes} bytes` : "正在处理"}</span>{progress.total_files != null && <span>{progress.files_done} / {progress.total_files} 个文件</span>}</div>}{progress?.current_file && <p className="path">{progress.current_file}</p>}<button className="cancel" onClick={() => void invoke("cancel_install")} disabled={job?.cancelled === true}>{job?.cancelled ? "正在取消…" : "取消安装"}</button></section>}
    {sourceScan.running && <section className="progress source-progress" aria-live="polite"><h3>正在查找补丁来源</h3><div className={`progress-bar${sourceScan.total > 0 ? "" : " indeterminate"}`}><div className="progress-fill" style={sourceScan.total > 0 ? { width: `${(progressPercent(sourceScan.completed, sourceScan.total) ?? 0)}%` } : undefined} /></div><div className="progress-meta"><span>{sourceScan.total > 0 ? `已查询 ${sourceScan.completed} / ${sourceScan.total} 款游戏` : "正在准备查询已安装游戏"}</span>{settingsRevision != null && <span>来源设置版本 {settingsRevision}</span>}</div><button className="cancel" onClick={() => void cancelSourceScan()} disabled={sourceScan.cancelled}>{sourceScan.cancelled ? "正在取消…" : "取消查找"}</button></section>}
    {!sourceScan.running && sourceNotice && <div role="status" className="status">{sourceNotice}</div>}
    <div className="workspace"><aside><div className="search"><input aria-label="搜索游戏" placeholder="搜索游戏或 AppID" value={query} onChange={e => setQuery(e.target.value)} /></div>
      <div className="filters">{(["all", "matched", "installed", "attention"] as Filter[]).map((key, i) => <button key={key} className={filter === key ? "active" : ""} onClick={() => setFilter(key)}>{["全部", "有补丁", "已安装", "需处理"][i]}</button>)}</div>
      <div className="count">{shown.length} 个安装实例</div>
      {filter === "matched" && unqueried > 0 && <div className="count hint-row">还有 {unqueried} 款游戏未查询来源。<button type="button" className="link" onClick={() => { setFilter("all"); setQuery(""); }}>查看全部</button></div>}
      <div className="game-list">{shown.map(row => {
        const chip = patchChip(row, settling);
        return <button key={row.game.instance_id} className={`game ${selected === row.game.instance_id ? "selected" : ""}`} onClick={() => detail(row)} disabled={!!busy}><span className="game-head"><strong>{gameDisplayName(row.game)}</strong>{chip && <Chip tone={chip.tone} label={chip.label} />}</span><small>AppID {row.game.appid} · {rowSummary(row, settling)}</small><small>{row.game.availability === "installed" ? row.game.library_path : `不可用：${row.game.availability}`}</small></button>;
      })}</div>
      {scan && !scan.installs.length && <p className="empty">未找到已安装游戏。检查 Steam 是否安装，或确认库磁盘已挂载。</p>}
    </aside><main>{!game ? <section className="welcome"><h2>选择一个游戏</h2><p>扫描会列出可访问 Steam 库中的所有安装实例，目录未收录的游戏也会显示。</p><p>目录版本：{scan?.catalog_revision ?? "—"} · 信任状态：{workflowState(scan?.catalog_trust ?? "未加载")}</p>{scan?.diagnostics.map((d, i) => <p key={i} className="warning">{reason(d.code)} {d.path}</p>)}</section> : <>
      <div className="detail-head"><div><h2>{gameDisplayName(game.game)}</h2><p>AppID {game.game.appid} · build {game.game.buildid ?? "未知版本"}</p><p className="path">{game.game.install_path}</p></div><div className="detail-badges"><span className="badge">{rowSummary(game, settling)}</span>{detailChip && <Chip tone={detailChip.tone} label={detailChip.label} />}</div></div>
      {!entries.length ? <section className="card"><h3>暂无可用补丁</h3><p>当前目录没有这款游戏的匹配条目，也没有已核实身份的来源候选。这不代表游戏不需要补丁，也不说明其补丁安装状态。</p></section> : <>
        <div className="tabs">{entries.map(item => <button key={item.key} className={`${selectedKey === item.key ? "active" : ""}${item.kind === "discovery" ? " tab-source" : ""}`} onClick={() => setSelectedKey(item.key)} disabled={!!busy}>{item.kind === "installable" ? item.patch.name : item.candidate.patch_title}{item.kind === "discovery" && <span className="tab-tag">来源</span>}</button>)}</div>
        {entry?.kind === "installable" && (() => {
          const current = entry.patch;
          const dynamic = current.support_level === "publisher_zip" || current.support_level === "publisher_exe";
          const primaryAction = primaryInstallAction(current, !!archive);
          const blockers = actionBlockers(current);
          return <>
            <section className="card"><h3>{current.name}</h3><div className="facts"><div><label>检测状态</label><strong>{patchState(current.detection.status)}</strong></div><div><label>来源能力</label><strong>{workflowState(current.support_level)}</strong></div><div><label>兼容性</label><strong>{workflowState(current.compatibility)}</strong></div><div><label>游戏版本</label><strong>{game.game.buildid ?? "未知，需审核"}</strong></div></div><p>补丁标识：{current.selection_id ?? current.release_id}</p><button onClick={() => openUrl(current.source_page)}>在外部浏览器打开官方来源 ↗</button>{current.support_level === "publisher_exe" && <p className="hint">安装时自动检查条件；下载后会先分析安装包，条件不满足不会写入文件。</p>}{blockers.map((b, i) => <p className="warning" key={i}>{reason(b.code)}{b.detail ? ` · ${b.detail}` : ""}</p>)}{isExternal(current.detection.status) && <p className="warning">检测到外部安装；没有本工具的原版备份，不能由此卸载。</p>}{dynamic && current.actions.automatic.available && <p className="hint">可自动安装；点击安装后会从来源下载并核对内容后写入。</p>}</section>
            <section className="card"><h3>安装与预览</h3><p>{primaryAction.input === "local_archive" ? "此操作需要导入本地 ZIP；查看文件变更仅供只读核对。" : "安装会按核心提供的主要操作路线核对补丁后写入游戏文件；查看文件变更仅供只读核对。"}</p><div className="actions"><button onClick={viewPreview} disabled={!!busy}>查看文件变更（只读）</button>{current.actions.local_archive.available && <button onClick={chooseArchive} disabled={!!busy}>导入本地 ZIP</button>}{archive && current.actions.local_archive.available && <button onClick={() => void startInstall("local_archive")} disabled={!!busy || installing || !primaryInstallAction(current, true, "local_archive").available}>使用本地 ZIP 安装</button>}<button className="primary" onClick={() => void startInstall()} disabled={!!busy || installing || !primaryAction.available}>{primaryAction.label === "overwrite" ? "覆盖安装补丁" : "安装补丁"}</button></div>{archive && <p className="path">已选择：{archive}</p>}{preview && <ChangeList title="只读文件变更" value={preview} />}</section>
          </>;
        })()}
        {entry?.kind === "discovery" && <CandidateCard candidate={entry.candidate} onOpen={openUrl} />}
      </>}
      <section className="card"><div className="card-head"><h3>查找来源</h3><Chip tone={statusTone(game.discovery?.status)} label={chipLabel(game.discovery?.status, sourceScan.running)} /></div><p>{statusLabel(game.discovery?.status, sourceScan.running)}{game.discovery?.candidates?.length ? ` · 共 ${game.discovery.candidates.length} 条候选，其中 ${game.discovery.candidates.filter(candidateMatched).length} 条身份已核实` : ""}</p><div className="actions"><label className="field-label">偏好的补丁语言<select value={preferredLanguage ?? ""} onChange={e => setPreferredLanguage(e.target.value || null)} disabled={!!busy}><option value="">未指定</option><option value="zh-CN">简体中文</option><option value="zh-TW">繁体中文</option><option value="en">英文</option><option value="ja">日文</option></select></label><button onClick={() => void findGame(game.game.instance_id, true)} disabled={!!busy || discoveryBusy === game.game.instance_id}>{discoveryBusy === game.game.instance_id ? "正在查找…" : "重新查找"}</button></div><p className="hint">语言偏好用于向来源查询时优先核对，不改变游戏身份；游戏实际语言与 DLC 状态仍需确认。</p>{game.discovery?.diagnostics.map((d, i) => <p className="warning" key={i}>{d.source_id ? `（${d.source_id}）` : ""}{reason(d.code)}{d.detail && d.detail !== d.code ? ` · ${reason(d.detail)}` : ""}</p>)}</section>
      <section className="card"><h3>本工具事务</h3>{owned.length ? owned.map(tx => <div className="transaction" key={tx.transaction_id}><span>{tx.release_id || tx.selection_id || tx.content_id || tx.transaction_id} · {workflowState(tx.state)}{tx.error && ` · ${reason(tx.error)}`}</span></div>) : <p>没有托管安装记录。外部安装不能由本工具卸载。</p>}</section>
    </>}</main></div>
    <SourcePanel
      open={sourcesOpen}
      onClose={() => setSourcesOpen(false)}
      globalSources={globalSources}
      gameSources={gameSources}
      gameName={game ? gameDisplayName(game.game) : null}
      settingsRevision={settingsRevision}
      loading={sourcesLoading}
      busy={busy}
      error={error}
      notice={notice}
      scan={{ running: sourceScan.running, cancelled: sourceScan.cancelled, completed: sourceScan.completed, total: sourceScan.total }}
      onReload={reloadSources}
      onRefreshSources={refreshSources}
      onToggleSource={toggleSource}
      onToggleGameSource={toggleGameSource}
      onAddSource={addSource}
      onCancelScan={() => void cancelSourceScan()}
      onOpenUrl={openUrl}
    />
  </div>;
}

function ChangeList({ title, value }: { title: string; value: Preview }) {
  return <div className="change-list"><h4>{title} · {value.changes.length} 项变更</h4>{value.blockers.length > 0 && value.blockers.map((b, i) => <p className="warning" key={i}>{reason(b)}</p>)}{value.changes.map((c, i) => <div key={i}><b>{workflowState(c.action)}</b><code>{c.path}</code>{c.baseline_matches === false && <span>基线不匹配</span>}</div>)}</div>;
}

createRoot(document.getElementById("root")!).render(<App />);
