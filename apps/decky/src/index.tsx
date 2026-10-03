import { callable, definePlugin } from "@decky/api";
import { ButtonItem, DropdownItem, Navigation, PanelSection, PanelSectionRow, TextField } from "@decky/ui";
import { FaGamepad, FaPuzzlePiece } from "react-icons/fa";
import { Fragment, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ComponentProps } from "react";
import { getListPreferences, setListPreferences, subscribeListPreferences } from "./preferences";
import { gamePresentation } from "./steam";
import { installBlocker, planExpired, preparedSource, transactionResult, type PreparedInstall } from "./install";
import { filterInstalls, isExternal, patchState, reason, workflowState, type Filter, type Install, type Plan, type Preview, type ScanResult, type Transaction, type SourceSubscription } from "@steam-patch/ui";

const scan = callable<[], ScanResult>("scan");
const preview = callable<[instanceId: string, releaseId: string], Preview>("preview");
const planInstall = callable<[instanceId: string, releaseId: string, archivePath: string], Plan>("plan_install");
const prepareInstall = callable<[instanceId: string, releaseId: string], PreparedInstall>("prepare_install");
const startInstall = callable<[planId: string, confirmed: boolean], Transaction>("start_install");
const listTransactions = callable<[], Transaction[] | { transactions: Transaction[] }>("list_transactions");
const uninstall = callable<[transactionId: string, confirmed: boolean], unknown>("uninstall");
const cancelDownload = callable<[], unknown>("cancel_download");
const recover = callable<[confirmed: boolean], unknown>("recover");
const setSourceEnabled = callable<[sourceId: string, enabled: boolean], SourceSubscription[]>("set_source_enabled");
const addSource = callable<[ruleJson: string], SourceSubscription[]>("add_source");

// QAM defaults to inline settings rows, reserving the left half for a label.
// Actions have no label, so explicitly put their control on a full-width row.
function ActionButton({ children, ...props }: ComponentProps<typeof ButtonItem>) {
  return <ButtonItem {...props} layout="below" bottomSeparator="none">
    <span style={{ whiteSpace: "normal", overflowWrap: "anywhere", lineHeight: 1.4 }}>{children}</span>
  </ButtonItem>;
}

const filterOptions: { data: Filter; label: string }[] = [
  { data: "all", label: "全部" },
  { data: "matched", label: "有补丁" },
  { data: "installed", label: "已安装补丁" },
  { data: "attention", label: "需要处理" },
];

const panelStyles = `
  .steam-patch-panel { min-width: 0; }
  .steam-patch-panel button {
    box-sizing: border-box;
    width: 100%;
    min-width: 0;
    white-space: normal;
    overflow-wrap: anywhere;
  }
  .steam-patch-panel p {
    margin: 8px 0;
    line-height: 1.4;
    overflow-wrap: anywhere;
  }
  .steam-patch-panel input { box-sizing: border-box; width: 100%; min-width: 0; }
  .steam-patch-game { display: flex; align-items: center; gap: 10px; text-align: left; }
  .steam-patch-game-icon { width: 36px; height: 36px; flex: 0 0 36px; border-radius: 4px; object-fit: cover; }
  .steam-patch-game-placeholder { display: flex; align-items: center; justify-content: center; background: #263647; }
  .steam-patch-game-text { min-width: 0; }
  .steam-patch-game-name { display: block; font-weight: 600; }
  .steam-patch-game-status { display: block; font-size: 12px; color: #b8c5d3; margin-top: 3px; }
`;

function GameButton({ item, onClick, disabled }: { item: Install; onClick: () => void; disabled: boolean }) {
  const { name, icon } = gamePresentation(item.game.appid, item.game.name);
  const [failedIcon, setFailedIcon] = useState<string | undefined>();
  return <ActionButton onClick={onClick} disabled={disabled}>
    <span className="steam-patch-game">
      {icon && icon !== failedIcon
        ? <img className="steam-patch-game-icon" src={icon} alt="" loading="lazy" onError={() => setFailedIcon(icon)} />
        : <span className="steam-patch-game-icon steam-patch-game-placeholder"><FaGamepad aria-hidden="true" /></span>}
      <span className="steam-patch-game-text"><span className="steam-patch-game-name">{name}</span>
        <span className="steam-patch-game-status">{item.patches.length ? `找到 ${item.patches.length} 个补丁` : "暂未找到补丁"}</span>
      </span>
    </span>
  </ActionButton>;
}

function Content() {
  const [report, setReport] = useState<ScanResult | null>(null);
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [release, setRelease] = useState<string | null>(null);
  const { filter, query } = useSyncExternalStore(subscribeListPreferences, getListPreferences);
  const [archive, setArchive] = useState("");
  const [showManualArchive, setShowManualArchive] = useState(false);
  const [view, setView] = useState<Preview | null>(null);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [busySince, setBusySince] = useState<number | null>(null);
  const [now, setNow] = useState(Date.now());
  const requestEpoch = useRef(0);
  const [showSources, setShowSources] = useState(false);
  const [sourceRule, setSourceRule] = useState("");

  useEffect(() => () => { requestEpoch.current++; }, []);

  useEffect(() => {
    if (!busy && !plan) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [busy, plan]);

  const shown = useMemo(() => {
    const search = query.trim().toLocaleLowerCase();
    return filterInstalls(report?.installs ?? [], filter, "").filter(item =>
      !search || [item.game.name, gamePresentation(item.game.appid, item.game.name).name, String(item.game.appid)]
        .some(value => value.toLocaleLowerCase().includes(search)));
  }, [report, filter, query]);
  const row = report?.installs.find(i => i.game.instance_id === selected);
  const patch = row?.patches.find(p => p.release_id === release);
  const owned = transactions.filter(t => t.instance_id === selected);
  const blocker = patch ? installBlocker(patch) : null;
  const activePlan = plan && plan.instance_id === selected && plan.release_id === release && !blocker ? plan : null;
  const expired = activePlan ? planExpired(activePlan, now) : false;

  async function work<T>(label: string, fn: () => Promise<T>, onSuccess: (value: T) => void | Promise<void>) {
    setBusy(label); setBusySince(Date.now()); setError(null); setNote(null);
    try { await onSuccess(await fn()); } catch (e) { setError(reason(String(e))); } finally { setBusy(null); setBusySince(null); }
  }

  async function readSnapshot() {
    const next = await scan();
    let tx: Transaction[] = [];
    try { const result = await listTransactions(); tx = Array.isArray(result) ? result : result.transactions ?? []; }
    catch { /* Older read-only core can still show the scan. */ }
    return { next, tx };
  }
  function refresh() {
    void work("正在扫描…", readSnapshot, ({ next, tx }) => { setReport(next); setTransactions(tx); });
  }
  function changeSources(action: () => Promise<SourceSubscription[]>) {
    requestEpoch.current++;
    setPlan(null); setConfirm(null);
    void work("正在更新补丁源并查询…", async () => { await action(); return readSnapshot(); }, ({ next, tx }) => {
      setReport(next); setTransactions(tx); setSourceRule("");
    });
  }
  useEffect(refresh, []);

  function choose(item: Install) {
    requestEpoch.current++;
    setSelected(item.game.instance_id); setRelease(item.patches[0]?.release_id ?? null);
    setArchive(""); setShowManualArchive(false); setView(null); setPlan(null); setConfirm(null);
  }
  function selectedRelease(id: string) { requestEpoch.current++; setRelease(id); setArchive(""); setShowManualArchive(false); setView(null); setPlan(null); setConfirm(null); }
  function source(url: string) {
    if (!url.startsWith("https://")) { setError("来源地址不是 HTTPS，已阻止打开。"); return; }
    Navigation.NavigateToExternalWeb(url);
  }
  function viewChanges() {
    if (!selected || !release) return;
    const epoch = ++requestEpoch.current;
    void work("正在查看将要修改的文件…", () => preview(selected, release), value => {
      if (epoch === requestEpoch.current && value.instance_id === selected && value.release_id === release) setView(value);
    });
  }
  function getPlan() {
    if (!selected || !release || !patch || installBlocker(patch)) return;
    if (!archive.trim()) { setError("请先填写 Downloads 内的 ZIP 路径，或使用上方自动准备功能。"); return; }
    const epoch = ++requestEpoch.current;
    setPlan(null); setConfirm(null);
    void work("正在校验目录和文件…", () => planInstall(selected, release, archive), value => {
      if (epoch !== requestEpoch.current || value.instance_id !== selected || value.release_id !== release) return;
      setPlan(value); setConfirm(null);
    });
  }
  function prepareSelectedInstall() {
    if (!selected || !release || !patch || installBlocker(patch)) return;
    const epoch = ++requestEpoch.current;
    setPlan(null); setConfirm(null);
    const label = patch.download_available ? "正在查找本地 ZIP、下载或校验文件…" : "正在查找本地 ZIP 并校验文件…";
    void work(label, () => prepareInstall(selected, release), result => {
      if (epoch !== requestEpoch.current || result.plan.instance_id !== selected || result.plan.release_id !== release) return;
      setArchive(result.archive); setPlan(result.plan); setConfirm(null); setNow(Date.now());
      setNote(preparedSource(result.acquisition, result.mode));
    });
  }
  function installSelectedPlan() {
    if (!activePlan?.plan_id || confirm !== activePlan.plan_id || !patch || installBlocker(patch)) return;
    if (planExpired(activePlan)) { setConfirm(null); setError("安装计划已过期，请重新生成计划。"); return; }
    void work("正在安装，请保持游戏关闭…", () => startInstall(activePlan.plan_id!, true), async tx => {
      setPlan(null); setConfirm(null);
      const result = transactionResult(tx);
      try {
        const { next, tx: all } = await readSnapshot();
        setReport(next); setTransactions(all.some(item => item.transaction_id === tx.transaction_id) ? all : [tx, ...all]);
      } catch { setTransactions(previous => [tx, ...previous.filter(item => item.transaction_id !== tx.transaction_id)]); }
      setNote(result);
    });
  }

  return <div className="steam-patch-panel"><style>{panelStyles}</style>
    <PanelSection title="Steam 补丁管理">{busy?.includes("下载") && <PanelSectionRow><ActionButton onClick={() => { requestEpoch.current++; void cancelDownload(); }}>取消下载</ActionButton></PanelSectionRow>}<PanelSectionRow><ActionButton onClick={refresh} disabled={!!busy}>重新扫描</ActionButton></PanelSectionRow><PanelSectionRow><ActionButton onClick={() => void work("正在恢复…", () => recover(true), () => { setNote("恢复检查完成。"); refresh(); })} disabled={!!busy}>已关闭游戏并暂停更新，修复未完成的操作</ActionButton></PanelSectionRow>
      {busy && <PanelSectionRow><p>{busy} 已经过 {Math.floor(Math.max(0, now - (busySince ?? now)) / 1000)} 秒。完成后显示结果；暂无百分比进度。{busySince && now - busySince >= 30000 && (busy.includes("校验") || busy.includes("安装")) ? " 大型 ZIP 的完整校验或安装可能需要数分钟，请继续等待。" : ""}</p></PanelSectionRow>}
      {error && <PanelSectionRow><p style={{ color: "#ffb28d" }}>{error}</p></PanelSectionRow>}
      {note && <PanelSectionRow><p>{note}</p></PanelSectionRow>}
      <PanelSectionRow><p>补丁信息版本 {report?.catalog_revision ?? "—"} · {workflowState(report?.catalog_trust ?? "未加载")}</p></PanelSectionRow>
      <PanelSectionRow><ActionButton onClick={() => setShowSources(!showSources)} disabled={!!busy}>{showSources ? "收起补丁源" : "管理订阅的补丁源"}</ActionButton></PanelSectionRow>
    </PanelSection>
    {showSources && <PanelSection title="补丁源订阅">
      <PanelSectionRow><p>重新扫描会查询已启用的网站，按各自规则解析和匹配游戏。查询失败时标明缓存状态。</p></PanelSectionRow>
      {report?.sources?.map(entry => <Fragment key={entry.id}>
        <PanelSectionRow><p>{entry.name} · {entry.enabled ? "已订阅" : "已停用"}<br />{entry.pages.join(" · ")}<br />{entry.status ? reason(entry.status) : "待查询"}{entry.entry_count !== undefined ? ` · ${entry.entry_count} 条` : ""}{entry.cached_at ? <><br />缓存取得于 {new Date(entry.cached_at * 1000).toLocaleString()}</> : null}</p></PanelSectionRow>
        <PanelSectionRow><ActionButton onClick={() => changeSources(() => setSourceEnabled(entry.id, !entry.enabled))} disabled={!!busy}>{entry.enabled ? "停用" : "订阅"} {entry.name}</ActionButton></PanelSectionRow>
      </Fragment>)}
      <PanelSectionRow><TextField label="添加源规则（JSON）" value={sourceRule} onChange={e => setSourceRule(e.target.value)} /></PanelSectionRow>
      <PanelSectionRow><p>支持网站专用适配器及可配置的表格解析。规则需指定网站、下载域名和各列含义。</p></PanelSectionRow>
      <PanelSectionRow><ActionButton onClick={() => changeSources(() => addSource(sourceRule))} disabled={!!busy || !sourceRule.trim()}>添加并查询补丁源</ActionButton></PanelSectionRow>
    </PanelSection>}
    {!selected && <><PanelSection title="已安装游戏"><PanelSectionRow><TextField label="搜索游戏名称或编号" value={query} onChange={e => setListPreferences({ query: e.target.value })} /></PanelSectionRow>
      <PanelSectionRow><DropdownItem label="筛选游戏" layout="below" bottomSeparator="none" rgOptions={filterOptions} selectedOption={filter} onChange={option => {
        if (filterOptions.some(entry => entry.data === option.data)) setListPreferences({ filter: option.data });
      }} /></PanelSectionRow>
      <PanelSectionRow><p>{shown.length} 款游戏</p></PanelSectionRow>
      {shown.map(item => <PanelSectionRow key={item.game.instance_id}><GameButton item={item} onClick={() => choose(item)} disabled={!!busy} /></PanelSectionRow>)}
      {report && report.installs.length > 0 && !shown.length && <PanelSectionRow><p>没有符合条件的游戏，请更换筛选条件或搜索名称。</p></PanelSectionRow>}
      {report && !report.installs.length && <PanelSectionRow><p>未找到 Steam 游戏。请确认库磁盘或 SD 卡已挂载。</p></PanelSectionRow>}
      {report?.diagnostics.map((d, i) => <PanelSectionRow key={i}><p>{reason(d.code)}{d.path ? ` · ${d.path}` : ""}</p></PanelSectionRow>)}
    </PanelSection></>}
    {row && <><PanelSection title={gamePresentation(row.game.appid, row.game.name).name}><PanelSectionRow><ActionButton onClick={() => { requestEpoch.current++; setSelected(null); setRelease(null); setPlan(null); setConfirm(null); }} disabled={!!busy}>← 返回游戏列表</ActionButton></PanelSectionRow>
      <PanelSectionRow><p>游戏编号 {row.game.appid} · 游戏版本 {row.game.buildid ?? "未知"}<br />安装位置：{row.game.install_path}</p></PanelSectionRow>
      {!row.patches.length && <PanelSectionRow><p>暂未找到补丁信息，无法判断此游戏是否需要或已经安装补丁。</p></PanelSectionRow>}
      {row.patches.map(item => <PanelSectionRow key={item.release_id}><ActionButton onClick={() => selectedRelease(item.release_id)} disabled={!!busy}>{item.name} {release === item.release_id ? "✓" : ""}</ActionButton></PanelSectionRow>)}
    </PanelSection>
    {patch && <><PanelSection title="补丁详情"><PanelSectionRow><p>{patch.name}<br />状态：{patchState(patch.detection.status)}<br />兼容性：{workflowState(patch.compatibility)}<br />安装方式：{workflowState(patch.support_level)}</p></PanelSectionRow>
      {patch.blockers.map((b, i) => <PanelSectionRow key={i}><p style={{ color: "#ffb28d" }}>{reason(b)}</p></PanelSectionRow>)}
      {isExternal(patch.detection.status) && <PanelSectionRow><p>这个补丁通过其他方式安装，本工具没有原文件备份，无法替你卸载。</p></PanelSectionRow>}
      <PanelSectionRow><ActionButton onClick={() => source(patch.source_page)}>在外部浏览器打开来源 ↗</ActionButton></PanelSectionRow>
    </PanelSection><PanelSection title="安装与预览"><PanelSectionRow><ActionButton onClick={viewChanges} disabled={!!busy || patch.support_level === "publisher_zip"}>查看文件变更</ActionButton></PanelSectionRow>
      {blocker && <PanelSectionRow><p style={{ color: "#ffb28d" }}>无法获取或安装：{blocker}</p></PanelSectionRow>}
      <PanelSectionRow><p>{patch.support_level === "publisher_zip" ? "从官方获取 ZIP，默认保持包内相对路径解压至游戏根目录。先下载并检查文件，再显示安装计划；覆盖文件会备份。此方式尚未逐游戏验证效果，不会执行包内安装程序。" : patch.download_available ? "优先复用本地已审核 ZIP；未找到时自动下载并校验。" : "此来源不能自动下载。将查找 Downloads 中已审核的 ZIP；若未找到，请从来源页下载后使用手动备用方式。"}</p></PanelSectionRow>
      <PanelSectionRow><ActionButton onClick={prepareSelectedInstall} disabled={!!busy || !!blocker}>{patch.download_available ? "自动下载并准备安装" : "查找本地 ZIP 并准备安装"}</ActionButton></PanelSectionRow>
      {patch.support_level === "managed_zip" && <PanelSectionRow><ActionButton onClick={() => setShowManualArchive(value => !value)} disabled={!!busy}>手动指定 ZIP（高级）{showManualArchive ? " ▴" : " ▾"}</ActionButton></PanelSectionRow>}
      {showManualArchive && patch.support_level === "managed_zip" && <><PanelSectionRow><TextField label="本地 ZIP 路径（仅 ~/Downloads 内）" value={archive} disabled={!!busy} onChange={e => { requestEpoch.current++; setArchive(e.target.value); setPlan(null); setConfirm(null); }} /></PanelSectionRow>
        <PanelSectionRow><p>浏览器下载后切换到桌面模式确认文件路径，再回到这里输入。导入仅允许 Downloads 内的 ZIP。</p></PanelSectionRow>
        <PanelSectionRow><ActionButton onClick={getPlan} disabled={!!busy || !!blocker}>使用指定 ZIP 验证并生成安装计划</ActionButton></PanelSectionRow></>}
      {view && <Changes title="文件变更预览" value={view} />}{activePlan && <Changes title="安装计划" value={activePlan} />}
      {activePlan && <PanelSectionRow><p>{expired ? "安装计划已过期，请重新生成计划。" : `安装计划剩余 ${Math.ceil((activePlan.expires_at! * 1000 - now) / 1000)} 秒。`}</p></PanelSectionRow>}
      {activePlan?.plan_id && activePlan.blockers.length === 0 && <><PanelSectionRow><ActionButton onClick={() => { if (planExpired(activePlan)) { setConfirm(null); setError("安装计划已过期，请重新生成计划。"); } else setConfirm(activePlan.plan_id!); }} disabled={!!busy || expired}>查看确认</ActionButton></PanelSectionRow>
        {confirm === activePlan.plan_id && !expired && <PanelSectionRow><ActionButton onClick={installSelectedPlan} disabled={!!busy || expired}>已关闭游戏及更新，确认安装 {activePlan.changes.length} 项变更</ActionButton></PanelSectionRow>}</>}
    </PanelSection></>}
    <PanelSection title="补丁操作记录">{owned.length ? owned.map(tx => <Fragment key={tx.transaction_id}>
      <PanelSectionRow><p>{tx.release_id} · {workflowState(tx.state)}{tx.error && ` · ${reason(tx.error)}`}</p></PanelSectionRow>
      {tx.state === "committed" && <PanelSectionRow><ActionButton disabled={!!busy} onClick={() => setConfirm(tx.transaction_id)}>卸载这个补丁</ActionButton></PanelSectionRow>}
      {confirm === tx.transaction_id && <PanelSectionRow><ActionButton disabled={!!busy} onClick={() => void work("正在安全卸载…", () => uninstall(tx.transaction_id, true), () => { setConfirm(null); refresh(); })}>已关闭游戏及更新，确认恢复文件</ActionButton></PanelSectionRow>}
    </Fragment>) : <PanelSectionRow><p>没有通过本工具安装的补丁。其他方式安装的补丁需要自行卸载。</p></PanelSectionRow>}</PanelSection>
    </>}
  </div>;
}

function Changes({ title, value }: { title: string; value: Preview | Plan }) {
  return <><PanelSectionRow><p>{title} · {value.changes.length} 个文件需要修改</p></PanelSectionRow>{value.blockers.map((b, i) => <PanelSectionRow key={i}><p style={{ color: "#ffb28d" }}>{reason(b)}</p></PanelSectionRow>)}{value.changes.map((c, i) => <PanelSectionRow key={i}><p>{workflowState(c.action)} · {c.path}{c.baseline_matches === false ? " · 原始文件已变化" : ""}</p></PanelSectionRow>)}{"executable" in value && value.executable === false && <PanelSectionRow><p>目前只能查看这些改动，暂不能安装。</p></PanelSectionRow>}</>;
}

export default definePlugin(() => ({ name: "Steam 补丁管理", titleView: <div>Steam 补丁管理</div>, content: <Content />, icon: <FaPuzzlePiece /> }));
