import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { filterInstalls, isExternal, patchState, reason, workflowState, type Filter, type Install, type Patch, type Plan, type Preview, type ScanResult, type Transaction } from "@steam-patch/ui";
import "./style.css";

type Op = "scan" | "preview" | "plan_install" | "fetch_artifact" | "start_install" | "list_transactions" | "uninstall" | "recover";
function call<T>(operation: Op, args: Record<string, string | boolean> = {}): Promise<T> {
  return invoke<T>("core_call", { operation, args });
}

function App() {
  const [scan, setScan] = useState<ScanResult | null>(null);
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [release, setRelease] = useState<string | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [archive, setArchive] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const game = scan?.installs.find(x => x.game.instance_id === selected) ?? null;
  const patch = game?.patches.find(x => x.release_id === release) ?? null;
  const shown = useMemo(() => filterInstalls(scan?.installs ?? [], filter, query), [scan, filter, query]);
  const owned = transactions.filter(x => x.instance_id === selected);

  async function run<T>(label: string, task: () => Promise<T>, done: (result: T) => void) {
    setBusy(label); setError(null); setNotice(null);
    try { done(await task()); }
    catch (e) { setError(reason(String(e))); }
    finally { setBusy(null); }
  }

  async function refresh() {
    await run("正在扫描 Steam 库…", async () => {
      const [nextScan, tx] = await Promise.all([call<ScanResult>("scan"), call<Transaction[] | { transactions: Transaction[] }>("list_transactions")]);
      return { nextScan, tx };
    }, ({ nextScan, tx }) => {
      setScan(nextScan);
      setTransactions(Array.isArray(tx) ? tx : tx.transactions ?? []);
      if (!nextScan.installs.some(x => x.game.instance_id === selected)) { setSelected(null); setRelease(null); }
    });
  }
  useEffect(() => { void refresh(); }, []);
  useEffect(() => { setPreview(null); setPlan(null); setArchive(null); }, [selected, release]);

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
  function makePlan() {
    if (!selected || !release) return;
    void run("正在验证目录、游戏版本与补丁包…", () => call<Plan>("plan_install", { instanceId: selected, releaseId: release }), setPlan);
  }
  function fetchArtifact() {
    if (!selected || !release) return;
    void run("正在从已审核来源获取文件…", () => call<{ archive?: string }>("fetch_artifact", { instanceId: selected, releaseId: release }), result => {
      setArchive(result.archive ?? "已下载到应用缓存"); setNotice("下载完成。请生成安装计划并核对变更。");
    });
  }
  function startInstall() {
    if (!plan?.plan_id || plan.blockers.length || plan.instance_id !== selected || plan.release_id !== release) return;
    if (!window.confirm("请先关闭游戏并暂停 Steam 更新。确认执行预览中的文件变更？")) return;
    void run("正在安装；请保持游戏和 Steam 更新关闭…", () => call<unknown>("start_install", { planId: plan.plan_id!, confirmedGameClosed: true }), () => {
      setPlan(null); setPreview(null); setNotice("安装任务已返回结果；正在重新读取事务与游戏状态。"); void refresh();
    });
  }
  function uninstall(id: string) {
    if (!window.confirm("只会恢复本工具记录且未经后续修改的文件。请先关闭游戏并暂停 Steam 更新。确认卸载此事务？")) return;
    void run("正在安全卸载…", () => call<unknown>("uninstall", { transactionId: id, confirmedGameClosed: true }), () => { setNotice("卸载完成，正在刷新状态。"); void refresh(); });
  }
  function recover() {
    if (!window.confirm("请关闭所有相关游戏并暂停 Steam 更新。确认恢复未完成事务？")) return;
    void run("正在检查并恢复未完成事务…", () => call<unknown>("recover", { confirmedGameClosed: true }), () => { setNotice("恢复检查完成。"); void refresh(); });
  }

  return <div className="shell">
    <header><div><h1>Steam 补丁管理</h1><p>本地游戏 · 已审核来源 · 文件变更先预览</p></div><div className="header-actions"><button onClick={recover} disabled={!!busy}>恢复事务</button>{busy?.includes("获取") && <button onClick={() => void invoke("cancel_download")}>取消下载</button>}<button className="primary" onClick={() => void refresh()} disabled={!!busy}>重新扫描</button></div></header>
    {busy && <div role="status" className="status">{busy} 任务完成后会显示结果；当前没有百分比进度。</div>}
    {error && <div role="alert" className="error">{error}</div>}{notice && <div role="status" className="notice">{notice}</div>}
    <div className="workspace"><aside><div className="search"><input aria-label="搜索游戏" placeholder="搜索游戏或 AppID" value={query} onChange={e => setQuery(e.target.value)} /></div>
      <div className="filters">{(["all", "matched", "installed", "attention"] as Filter[]).map((key, i) => <button key={key} className={filter === key ? "active" : ""} onClick={() => setFilter(key)}>{["全部", "有补丁", "已安装", "需处理"][i]}</button>)}</div>
      <div className="count">{shown.length} 个安装实例</div><div className="game-list">{shown.map(row => <button key={row.game.instance_id} className={`game ${selected === row.game.instance_id ? "selected" : ""}`} onClick={() => detail(row)} disabled={!!busy}><strong>{row.game.name}</strong><small>AppID {row.game.appid} · {row.patches.length ? `${row.patches.length} 个目录条目` : "目录暂无匹配"}</small><small>{row.game.availability === "installed" ? row.game.library_path : `不可用：${row.game.availability}`}</small></button>)}</div>
      {scan && !scan.installs.length && <p className="empty">未找到已安装游戏。检查 Steam 是否安装，或确认库磁盘已挂载。</p>}
    </aside><main>{!game ? <section className="welcome"><h2>选择一个游戏</h2><p>扫描会列出可访问 Steam 库中的所有安装实例，目录未收录的游戏也会显示。</p><p>目录版本：{scan?.catalog_revision ?? "—"} · 信任状态：{workflowState(scan?.catalog_trust ?? "未加载")}</p>{scan?.diagnostics.map((d, i) => <p key={i} className="warning">{reason(d.code)} {d.path}</p>)}</section> : <>
      <div className="detail-head"><div><h2>{game.game.name}</h2><p>AppID {game.game.appid} · build {game.game.buildid ?? "未知版本"}</p><p className="path">{game.game.install_path}</p></div><span className="badge">{game.patches.length ? "目录有匹配" : "目录暂无匹配"}</span></div>
      {!game.patches.length ? <section className="card"><h3>暂无已审核补丁</h3><p>当前目录没有这款游戏的匹配条目。这不代表游戏不需要补丁，也不能说明其补丁安装状态。</p></section> : <><div className="tabs">{game.patches.map(p => <button key={p.release_id} className={release === p.release_id ? "active" : ""} onClick={() => setRelease(p.release_id)} disabled={!!busy}>{p.name}</button>)}</div>{patch && <><section className="card"><h3>{patch.name}</h3><div className="facts"><div><label>检测状态</label><strong>{patchState(patch.detection.status)}</strong></div><div><label>来源能力</label><strong>{workflowState(patch.support_level)}</strong></div><div><label>兼容性</label><strong>{workflowState(patch.compatibility)}</strong></div><div><label>游戏版本</label><strong>{game.game.buildid ?? "未知，需审核"}</strong></div></div><p>发行版本：{patch.release_id}</p><button onClick={() => void invoke("open_source", { url: patch.source_page })}>在外部浏览器打开官方来源 ↗</button>{patch.blockers.map((b, i) => <p className="warning" key={i}>{reason(b)}</p>)}{isExternal(patch.detection.status) && <p className="warning">检测到外部安装；没有本工具的原版备份，不能由此卸载。</p>}</section>
        <section className="card"><h3>安装与预览</h3><p>只对可信目录、已验证游戏版本和审核 ZIP 生成可执行计划。来源要求手动下载时，在浏览器获取文件后导入。</p><div className="actions"><button onClick={viewPreview} disabled={!!busy}>查看变更预览</button><button onClick={chooseArchive} disabled={!!busy}>导入本地 ZIP</button><button onClick={fetchArtifact} disabled={!!busy || patch.support_level !== "managed_zip" || !patch.download_available}>从审核来源获取</button><button className="primary" onClick={makePlan} disabled={!!busy || !["managed_zip", "manual_download"].includes(patch.support_level)}>生成安装计划</button></div>{archive && <p className="path">已选择：{archive}</p>}{preview && <ChangeList title="只读预览" value={preview} />}{plan && <><ChangeList title="安装计划" value={plan} />{plan.plan_id && plan.blockers.length === 0 && <button className="danger" onClick={startInstall} disabled={!!busy}>确认安装这些变更</button>}</>}</section>
      </>}</>}
      <section className="card"><h3>本工具事务</h3>{owned.length ? owned.map(tx => <div className="transaction" key={tx.transaction_id}><span>{tx.release_id} · {workflowState(tx.state)}{tx.error && ` · ${reason(tx.error)}`}</span>{tx.state === "committed" && <button onClick={() => uninstall(tx.transaction_id)} disabled={!!busy}>卸载并恢复</button>}</div>) : <p>没有托管安装记录。外部安装不能由本工具卸载。</p>}</section>
    </>}</main></div>
  </div>;
}

function ChangeList({ title, value }: { title: string; value: Preview | Plan }) {
  return <div className="change-list"><h4>{title} · {value.changes.length} 项变更</h4>{value.blockers.length > 0 && value.blockers.map((b, i) => <p className="warning" key={i}>{reason(b)}</p>)}{value.changes.map((c, i) => <div key={i}><b>{workflowState(c.action)}</b><code>{c.path}</code>{c.baseline_matches === false && <span>基线不匹配</span>}</div>)}{"executable" in value && value.executable === false && <p>此结果只供查看，不能直接执行。</p>}</div>;
}

createRoot(document.getElementById("root")!).render(<App />);
