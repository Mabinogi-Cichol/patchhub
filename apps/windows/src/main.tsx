import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { canAutoInstall, filterInstalls, gameDisplayName, installSummary, isExternal, isManaged, patchState, reason, workflowState, type Filter, type Install, type Patch, type Preview, type ScanResult, type Transaction } from "@steam-patch/ui";
import "./style.css";

type CoreOperation = "scan" | "preview" | "list_transactions" | "recover" | "migrate_state";
type InstallStatus = {
  running: boolean;
  cancelled: boolean;
  progress: {
    stage: string;
    bytes_done: number;
    total_bytes: number | null;
    files_done: number;
    total_files: number | null;
    current_file: string | null;
    archive_bytes: number | null;
  } | null;
};

function call<T>(operation: CoreOperation, args: { instanceId?: string; releaseId?: string } = {}): Promise<T> {
  return invoke<T>("core_call", { operation, args });
}

let migration: Promise<void> | null = null;
function migrateOnce(): Promise<void> {
  if (!migration) migration = call("migrate_state").then(() => undefined).catch(() => undefined);
  return migration;
}

const installedStatuses = ["managed", "installed_managed", "installed_owned", "external", "installed_external"];

function projectScan(value: ScanResult, transactions: Transaction[]): ScanResult {
  const signed = value.catalog_trust === "signed";
  const committedReleaseIds = (instance: string) => new Set(transactions.filter(tx => tx.instance_id === instance && tx.state === "committed").map(tx => tx.release_id));
  return {
    ...value,
    installs: value.installs.map(row => {
      const committed = committedReleaseIds(row.game.instance_id);
      return {
        ...row,
        patches: row.patches.map(patch => {
          const auto = signed && patch.blockers.length === 0 && patch.download_available === true && patch.support_level === "managed_zip";
          return {
            ...patch,
            auto_installable: auto,
            reinstall_available: auto && (committed.has(patch.release_id) || installedStatuses.includes(patch.detection.status)),
          };
        }),
      };
    }),
  };
}

function transactionResult(tx: Transaction): string {
  const details = tx.error ? ` 原因：${reason(tx.error)}` : "";
  switch (tx.state) {
    case "committed": return "安装完成";
    case "superseded": return `之前已安装补丁，现已重新安装。${details}`;
    case "rolled_back": return `安装失败，已恢复到安装前。${details}`;
    case "needs_recovery": return `安装中断，需要恢复。${details}`;
    default: return `安装返回状态：${tx.state}。${details}`;
  }
}

function App() {
  const [scan, setScan] = useState<ScanResult | null>(null);
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [release, setRelease] = useState<string | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [archive, setArchive] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [job, setJob] = useState<InstallStatus | null>(null);
  const [installing, setInstalling] = useState(false);
  const requestIds = useRef(new Map<string, string>());

  const game = scan?.installs.find(x => x.game.instance_id === selected) ?? null;
  const patch = game?.patches.find(x => x.release_id === release) ?? null;
  const shown = useMemo(() => filterInstalls(scan?.installs ?? [], filter, query), [scan, filter, query]);
  const owned = transactions.filter(x => x.instance_id === selected);
  const scanBusy = busy?.includes("扫描") ?? false;

  async function run<T>(label: string, task: () => Promise<T>, done: (result: T) => void | Promise<void>) {
    setBusy(label); setError(null); setNotice(null);
    try { await done(await task()); }
    catch (e) { setError(reason(String(e))); }
    finally { setBusy(null); }
  }

  async function refresh() {
    await migrateOnce();
    await run("正在扫描 Steam 库…", async () => {
      // Both core calls open the same journal; finish one before starting the other.
      const nextScan = await call<ScanResult>("scan");
      const rawTransactions = await call<Transaction[] | { transactions: Transaction[] }>("list_transactions");
      const nextTransactions = Array.isArray(rawTransactions) ? rawTransactions : rawTransactions.transactions ?? [];
      return { nextScan: projectScan(nextScan, nextTransactions), nextTransactions };
    }, ({ nextScan, nextTransactions }) => {
      setScan(nextScan);
      setTransactions(nextTransactions);
      if (!nextScan.installs.some(x => x.game.instance_id === selected)) { setSelected(null); setRelease(null); }
    });
  }

  useEffect(() => { void refresh(); }, []);
  useEffect(() => { setPreview(null); setArchive(null); }, [selected, release]);
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

  function detail(row: Install) {
    setSelected(row.game.instance_id);
    setRelease(row.patches[0]?.release_id ?? null);
  }
  function viewPreview() {
    if (!selected || !release) return;
    void run("正在检查文件变更…", () => call<Preview>("preview", { instanceId: selected, releaseId: release }), setPreview);
  }
  function chooseArchive() {
    void run("正在选择 ZIP 文件…", () => invoke<string | null>("choose_archive"), value => setArchive(value));
  }
  async function startInstall(operation: "install" | "reinstall") {
    if (!selected || !release || installing || busy) return;
    const selectedArchive = archive ?? "";
    const key = `${selected}|${release}|${operation}|${selectedArchive}`;
    const requestId = requestIds.current.get(key) ?? crypto.randomUUID();
    requestIds.current.set(key, requestId);
    setInstalling(true); setJob(null); setBusy("正在安装…"); setError(null); setNotice(null);
    try {
      const tx = await invoke<Transaction>("start_install", {
        instanceId: selected,
        releaseId: release,
        operation,
        requestId,
        archive: archive ?? undefined,
      });
      const message = tx.state === "committed" && operation === "reinstall" ? "重新安装完成" : transactionResult(tx);
      requestIds.current.delete(key);
      setInstalling(false); setJob(null); setBusy(null);
      await refresh();
      setNotice(message);
    } catch (e) {
      const message = reason(String(e));
      requestIds.current.delete(key);
      setInstalling(false); setJob(null); setBusy(null);
      await refresh();
      setError(message);
    }
  }
  function recover() {
    void run("正在检查并恢复未完成事务…", () => call("recover"), async () => {
      await refresh();
      setNotice("恢复检查完成。");
    });
  }

  const stageLabels: Record<string, string> = {
    connecting: "正在连接", downloading: "下载补丁", validating: "校验文件", planning: "检查安装条件",
    backing_up: "保护现有文件", writing: "写入文件", committing: "提交安装", rolling_back: "恢复原文件",
  };
  const progress = job?.progress;
  const progressIndeterminate = !progress || !progress.total_bytes || (progress.stage === "writing" && progress.total_bytes == null);
  const progressWidth = progress && progress.total_bytes ? Math.min(100, progress.bytes_done / progress.total_bytes * 100) : 0;

  return <div className="shell">
    <header><div><h1>Steam 补丁管理</h1><p>本地游戏 · 已审核来源 · 文件变更先预览</p></div><div className="header-actions"><button onClick={recover} disabled={!!busy}>尝试恢复未完成操作</button><button className="primary" onClick={() => void refresh()} disabled={!!busy}>重新扫描</button></div></header>
    {busy && !installing && <div role="status" className="status">{busy} 任务完成后会显示结果；当前没有百分比进度。</div>}
    {error && <div role="alert" className="error">{error}</div>}{notice && <div role="status" className="notice">{notice}</div>}
    {installing && <section className="progress" aria-live="polite"><h3>{progress ? (stageLabels[progress.stage] ?? progress.stage) : "正在准备安装"}</h3><div className={`progress-bar${progressIndeterminate ? " indeterminate" : ""}`}><div className="progress-fill" style={progressIndeterminate ? undefined : { width: `${progressWidth}%` }} /></div>{progress && <div className="progress-meta"><span>{progress.total_bytes ? `${progress.bytes_done} / ${progress.total_bytes} bytes` : "正在处理"}</span>{progress.total_files != null && <span>{progress.files_done} / {progress.total_files} 个文件</span>}</div>}{progress?.current_file && <p className="path">{progress.current_file}</p>}<button className="cancel" onClick={() => void invoke("cancel_install")} disabled={job?.cancelled === true}>{job?.cancelled ? "正在取消…" : "取消安装"}</button></section>}
    <div className="workspace"><aside><div className="search"><input aria-label="搜索游戏" placeholder="搜索游戏或 AppID" value={query} onChange={e => setQuery(e.target.value)} /></div>
      <div className="filters">{(["all", "matched", "installed", "attention"] as Filter[]).map((key, i) => <button key={key} className={filter === key ? "active" : ""} onClick={() => setFilter(key)}>{["全部", "有补丁", "已安装", "需处理"][i]}</button>)}</div>
      <div className="count">{shown.length} 个安装实例</div><div className="game-list">{shown.map(row => <button key={row.game.instance_id} className={`game ${selected === row.game.instance_id ? "selected" : ""}`} onClick={() => detail(row)} disabled={!!busy}><strong>{gameDisplayName(row.game)}</strong><small>AppID {row.game.appid} · {installSummary(row, scanBusy)}</small><small>{row.game.availability === "installed" ? row.game.library_path : `不可用：${row.game.availability}`}</small></button>)}</div>
      {scan && !scan.installs.length && <p className="empty">未找到已安装游戏。检查 Steam 是否安装，或确认库磁盘已挂载。</p>}
    </aside><main>{!game ? <section className="welcome"><h2>选择一个游戏</h2><p>扫描会列出可访问 Steam 库中的所有安装实例，目录未收录的游戏也会显示。</p><p>目录版本：{scan?.catalog_revision ?? "—"} · 信任状态：{workflowState(scan?.catalog_trust ?? "未加载")}</p>{scan?.diagnostics.map((d, i) => <p key={i} className="warning">{reason(d.code)} {d.path}</p>)}</section> : <>
      <div className="detail-head"><div><h2>{gameDisplayName(game.game)}</h2><p>AppID {game.game.appid} · build {game.game.buildid ?? "未知版本"}</p><p className="path">{game.game.install_path}</p></div><span className="badge">{installSummary(game, scanBusy)}</span></div>
      {!game.patches.length ? <section className="card"><h3>暂无已审核补丁</h3><p>当前目录没有这款游戏的匹配条目。这不代表游戏不需要补丁，也不能说明其补丁安装状态。</p></section> : <><div className="tabs">{game.patches.map(p => <button key={p.release_id} className={release === p.release_id ? "active" : ""} onClick={() => setRelease(p.release_id)} disabled={!!busy}>{p.name}</button>)}</div>{patch && <>
        <section className="card"><h3>{patch.name}</h3><div className="facts"><div><label>检测状态</label><strong>{patchState(patch.detection.status)}</strong></div><div><label>来源能力</label><strong>{workflowState(patch.support_level)}</strong></div><div><label>兼容性</label><strong>{workflowState(patch.compatibility)}</strong></div><div><label>游戏版本</label><strong>{game.game.buildid ?? "未知，需审核"}</strong></div></div><p>发行版本：{patch.release_id}</p><button onClick={() => void invoke("open_source", { url: patch.source_page })}>在外部浏览器打开官方来源 ↗</button>{patch.blockers.map((b, i) => <p className="warning" key={i}>{reason(b)}</p>)}{isExternal(patch.detection.status) && <p className="warning">检测到外部安装；没有本工具的原版备份，不能由此卸载。</p>}</section>
        {patch.support_level === "managed_zip" && (() => { const installed = patch.reinstall_available === true || isManaged(patch.detection.status) || isExternal(patch.detection.status); const operation = installed ? "reinstall" : "install"; const allowed = canAutoInstall(patch) || !!archive; return <section className="card"><h3>安装与预览</h3><p>安装将直接提交审核 ZIP 的文件变更；查看文件变更仅供只读核对。</p><div className="actions"><button onClick={viewPreview} disabled={!!busy}>查看文件变更（只读）</button>{!canAutoInstall(patch) && <button onClick={chooseArchive} disabled={!!busy}>导入本地 ZIP</button>}{allowed && <button className="primary" onClick={() => void startInstall(operation)} disabled={!!busy || installing}>{installed ? "重新安装补丁" : "安装补丁"}</button>}</div>{archive && <p className="path">已选择：{archive}</p>}{preview && <ChangeList title="只读文件变更" value={preview} />}</section>; })()}
      </>}</>}
      <section className="card"><h3>本工具事务</h3>{owned.length ? owned.map(tx => <div className="transaction" key={tx.transaction_id}><span>{tx.release_id} · {workflowState(tx.state)}{tx.error && ` · ${reason(tx.error)}`}</span></div>) : <p>没有托管安装记录。外部安装不能由本工具卸载。</p>}</section>
    </>}</main></div>
  </div>;
}

function ChangeList({ title, value }: { title: string; value: Preview }) {
  return <div className="change-list"><h4>{title} · {value.changes.length} 项变更</h4>{value.blockers.length > 0 && value.blockers.map((b, i) => <p className="warning" key={i}>{reason(b)}</p>)}{value.changes.map((c, i) => <div key={i}><b>{workflowState(c.action)}</b><code>{c.path}</code>{c.baseline_matches === false && <span>基线不匹配</span>}</div>)}</div>;
}

createRoot(document.getElementById("root")!).render(<App />);
