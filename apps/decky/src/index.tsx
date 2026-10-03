import { callable, definePlugin, routerHook } from "@decky/api";
import { Button, ButtonItem, ConfirmModal, DropdownItem, Field, Focusable, ModalRoot, Navigation, PanelSection, PanelSectionRow, ProgressBar, SidebarNavigation, TextField, ToggleField, showModal, useParams } from "@decky/ui";
import { FaGamepad, FaPuzzlePiece } from "react-icons/fa";
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ComponentProps, type ReactNode } from "react";
import { filterInstalls, isExternal, isManaged, needsAttention, patchState, reason, workflowState, type Filter, type Install, type Patch, type Plan, type ScanResult, type SourceSubscription, type Transaction } from "@steam-patch/ui";
import { getListPreferences, setListPreferences, subscribeListPreferences } from "./preferences";
import { gamePresentation } from "./steam";
import { installBlocker, planExpired, preparedSource, transactionResult, type PreparedInstall } from "./install";
import { createManagerSession, needsRecovery } from "./session";
import { decodeInstance, encodeInstance } from "./route";
import { tasks, taskIsTerminal, type TaskSnapshot } from "./tasks";

const scan = callable<[], ScanResult>("scan");
const listTransactions = callable<[], Transaction[] | { transactions: Transaction[] }>("list_transactions");
const uninstall = callable<[string, boolean], Transaction>("uninstall");
const recover = callable<[boolean], Transaction[]>("recover");
const setSourceEnabled = callable<[string, boolean], SourceSubscription[]>("set_source_enabled");
const addSource = callable<[string], SourceSubscription[]>("add_source");
const discardPlan = callable<[string], unknown>("discard_plan");

async function readSnapshot() {
  const report = await scan();
  let transactions = session.getSnapshot().transactions;
  try { const result = await listTransactions(); transactions = Array.isArray(result) ? result : result.transactions ?? []; }
  catch { /* Keep the last journal when an older core cannot list it. */ }
  return { report, transactions };
}
const session = createManagerSession(readSnapshot);
const route = "/steam-patch-manager/:instance/:panel?";
const sourceRoute = "sources";
const panelKeys = ["overview", "patches", "records", "sources"] as const;
const filterOptions: { data: Filter; label: string }[] = [
  { data: "all", label: "全部" }, { data: "matched", label: "有补丁" },
  { data: "installed", label: "已安装补丁" }, { data: "attention", label: "需要处理" },
];
let listScroll = 0;
let lastGame: string | null = null;

function openManager(instance: string) {
  Navigation.Navigate(`/steam-patch-manager/${encodeInstance(instance)}`);
  Navigation.CloseSideMenus();
}
function openSources() {
  Navigation.Navigate(`/steam-patch-manager/${sourceRoute}`);
  Navigation.CloseSideMenus();
}
function Action({ children, ...props }: ComponentProps<typeof ButtonItem>) {
  return <ButtonItem {...props} layout="below" bottomSeparator="none">{children}</ButtonItem>;
}
function Status({ children, warning = false }: { children: ReactNode; warning?: boolean }) {
  return <p className={warning ? "patch-warning" : undefined}>{children}</p>;
}
function Row({ children }: { children: ReactNode }) { return <PanelSectionRow>{children}</PanelSectionRow>; }
// Native left-label / right-value row. Long values opt into a stacked layout.
function InfoRow({ label, children, below = false }: { label: ReactNode; children: ReactNode; below?: boolean }) {
  return <PanelSectionRow><Field label={label} childrenLayout={below ? "below" : "inline"} spacingBetweenLabelAndChild="none"><span className="patch-kv-value">{children}</span></Field></PanelSectionRow>;
}
function sourceInfo(patch: Patch, sources: SourceSubscription[] = []) {
  const subscription = sources.find(source => source.id === patch.source_id);
  const name = patch.source_name || subscription?.name || "匹配源未知";
  const state = patch.source_status || subscription?.status;
  const cached = patch.source_cached_at ?? subscription?.cached_at;
  return `${name} · ${state ? reason(state) : "查询状态未知"}${cached && ["cached", "source_offline_cached"].includes(state ?? "") ? ` · 缓存于 ${new Date(cached * 1000).toLocaleString()}` : ""}`;
}
function openSource(url: string) {
  if (!url.startsWith("https://")) { session.update({ error: "来源地址不是 HTTPS，已阻止打开。" }); return; }
  Navigation.NavigateToExternalWeb(url);
}
function record(tx: Transaction) {
  const old = session.getSnapshot().transactions;
  session.update({ transactions: [tx, ...old.filter(item => item.transaction_id !== tx.transaction_id)] });
}
function refreshAfterMutation(note?: string, error?: string) {
  void session.refresh().then(() => { if (note || error) session.update({ ...(note ? { note } : {}), ...(error ? { error } : {}) }); });
}
function confirmDialog(title: string, description: ReactNode, ok: string, run: () => void) {
  const modal = showModal(<ConfirmModal strTitle={title} strDescription={description}
    strOKButtonText={ok} strCancelButtonText="取消" bDestructiveWarning
    onOK={() => { modal.Close(); run(); }} onCancel={() => modal.Close()} closeModal={() => modal.Close()} />);
}

const css = `
  .patch-qam { display:flex; flex-direction:column; min-height:0; height:100%; }
  .patch-qam-list { min-height:0; flex:1; overflow-y:auto; }
  .patch-qam-controls { flex:none; }
  .patch-qam .patch-game button { min-width:0; width:100%; }
  .patch-game-row { display:flex; align-items:center; gap:10px; width:100%; height:50px; min-width:0; text-align:left; }
  .patch-game-icon { width:36px; height:36px; flex:none; object-fit:cover; border-radius:4px; }
  .patch-game-fallback { display:flex; align-items:center; justify-content:center; background:#263647; }
  .patch-game-text { min-width:0; flex:1; overflow:hidden; }
  .patch-game-name { display:block; overflow:hidden; white-space:nowrap; text-overflow:ellipsis; font-weight:600; }
  .patch-game:hover .patch-game-name[data-overflow="true"],
  .patch-game:focus-within .patch-game-name[data-overflow="true"],
  .patch-game .gpfocus .patch-game-name[data-overflow="true"] { text-overflow:clip; }
  .patch-game:hover .patch-game-name[data-overflow="true"] span,
  .patch-game:focus-within .patch-game-name[data-overflow="true"] span,
  .patch-game .gpfocus .patch-game-name[data-overflow="true"] span { display:inline-block; animation:patch-marquee var(--duration) linear .8s infinite alternate; }
  @keyframes patch-marquee { 0%,12% {transform:translateX(0)} 88%,100% {transform:translateX(calc(-1 * var(--travel)))} }
  .patch-game-status { display:block; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; font-size:12px; opacity:.76; }
  .patch-full { height:100%; min-height:0; }
  .patch-page { box-sizing:border-box; height:100%; overflow-y:auto; padding:0 24px 48px; }
  .patch-page h2 { margin:14px 0 6px; font-size:20px; overflow-wrap:anywhere; }
  .patch-page p { line-height:1.4; overflow-wrap:anywhere; }
  .patch-page .patch-warning { color:#ffb28d; }
  .patch-page button { max-width:100%; }
  .patch-nav-header { display:flex; flex-direction:column; min-width:0; }
  .patch-nav-tool { font-size:13px; font-weight:600; opacity:.85; }
  .patch-nav-game { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:12px; opacity:.7; }
  .patch-actions { display:flex; gap:10px; margin-top:8px; }
  .patch-actions > * { flex:1 1 0; min-width:0; }
  .patch-patch-name { font-weight:600; }
  .patch-patch-source { font-size:12px; opacity:.8; overflow-wrap:anywhere; }
  .patch-kv-value { overflow-wrap:anywhere; }
  .patch-modal { width:min(640px,90vw); }
  .patch-modal p { line-height:1.4; overflow-wrap:anywhere; }
  .patch-metrics { display:grid; grid-template-columns:1fr 1fr; gap:4px 20px; margin:12px 0; }
`;

function matchSummary(item: Install): string {
  if (needsAttention(item)) return "需要处理";
  if (item.patches.some(p => isManaged(p.detection.status) || isExternal(p.detection.status))) return "已安装补丁";
  return item.patches.length ? `可安装 · ${item.patches.length} 个补丁` : "暂无匹配补丁";
}
function GameButton({ item }: { item: Install }) {
  const { name, icon } = gamePresentation(item.game.appid, item.game.name);
  const [badIcon, setBadIcon] = useState(false);
  const nameRef = useRef<HTMLSpanElement>(null);
  const [travel, setTravel] = useState(0);
  useLayoutEffect(() => {
    const element = nameRef.current;
    if (!element) return;
    const measure = () => setTravel(Math.max(0, element.scrollWidth - element.clientWidth));
    measure(); const observer = new ResizeObserver(measure); observer.observe(element); return () => observer.disconnect();
  }, [name]);
  const state = matchSummary(item);
  return <div className="patch-game" data-instance={item.game.instance_id}><Action onClick={() => { lastGame = item.game.instance_id; openManager(item.game.instance_id); }}>
    <span className="patch-game-row" aria-label={`${name}，${state}`}>
      {icon && !badIcon ? <img className="patch-game-icon" src={icon} alt="" onError={() => setBadIcon(true)} />
        : <span className="patch-game-icon patch-game-fallback"><FaGamepad aria-hidden="true" /></span>}
      <span className="patch-game-text"><span className="patch-game-name" data-overflow={travel > 0} ref={nameRef}
        style={{ "--travel": `${travel}px`, "--duration": `${Math.max(4, travel / 30 + 2)}s` } as React.CSSProperties}><span aria-hidden="true">{name}</span></span>
        <span className="patch-game-status">{state}</span></span>
    </span></Action></div>;
}

function QuickAccess() {
  const { report, busy, error, note } = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { filter, query } = useSyncExternalStore(subscribeListPreferences, getListPreferences);
  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => { void session.ensureSnapshot(); void tasks.resume().then(() => tasks.requestCancel()).catch(() => {}); }, []);
  useLayoutEffect(() => { if (listRef.current) listRef.current.scrollTop = listScroll; }, [report, filter, query]);
  const shown = useMemo(() => filterInstalls(report?.installs ?? [], filter, "").filter(item => {
    const q = query.trim().toLocaleLowerCase();
    return !q || [item.game.name, gamePresentation(item.game.appid, item.game.name).name, String(item.game.appid)]
      .some(value => value.toLocaleLowerCase().includes(q));
  }), [report, filter, query]);
  useLayoutEffect(() => {
    if (!lastGame || !listRef.current) return;
    const row = Array.from(listRef.current.querySelectorAll<HTMLElement>("[data-instance]")).find(node => node.dataset.instance === lastGame);
    row?.querySelector("button")?.focus();
    if (row) lastGame = null;
  }, [shown]);
  return <div className="patch-qam"><style>{css}</style>
    <div className="patch-qam-controls"><PanelSection title="Steam 补丁管理">
      <Row><TextField label="搜索游戏名称或编号" value={query} onChange={e => setListPreferences({ query: e.target.value })} /></Row>
      <Row><DropdownItem label="筛选" layout="below" rgOptions={filterOptions} selectedOption={filter} onChange={option => setListPreferences({ filter: option.data })} /></Row>
      {busy && <Row><Status>{busy}</Status></Row>}{error && <Row><Status warning>{error}</Status></Row>}
      {note && <Row><Status>{note}</Status></Row>}
    </PanelSection></div>
    <div className="patch-qam-list" ref={listRef} onScroll={e => { listScroll = e.currentTarget.scrollTop; }}>
      <PanelSection title={`游戏列表 · ${shown.length}`}>
        {shown.map(item => <Row key={item.game.instance_id}><GameButton item={item} /></Row>)}
        {report && report.installs.length > 0 && !shown.length && <Row><Status>没有符合条件的游戏。</Status><Action onClick={() => setListPreferences({ filter: "all", query: "" })}>清除筛选</Action></Row>}
        {report && !report.installs.length && <Row><Status>未找到 Steam 游戏。请检查游戏库或 SD 卡。</Status></Row>}
        {report?.diagnostics.map((d, i) => <Row key={i}><Status warning>{reason(d.code)}{d.path ? ` · ${d.path}` : ""}</Status></Row>)}
      </PanelSection>
    </div>
    <div className="patch-qam-controls"><PanelSection>
      <Row><Action onClick={openSources}>管理订阅源 ›</Action></Row>
      <Row><Action onClick={() => void session.refresh()} disabled={!!busy}>重新扫描</Action></Row>
    </PanelSection></div>
  </div>;
}

const stageNames: Record<string, string> = {
  scanning: "查找本地 ZIP", searching: "查找本地 ZIP", connecting: "正在连接", downloading: "下载补丁",
  validating: "校验文件", planning: "生成安装计划", backing_up: "备份原文件",
  writing: "写入文件", committing: "提交安装", rolling_back: "恢复原文件",
};
function size(bytes: number) {
  if (!Number.isFinite(bytes)) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let n = Math.max(0, bytes), i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${i ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
}
function duration(seconds: number) { return `${Math.floor(seconds / 60)} 分 ${Math.floor(seconds % 60)} 秒`; }
function TaskModal({ game, patch, onTerminal }: { game: string; patch: string; onTerminal: (task: TaskSnapshot) => void }) {
  const task = useSyncExternalStore(tasks.subscribe, tasks.getSnapshot);
  const handled = useRef<string | null>(null);
  useEffect(() => () => { void tasks.requestCancel(); }, []);
  useEffect(() => {
    if (task && taskIsTerminal(task.state) && handled.current !== `${task.task_id}:${task.state}`) {
      handled.current = `${task.task_id}:${task.state}`; onTerminal(task);
    }
  }, [task, onTerminal]);
  const cancelOrClose = () => {
    if (task && !taskIsTerminal(task.state)) { void tasks.requestCancel(); return; }
    modal?.Close(); tasks.clear();
  };
  const busy = task && !taskIsTerminal(task.state);
  const stage = task?.state === "cancelling" ? "正在取消" : task?.state === "rolling_back" ? "正在恢复原文件" : stageNames[task?.stage ?? ""] ?? task?.stage ?? "正在启动";
  const measurable = task?.total_bytes != null && task.total_bytes > 0;
  const progress = measurable ? Math.min(100, task!.bytes_done / task!.total_bytes! * 100) : 0;
  return <ModalRoot className="patch-modal" bDisableBackgroundDismiss bHideCloseIcon onCancel={cancelOrClose} closeModal={cancelOrClose}>
    <h2>{task?.kind === "install" ? "安装补丁" : "准备补丁"}</h2><p>{game} · {patch}</p>
    <p>当前阶段：{stage}{measurable ? ` · ${Math.round(progress)}%` : ""}</p>
    <ProgressBar indeterminate={!measurable} nProgress={progress / 100} />
    <div className="patch-metrics">
      <span>已处理：{size(task?.bytes_done ?? 0)}{measurable ? ` / ${size(task!.total_bytes!)}` : " / 总量未知"}</span>
      <span>文件：{task?.files_done ?? 0}{task?.total_files != null ? ` / ${task.total_files}` : ""}</span>
      {task?.kind === "prepare" && task.stage === "downloading" && <span>下载速度：{size(task.speed_bps)}/s</span>}
      {task?.kind === "prepare" && task.archive_bytes != null && task.stage !== "downloading" && <span>ZIP 大小：{size(task.archive_bytes)}</span>}
      {task?.kind === "install" && task.archive_bytes != null && <span>安装包大小：{size(task.archive_bytes)}</span>}
      <span>已用：{duration(task?.elapsed_seconds ?? 0)}</span>
      {task?.kind === "prepare" && task.stage === "downloading" && <span>预计剩余：{task.eta_seconds == null ? "估算中" : duration(task.eta_seconds)}</span>}
    </div>
    {task?.current_file && <p>当前文件：{task.current_file}</p>}
    {task?.error && <Status warning>{reason(task.error)}</Status>}
    {task?.state === "cancelled" && <p>已安全取消。</p>}
    {task?.state === "completed" && <p>{task.kind === "install" ? "安装已完成。" : "准备完成。"}</p>}
    {busy && <p>退出此界面将中断任务。安装取消时会恢复本次修改的文件。</p>}
    <Action onClick={cancelOrClose} disabled={task?.state === "cancelling" || task?.state === "rolling_back"}>
      {busy ? task?.state === "cancelling" || task?.state === "rolling_back" ? stage : "取消并返回" : "返回补丁管理"}
    </Action>
  </ModalRoot>;
}
let modal: ReturnType<typeof showModal> | null = null;
function showTask(game: string, patch: string, onTerminal: (task: TaskSnapshot) => void) {
  modal?.Close();
  modal = showModal(<TaskModal game={game} patch={patch} onTerminal={value => {
    onTerminal(value);
    if ((value.kind === "prepare" && value.state === "completed") || value.state === "cancelled") { modal?.Close(); modal = null; tasks.clear(); }
  }} />, undefined, { strTitle: "补丁进度", bHideMainWindowForPopouts: false, fnOnClose: () => { void tasks.requestCancel(); } });
}

function GameManager({ item, seg, panel }: { item: Install; seg: string; panel?: string }) {
  const { report, transactions, busy, error, note } = useSyncExternalStore(session.subscribe, session.getSnapshot);
  // The decky SidebarNavigation is route-driven: it renders the page whose
  // `route` matches the URL and switches by replacing the URL. Keep one
  // sub-route per panel so selection and content follow the URL.
  const activePanel = panel && (panelKeys as readonly string[]).includes(panel) ? panel : "overview";
  const base = `/steam-patch-manager/${seg}`;
  const navigate = useRef<((route: string) => void) | null>(null);
  const [release, setRelease] = useState(item.patches[0]?.release_id ?? "");
  const [plan, setPlan] = useState<Plan | null>(null);
  const [archive, setArchive] = useState("");
  const [acquisition, setAcquisition] = useState<PreparedInstall["acquisition"] | null>(null);
  const [manual, setManual] = useState(false);
  const [changesOpen, setChangesOpen] = useState(false);
  const [rule, setRule] = useState("");
  const [now, setNow] = useState(Date.now());
  const panelScroll = useRef<Record<string, number>>({});
  const contentRef = useRef<HTMLDivElement>(null);
  const planRef = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);
  const game = gamePresentation(item.game.appid, item.game.name).name;
  const patch = item.patches.find(p => p.release_id === release);
  const active = plan?.instance_id === item.game.instance_id && plan.release_id === release ? plan : null;
  const blocked = patch ? installBlocker(patch) : null;
  const owned = transactions.filter(t => t.instance_id === item.game.instance_id).reverse();
  const committed = owned.filter(t => t.release_id === release && t.state === "committed");
  const installedManaged = patch ? isManaged(patch.detection.status) : false;
  const installedExternal = patch ? isExternal(patch.detection.status) : false;
  const installed = !!patch && (installedManaged || installedExternal || committed.length > 0);
  const unfinished = transactions.filter(needsRecovery);
  useEffect(() => {
    mounted.current = true;
    void session.ensureSnapshot();
    void tasks.resume().then(() => {
      const task = tasks.getSnapshot();
      if (!mounted.current || !task || task.instance_id !== item.game.instance_id) return;
      if (taskIsTerminal(task.state)) { finishTask(task); tasks.clear(); }
      else {
        const name = item.patches.find(p => p.release_id === task.release_id)?.name ?? task.release_id ?? "补丁";
        showTask(game, name, finishTask);
      }
    }).catch(() => {});
    return () => { mounted.current = false; void tasks.requestCancel(); };
  }, []);
  useEffect(() => { if (!active) return; const id = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(id); }, [active]);
  useLayoutEffect(() => { if (contentRef.current) contentRef.current.scrollTop = panelScroll.current[activePanel] ?? 0; }, [activePanel]);
  useLayoutEffect(() => {
    if (activePanel === "patches" && active && planRef.current) {
      planRef.current.scrollIntoView({ block: "start" });
      planRef.current.focus();
    }
  }, [active?.plan_id, activePanel]);
  function navigatePanel(next: string) {
    if (next === activePanel) return;
    const target = `${base}/${next}`;
    // The sidebar exposes its own switcher, which replaces the URL instead of
    // growing browser history; fall back to a normal navigation before it
    // registers on the first render.
    if (navigate.current) navigate.current(target); else Navigation.Navigate(target);
  }
  function chooseRelease(next: string) { setRelease(next); setPlan(null); setArchive(""); setAcquisition(null); setChangesOpen(false); }
  function changeSources(action: () => Promise<SourceSubscription[]>) {
    setPlan(null);
    void session.work("正在更新补丁源并查询…", async () => { await action(); return readSnapshot(); }, snapshot => {
      session.update(snapshot); setRule("");
    });
  }
  function finishTask(task: TaskSnapshot) {
    if (task.state === "completed" && task.kind === "prepare") {
      const value = task.result as PreparedInstall;
      if (value?.plan.instance_id === item.game.instance_id && value.plan.release_id === task.release_id) {
        if (task.release_id) setRelease(task.release_id);
        setPlan(value.plan); setArchive(value.archive); setAcquisition(value.acquisition); setChangesOpen(false);
        session.update({ note: preparedSource(value.acquisition, value.mode) });
        navigatePanel("patches"); panelScroll.current.patches = 0;
      }
    } else if (task.state === "completed" && task.kind === "install") {
      const tx = task.result as Transaction;
      if (tx?.transaction_id) record(tx);
      setPlan(null); refreshAfterMutation(tx ? transactionResult(tx) : "安装完成，请检查操作记录。");
    } else if (task.state === "failed") {
      const error = reason(task.error ?? "任务失败，请检查操作记录。");
      session.update({ error });
      refreshAfterMutation(undefined, error);
    } else if (task.state === "cancelled") {
      session.update({ note: "任务已安全取消。" }); refreshAfterMutation();
    }
  }
  async function beginPrepare(manualArchive = "") {
    if (!patch || blocked || busy) return;
    try { setPlan(null); setAcquisition(null); const id = await tasks.startPrepare(item.game.instance_id, release, manualArchive);
      if (mounted.current && tasks.getSnapshot()?.task_id === id) showTask(game, patch.name, finishTask); }
    catch (e) { session.update({ error: reason(String(e)) }); }
  }
  async function beginInstall() {
    if (!active?.plan_id || planExpired(active) || blocked || busy || !patch) return;
    try { const id = await tasks.startInstall(active.plan_id, item.game.instance_id, release);
      if (mounted.current && tasks.getSnapshot()?.task_id === id) showTask(game, patch.name, finishTask); }
    catch (e) { session.update({ error: reason(String(e)) }); }
  }
  function cancelPreparedPlan() {
    if (!active?.plan_id) { setPlan(null); return; }
    void session.work("正在取消安装计划…", () => discardPlan(active.plan_id!), () => {
      setPlan(null); setAcquisition(null); setArchive("");
      session.update({ note: "本次安装计划已取消。" });
    });
  }
  const common = <>{busy && <Status>{busy}</Status>}{error && <Status warning>{error}</Status>}{note && <Status>{note}</Status>}</>;
  const overview = <div className="patch-page" ref={activePanel === "overview" ? contentRef : undefined} onScroll={e => panelScroll.current.overview = e.currentTarget.scrollTop}>
    {common}
    <PanelSection>
      <InfoRow label="补丁状态">{matchSummary(item)}</InfoRow>
      <InfoRow label="匹配来源">{item.patches.length ? [...new Set(item.patches.map(p => p.source_name || report?.sources?.find(s => s.id === p.source_id)?.name || "匹配源未知"))].join("、") : "—"}</InfoRow>
      {owned.some(needsRecovery) && <Row><Status warning>有未完成的操作需要处理。</Status><Action onClick={() => navigatePanel("records")}>查看操作记录</Action></Row>}
    </PanelSection>
    <PanelSection title="匹配到的补丁">{item.patches.length ? item.patches.map(p => <PanelSectionRow key={p.release_id}>
      <div className="patch-patch">
        <div className="patch-patch-name">{p.name}</div>
        <div className="patch-patch-source">匹配源：{sourceInfo(p, report?.sources)}</div>
        <div className="patch-actions">
          <Button onClick={() => { chooseRelease(p.release_id); navigatePanel("patches"); }}>查看补丁</Button>
          <Button onClick={() => openSource(p.source_page)}>打开来源网页 ↗</Button>
        </div>
      </div>
    </PanelSectionRow>) : <Row><Status>暂无匹配补丁；这不代表游戏不需要补丁。</Status><Action onClick={() => navigatePanel("sources")}>管理订阅源</Action></Row>}</PanelSection>
    <PanelSection title="游戏信息">
      <InfoRow label="游戏编号">{item.game.appid}</InfoRow>
      <InfoRow label="游戏版本">{item.game.buildid ?? "未知"}</InfoRow>
      <InfoRow label="安装位置" below>{item.game.install_path}</InfoRow>
    </PanelSection>
  </div>;
  const patches = <div className="patch-page" ref={activePanel === "patches" ? contentRef : undefined} onScroll={e => panelScroll.current.patches = e.currentTarget.scrollTop}>
    {common}
    {!patch ? <PanelSection><Row><Status>暂无匹配补丁。</Status></Row></PanelSection> : <>
      <PanelSection title="选择补丁">{item.patches.length > 1 ? <Row><DropdownItem label="补丁" rgOptions={item.patches.map(p => ({ data: p.release_id, label: p.name }))} selectedOption={release} onChange={option => chooseRelease(option.data)} /></Row>
        : <InfoRow label="补丁">{patch.name}</InfoRow>}
        <InfoRow label="匹配源">{sourceInfo(patch, report?.sources)}</InfoRow>
      </PanelSection>
      <PanelSection title="补丁详情">
        <InfoRow label="当前状态">{patchState(patch.detection.status)}</InfoRow>
        <InfoRow label="兼容性">{workflowState(patch.compatibility)}</InfoRow>
        <InfoRow label="安装方式">{workflowState(patch.support_level)}</InfoRow>
        {patch.blockers.map((b, i) => <Row key={i}><Status warning>{reason(b)}</Status></Row>)}
        <Row><Action onClick={() => openSource(patch.source_page)}>打开来源网页 ↗</Action></Row>
      </PanelSection>
      {active ? <div ref={planRef} tabIndex={-1}><PanelSection title="安装计划">
        <InfoRow label="补丁">{patch.name} · {sourceInfo(patch, report?.sources)}</InfoRow>
        <InfoRow label="文件来源">{acquisition === "download" ? "本次下载" : "本地 ZIP"}</InfoRow>
        <InfoRow label="文件变更">将修改 {active.changes.length} 个文件；覆盖文件会先备份。</InfoRow>
        <InfoRow label="计划有效期">{planExpired(active, now) ? "已过期，请重新准备。" : `剩余 ${Math.ceil((active.expires_at! * 1000 - now) / 1000)} 秒`}</InfoRow>
        {active.blockers.map((b, i) => <Row key={i}><Status warning>{reason(b)}</Status></Row>)}
        <Row><Action onClick={() => setChangesOpen(value => !value)}>查看文件变更（{active.changes.length}）{changesOpen ? " ▴" : " ▾"}</Action></Row>
        {changesOpen && active.changes.map((change, i) => <Row key={i}><Status>{workflowState(change.action)} · {change.path}{change.baseline_matches === false ? " · 原文件已变化" : ""}</Status></Row>)}
        <Row><Action disabled={!active.plan_id || planExpired(active, now) || !!active.blockers.length || !!blocked || !!busy} onClick={() => confirmDialog("确认安装", <p>{game} · {patch.name}<br />将修改 {active.changes.length} 个文件。请关闭游戏并暂停 Steam 更新。</p>, "已关闭游戏及更新，安装", () => void beginInstall())}>继续确认安装</Action></Row>
        <Row><Action disabled={!!busy} onClick={cancelPreparedPlan}>取消本次准备</Action></Row>
      </PanelSection></div> : installed ? <PanelSection title="已安装">
        {committed.length
          ? <Row><Status>此补丁由本工具安装，可恢复备份文件。</Status></Row>
          : installedExternal
            ? <Row><Status>通过其他方式安装；本工具没有原文件备份，不能自动卸载。</Status></Row>
            : <Row><Status warning>检测到已安装，但找不到本工具的安装记录，不能自动卸载。</Status></Row>}
        {committed.map(tx => <Row key={tx.transaction_id}><Action disabled={!!busy} onClick={() => confirmDialog("确认卸载", <p>{game} · {patch.name}<br />将恢复安装前的备份文件。请关闭游戏并暂停 Steam 更新。</p>, "已关闭游戏及更新，卸载", () => void session.work("正在安全卸载…", () => uninstall(tx.transaction_id, true), value => {
          record(value); refreshAfterMutation(value.state === "uninstalled" ? "补丁已卸载，原文件已恢复。" : `卸载状态：${workflowState(value.state)}。${value.error ? reason(value.error) : "请检查操作记录。"}`);
        }))}>卸载补丁</Action></Row>)}
        {!committed.length && (blocked
          ? <Row><Status warning>无法重新安装并接管：{blocked}</Status></Row>
          : <><Row><Status>重新安装并接管会在写入前备份当前文件，成功后即可在本工具中卸载。该操作会覆盖当前补丁文件。</Status></Row>
            <Row><Action disabled={!!busy || !!blocked} onClick={() => void beginPrepare()}>{patch.download_available ? "自动下载并重新安装" : "查找本地 ZIP 并重新安装"}</Action></Row></>)}
      </PanelSection> : <PanelSection title="安装准备"><Row><Status>{patch.download_available ? "自动查找或下载官方 ZIP，并校验后生成安装计划。" : "查找 Downloads 内的 ZIP，并校验后生成安装计划。"}</Status></Row>
        {blocked && <Row><Status warning>无法准备安装：{blocked}</Status></Row>}
        <Row><Action disabled={!!busy || !!blocked} onClick={() => void beginPrepare()}>{patch.download_available ? "自动下载并准备安装" : "查找本地 ZIP 并准备安装"}</Action></Row>
        {patch.support_level === "managed_zip" && <Row><Action onClick={() => setManual(value => !value)}>手动指定本地 ZIP（高级）{manual ? " ▴" : " ▾"}</Action></Row>}
        {manual && patch.support_level === "managed_zip" && <><Row><TextField label="Downloads 内的 ZIP 路径" value={archive} onChange={e => setArchive(e.target.value)} /></Row>
          <Row><Action disabled={!archive.trim() || !!blocked || !!busy} onClick={() => void beginPrepare(archive)}>使用指定 ZIP 准备安装</Action></Row></>}
      </PanelSection>}
    </>}
  </div>;
  const records = <div className="patch-page" ref={activePanel === "records" ? contentRef : undefined} onScroll={e => panelScroll.current.records = e.currentTarget.scrollTop}>
    {common}
    {unfinished.length > 0 && <PanelSection title="未完成的操作"><Row><Status warning>修复会处理以下所有游戏的未完成事务。请关闭这些游戏并暂停更新：</Status></Row>
      {unfinished.map(tx => <Row key={tx.transaction_id}><Status>{report?.installs.find(i => i.game.instance_id === tx.instance_id)?.game.name ?? tx.instance_id} · {workflowState(tx.state)}</Status></Row>)}
      <Row><Action disabled={!!busy} onClick={() => confirmDialog("修复未完成的操作", <p>将尝试恢复上列 {unfinished.length} 个游戏的未完成事务。请确认这些游戏已关闭，且 Steam 更新已暂停。</p>, "已关闭游戏及更新，修复", () => void session.work("正在恢复…", () => recover(true), values => {
        values.forEach(record); refreshAfterMutation("恢复已执行，请检查各项操作结果。");
      }))}>已关闭游戏并暂停更新，修复未完成的操作</Action></Row>
    </PanelSection>}
    <PanelSection title="当前游戏记录">{owned.length ? owned.map(tx => <Row key={tx.transaction_id}><Status>{tx.release_id} · {workflowState(tx.state)}{tx.error ? ` · ${reason(tx.error)}` : ""}</Status></Row>)
      : <Row><Status>没有本工具的操作记录。</Status></Row>}</PanelSection>
  </div>;
  const sources = <SourcesPage report={report} busy={!!busy} rule={rule} setRule={setRule} changeSources={changeSources} contentRef={activePanel === "sources" ? contentRef : undefined} onScroll={value => { panelScroll.current.sources = value; }} />;
  const navPages: ComponentProps<typeof SidebarNavigation>["pages"] = [
    { identifier: `${base}/overview`, route: `${base}/overview`, title: "概览", visible: true, content: overview },
    { identifier: `${base}/patches`, route: `${base}/patches`, title: "补丁管理", visible: true, content: patches },
    { identifier: `${base}/records`, route: `${base}/records`, title: "操作记录", visible: true, content: records },
    "separator", { identifier: `${base}/sources`, route: `${base}/sources`, title: "管理订阅源", visible: true, content: sources },
  ];
  // The decky SidebarNavigation resolves the visible page from page.route
  // against the URL and switches by replacing the URL. `fnSetNavigateToPage`
  // receives the internal switcher, so panel jumps reuse it instead of pushing
  // history, and `disableRouteReporting` leaves Steam's own router untouched.
  // `showTitle` renders the non-focusable two-line header at the top of the
  // page list (native PagedSettings only renders `title` when it is true).
  const navigation = { title: <div className="patch-nav-header"><span className="patch-nav-tool">Steam 补丁管理</span><span className="patch-nav-game" title={game}>{game}</span></div>, showTitle: true, page: `${base}/${activePanel}`, pages: navPages,
    fnSetNavigateToPage: (setter: (route: string) => void) => { navigate.current = setter; },
    disableRouteReporting: true } as unknown as ComponentProps<typeof SidebarNavigation>;
  return <SidebarNavigation {...navigation} />;
}

function SourcesPage({ report, busy, rule, setRule, changeSources, contentRef, onScroll }: {
  report: ScanResult | null; busy: boolean; rule: string; setRule: (value: string) => void;
  changeSources: (action: () => Promise<SourceSubscription[]>) => void;
  contentRef?: React.Ref<HTMLDivElement>; onScroll?: (value: number) => void;
}) {
  return <div className="patch-page" ref={contentRef} onScroll={e => onScroll?.(e.currentTarget.scrollTop)}>
    <Status>全局设置，对所有游戏生效。修改后会更新匹配结果。</Status>
    <PanelSection title="订阅源">{report?.sources?.length ? report.sources.map(source => <Fragment key={source.id}>
      <InfoRow label="名称">{source.name}</InfoRow>
      <InfoRow label="站点" below>{source.pages.join(" · ")}</InfoRow>
      <InfoRow label="启用状态">{source.enabled ? "已启用" : "已停用"}</InfoRow>
      <InfoRow label="查询结果">{source.entry_count != null ? `${source.entry_count} 条` : "—"}</InfoRow>
      <InfoRow label="查询或缓存">{source.status ? reason(source.status) : "待查询"}</InfoRow>
      <InfoRow label="缓存时间">{source.cached_at ? new Date(source.cached_at * 1000).toLocaleString() : "—"}</InfoRow>
      <Row><ToggleField label={`启用 ${source.name}`} checked={source.enabled} disabled={busy} onChange={enabled => changeSources(() => setSourceEnabled(source.id, enabled))} /></Row>
    </Fragment>) : <Row><Status>尚未配置订阅源。</Status></Row>}</PanelSection>
    <PanelSection title="添加源"><Row><TextField label="源规则 JSON（高级）" value={rule} onChange={e => setRule(e.target.value)} /></Row>
      <Row><Action disabled={busy || !rule.trim()} onClick={() => changeSources(() => addSource(rule))}>添加并查询补丁源</Action></Row></PanelSection>
  </div>;
}

function Manager() {
  const { instance, panel } = useParams<{ instance: string; panel?: string }>();
  const { report } = useSyncExternalStore(session.subscribe, session.getSnapshot);
  useEffect(() => { void session.ensureSnapshot(); }, []);
  const target = instance === sourceRoute ? instance : decodeInstance(instance ?? "");
  const item = report?.installs.find(i => i.game.instance_id === target);
  return <Focusable className="patch-full" onCancel={() => Navigation.NavigateBack()}><style>{css}</style>
    {instance === sourceRoute ? <SidebarNavigation showTitle title={<div className="patch-nav-header"><span className="patch-nav-tool">Steam 补丁管理</span><span className="patch-nav-game">全局设置</span></div>} pages={[{ identifier: "sources", route: "/steam-patch-manager/sources", title: "管理订阅源", visible: true, content: <GlobalSources /> }]} />
      : item ? <GameManager key={instance} item={item} seg={instance!} panel={panel} />
        : <div className="patch-page"><h2>Steam 补丁管理</h2><Status>{report ? "游戏已不在当前扫描结果中。" : "正在加载游戏列表…"}</Status><Action onClick={() => Navigation.NavigateBack()}>返回游戏列表</Action></div>}
  </Focusable>;
}
function GlobalSources() {
  const { report, busy, error, note } = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const [rule, setRule] = useState("");
  return <><div>{busy && <Status>{busy}</Status>}{error && <Status warning>{error}</Status>}{note && <Status>{note}</Status>}</div>
    <SourcesPage report={report} busy={!!busy} rule={rule} setRule={setRule} changeSources={action => void session.work("正在更新补丁源并查询…", async () => { await action(); return readSnapshot(); }, snapshot => { session.update(snapshot); setRule(""); })} />
  </>;
}

export default definePlugin(() => {
  routerHook.addRoute(route, Manager);
  return { name: "Steam 补丁管理", titleView: <div>Steam 补丁管理</div>, content: <QuickAccess />, icon: <FaPuzzlePiece />,
    onDismount() { tasks.dismount(); modal?.Close(); routerHook.removeRoute(route); },
  };
});
