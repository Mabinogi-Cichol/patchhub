import { callable, definePlugin } from "@decky/api";
import { ButtonItem, Navigation, PanelSection, PanelSectionRow, TextField } from "@decky/ui";
import { FaPuzzlePiece } from "react-icons/fa";
import { useEffect, useMemo, useState } from "react";
import { filterInstalls, isExternal, patchState, reason, workflowState, type Filter, type Install, type Plan, type Preview, type ScanResult, type Transaction } from "@steam-patch/ui";

const scan = callable<[], ScanResult>("scan");
const preview = callable<[instanceId: string, releaseId: string], Preview>("preview");
const planInstall = callable<[instanceId: string, releaseId: string, archivePath: string], Plan>("plan_install");
const fetchArtifact = callable<[instanceId: string, releaseId: string], { archive?: string }>("fetch_artifact");
const startInstall = callable<[planId: string, confirmed: boolean], unknown>("start_install");
const listTransactions = callable<[], Transaction[] | { transactions: Transaction[] }>("list_transactions");
const uninstall = callable<[transactionId: string, confirmed: boolean], unknown>("uninstall");
const cancelDownload = callable<[], unknown>("cancel_download");
const recover = callable<[confirmed: boolean], unknown>("recover");

function Content() {
  const [report, setReport] = useState<ScanResult | null>(null);
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [release, setRelease] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [archive, setArchive] = useState("");
  const [view, setView] = useState<Preview | null>(null);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const shown = useMemo(() => filterInstalls(report?.installs ?? [], filter, query), [report, filter, query]);
  const row = report?.installs.find(i => i.game.instance_id === selected);
  const patch = row?.patches.find(p => p.release_id === release);
  const owned = transactions.filter(t => t.instance_id === selected);

  async function work<T>(label: string, fn: () => Promise<T>, onSuccess: (value: T) => void) {
    setBusy(label); setError(null); setNote(null);
    try { onSuccess(await fn()); } catch (e) { setError(reason(String(e))); } finally { setBusy(null); }
  }

  function refresh() {
    void work("正在扫描…", async () => {
      const next = await scan();
      let tx: Transaction[] = [];
      try { const result = await listTransactions(); tx = Array.isArray(result) ? result : result.transactions ?? []; }
      catch { /* Older read-only core can still show the scan. */ }
      return { next, tx };
    }, ({ next, tx }) => { setReport(next); setTransactions(tx); });
  }
  useEffect(refresh, []);

  function choose(item: Install) {
    setSelected(item.game.instance_id); setRelease(item.patches[0]?.release_id ?? null);
    setArchive(""); setView(null); setPlan(null); setConfirm(null);
  }
  function selectedRelease(id: string) { setRelease(id); setView(null); setPlan(null); setConfirm(null); }
  function source(url: string) {
    if (!url.startsWith("https://")) { setError("来源地址不是 HTTPS，已阻止打开。"); return; }
    Navigation.NavigateToExternalWeb(url);
  }
  function getPlan() {
    if (!selected || !release) return;
    void work("正在校验目录和文件…", () => planInstall(selected, release, archive), value => { setPlan(value); setConfirm(null); });
  }
  function download() {
    if (!selected || !release) return;
    void work("正在下载已审核文件…", () => fetchArtifact(selected, release), result => {
      setArchive(result.archive ?? ""); setNote("下载已结束。请生成计划并查看文件变更。");
    });
  }

  return <>
    <PanelSection title="Steam 补丁管理">{busy?.includes("下载") && <PanelSectionRow><ButtonItem onClick={() => void cancelDownload()}>取消下载</ButtonItem></PanelSectionRow>}<PanelSectionRow><ButtonItem onClick={refresh} disabled={!!busy}>重新扫描</ButtonItem></PanelSectionRow><PanelSectionRow><ButtonItem onClick={() => void work("正在恢复…", () => recover(true), () => { setNote("恢复检查完成。"); refresh(); })} disabled={!!busy}>已关闭游戏及更新，恢复未完成事务</ButtonItem></PanelSectionRow>
      {busy && <PanelSectionRow><p>{busy} 完成后显示结果；暂无百分比进度。</p></PanelSectionRow>}
      {error && <PanelSectionRow><p style={{ color: "#ffb28d" }}>{error}</p></PanelSectionRow>}
      {note && <PanelSectionRow><p>{note}</p></PanelSectionRow>}
      <PanelSectionRow><p>目录版本 {report?.catalog_revision ?? "—"} · {workflowState(report?.catalog_trust ?? "未加载")}</p></PanelSectionRow>
    </PanelSection>
    {!selected && <><PanelSection title="已安装游戏"><PanelSectionRow><TextField label="搜索游戏或 AppID" value={query} onChange={e => setQuery(e.target.value)} /></PanelSectionRow>
      <PanelSectionRow><ButtonItem onClick={() => setFilter("all")}>全部 {filter === "all" ? "✓" : ""}</ButtonItem></PanelSectionRow>
      <PanelSectionRow><ButtonItem onClick={() => setFilter("matched")}>有补丁 {filter === "matched" ? "✓" : ""}</ButtonItem></PanelSectionRow>
      <PanelSectionRow><ButtonItem onClick={() => setFilter("installed")}>已安装 {filter === "installed" ? "✓" : ""}</ButtonItem></PanelSectionRow>
      <PanelSectionRow><ButtonItem onClick={() => setFilter("attention")}>需处理 {filter === "attention" ? "✓" : ""}</ButtonItem></PanelSectionRow>
      <PanelSectionRow><p>{shown.length} 个安装实例</p></PanelSectionRow>
      {shown.map(item => <PanelSectionRow key={item.game.instance_id}><ButtonItem onClick={() => choose(item)} disabled={!!busy}>{item.game.name} · {item.patches.length ? `${item.patches.length} 个补丁` : "目录暂无匹配"}</ButtonItem></PanelSectionRow>)}
      {report && !report.installs.length && <PanelSectionRow><p>未找到 Steam 游戏。请确认库磁盘或 SD 卡已挂载。</p></PanelSectionRow>}
      {report?.diagnostics.map((d, i) => <PanelSectionRow key={i}><p>{reason(d.code)}</p></PanelSectionRow>)}
    </PanelSection></>}
    {row && <><PanelSection title={row.game.name}><PanelSectionRow><ButtonItem onClick={() => { setSelected(null); setRelease(null); }} disabled={!!busy}>← 返回游戏列表</ButtonItem></PanelSectionRow>
      <PanelSectionRow><p>AppID {row.game.appid} · build {row.game.buildid ?? "未知版本"}<br />{row.game.install_path}</p></PanelSectionRow>
      {!row.patches.length && <PanelSectionRow><p>目录暂无匹配。这不代表游戏不需要补丁，也无法判断补丁是否已安装。</p></PanelSectionRow>}
      {row.patches.map(item => <PanelSectionRow key={item.release_id}><ButtonItem onClick={() => selectedRelease(item.release_id)} disabled={!!busy}>{item.name} {release === item.release_id ? "✓" : ""}</ButtonItem></PanelSectionRow>)}
    </PanelSection>
    {patch && <><PanelSection title="补丁详情"><PanelSectionRow><p>{patch.name}<br />状态：{patchState(patch.detection.status)}<br />兼容性：{workflowState(patch.compatibility)}<br />来源级别：{workflowState(patch.support_level)}</p></PanelSectionRow>
      {patch.blockers.map((b, i) => <PanelSectionRow key={i}><p style={{ color: "#ffb28d" }}>{reason(b)}</p></PanelSectionRow>)}
      {isExternal(patch.detection.status) && <PanelSectionRow><p>外部安装没有本工具备份，不能从此处卸载。</p></PanelSectionRow>}
      <PanelSectionRow><ButtonItem onClick={() => source(patch.source_page)}>在外部浏览器打开来源 ↗</ButtonItem></PanelSectionRow>
    </PanelSection><PanelSection title="安装与预览"><PanelSectionRow><ButtonItem onClick={() => void work("正在生成只读预览…", () => preview(row.game.instance_id, patch.release_id), setView)} disabled={!!busy}>查看文件变更</ButtonItem></PanelSectionRow>
      <PanelSectionRow><TextField label="本地 ZIP 路径（仅 ~/Downloads 内）" value={archive} onChange={e => setArchive(e.target.value)} /></PanelSectionRow>
      <PanelSectionRow><p>浏览器下载后切换到桌面模式确认文件路径，再回到这里输入。导入仅允许 Downloads 内的 ZIP。</p></PanelSectionRow>
      {patch.support_level === "managed_zip" && patch.download_available && <PanelSectionRow><ButtonItem onClick={download} disabled={!!busy}>获取已审核文件</ButtonItem></PanelSectionRow>}
      <PanelSectionRow><ButtonItem onClick={getPlan} disabled={!!busy || !["managed_zip", "manual_download"].includes(patch.support_level)}>验证并生成安装计划</ButtonItem></PanelSectionRow>
      {view && <Changes title="只读预览" value={view} />}{plan && <Changes title="安装计划" value={plan} />}
      {plan?.plan_id && plan.instance_id === selected && plan.release_id === release && plan.blockers.length === 0 && <><PanelSectionRow><ButtonItem onClick={() => setConfirm(plan.plan_id!)} disabled={!!busy}>查看确认</ButtonItem></PanelSectionRow>
        {confirm === plan.plan_id && <PanelSectionRow><ButtonItem onClick={() => void work("正在安装，请保持游戏关闭…", () => startInstall(plan.plan_id!, true), () => { setPlan(null); setConfirm(null); setNote("安装已返回结果，请重新扫描核验。"); refresh(); })} disabled={!!busy}>已关闭游戏及更新，确认安装 {plan.changes.length} 项变更</ButtonItem></PanelSectionRow>}</>}
    </PanelSection></>}
    <PanelSection title="本工具事务">{owned.length ? owned.map(tx => <PanelSectionRow key={tx.transaction_id}><p>{tx.release_id} · {workflowState(tx.state)}{tx.error && ` · ${reason(tx.error)}`}</p>{tx.state === "committed" && <ButtonItem onClick={() => setConfirm(tx.transaction_id)}>卸载此事务</ButtonItem>}{confirm === tx.transaction_id && <ButtonItem onClick={() => void work("正在安全卸载…", () => uninstall(tx.transaction_id, true), () => { setConfirm(null); refresh(); })}>已关闭游戏及更新，确认恢复文件</ButtonItem>}</PanelSectionRow>) : <PanelSectionRow><p>没有托管事务；外部安装不能从此处卸载。</p></PanelSectionRow>}</PanelSection>
    </>}
  </>;
}

function Changes({ title, value }: { title: string; value: Preview | Plan }) {
  return <><PanelSectionRow><p>{title} · {value.changes.length} 项文件变更</p></PanelSectionRow>{value.blockers.map((b, i) => <PanelSectionRow key={i}><p style={{ color: "#ffb28d" }}>{reason(b)}</p></PanelSectionRow>)}{value.changes.map((c, i) => <PanelSectionRow key={i}><p>{workflowState(c.action)} · {c.path}{c.baseline_matches === false ? " · 基线不匹配" : ""}</p></PanelSectionRow>)}{"executable" in value && value.executable === false && <PanelSectionRow><p>此结果只能查看，不能直接执行。</p></PanelSectionRow>}</>;
}

export default definePlugin(() => ({ name: "Steam 补丁管理", titleView: <div>Steam 补丁管理</div>, content: <Content />, icon: <FaPuzzlePiece /> }));
