import { callable, definePlugin, routerHook } from "@decky/api";
import { Button, ButtonItem, ConfirmModal, DropdownItem, Field, Focusable, Navigation, PanelSection, PanelSectionRow, SidebarNavigation, TextField, ToggleField, showModal, useParams } from "@decky/ui";
import { FaGamepad, FaPuzzlePiece } from "react-icons/fa";
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ComponentProps, type ReactNode } from "react";
import { candidateMatched, displayPatches, filterInstalls, isExternal, isManaged, needsAttention, patchState, reason, workflowState, type DiscoveryResult, type Filter, type Install, type Patch, type Plan, type ScanResult, type SourceCandidate, type SourceSubscription, type Transaction } from "@steam-patch/ui";
import { getListPreferences, setListPreferences, subscribeListPreferences } from "./preferences";
import { gamePresentation } from "./steam";
import { exeConsentReady, installBlocker, planExpired, preparedSource, transactionResult, type ExeConfirmations, type PreparedInstall } from "./install";
import { createManagerSession, needsRecovery } from "./session";
import { decodeInstance, encodeInstance, settingsPage, settingsRoutes } from "./route";
import { autoCleanupNote, cacheClearNote, cacheSummary, formatSize as size, type CacheStatus, type Settings } from "./cache";
import { tasks, taskIsTerminal, type TaskSnapshot } from "./tasks";
import { TaskProgressView } from "./task-progress-view";
import { candidateConditions, candidateLanguage, candidateSource, discoveryStatus, discoverySummary, safeSourceUrl, unqueriedCount } from "./discovery";

const scan = callable<[boolean], ScanResult>("scan");
const discoverGame = callable<[string, string[], string | null, boolean], DiscoveryResult>("discover_game");
const listTransactions = callable<[], Transaction[] | { transactions: Transaction[] }>("list_transactions");
const uninstall = callable<[string, boolean], Transaction>("uninstall");
const recover = callable<[boolean], Transaction[]>("recover");
const setSourceEnabled = callable<[string, boolean], SourceSubscription[]>("set_source_enabled");
const setGameSource = callable<[number, string, boolean], { appid: number }>("set_game_source");
const addSource = callable<[string], SourceSubscription[]>("add_source");
const discardPlan = callable<[string], unknown>("discard_plan");
const clearCache = callable<[], CacheStatus>("clear_cache");
const clearTransactionBackups = callable<[], CacheStatus>("clear_transaction_backups");
const getSettings = callable<[], Settings>("get_settings");
const setAutoCleanup = callable<[boolean], Settings>("set_auto_cleanup");
const setKeepBackups = callable<[boolean], Settings>("set_keep_backups");

async function readSnapshot(refreshSources = false) {
  const report = await scan(refreshSources);
  let transactions = session.getSnapshot().transactions;
  try { const result = await listTransactions(); transactions = Array.isArray(result) ? result : result.transactions ?? []; }
  catch { /* Keep the last journal when an older core cannot list it. */ }
  return { report, transactions };
}
const session = createManagerSession(readSnapshot, discoverGame);
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
  Navigation.Navigate(settingsRoutes.sources);
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
  if (!safeSourceUrl(url)) { session.update({ error: "来源地址不是可用的公开 HTTPS 网页，已阻止打开。" }); return; }
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
  .patch-page { box-sizing:border-box; height:100%; min-height:0; overflow-y:auto; overflow-x:hidden; padding:0 24px 48px; }
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
  @media (max-width:600px) { .patch-page { padding:0 12px 40px; } }
`;

function matchSummary(item: Install): string {
  if (needsAttention(item)) return "需要处理";
  if (item.patches.some(p => isManaged(p.detection.status) || isExternal(p.detection.status))) return "已安装补丁";
  return item.patches.length ? `已找到补丁 · ${item.patches.length} 个补丁` : discoverySummary(item);
}
function CandidateDetails({ candidate }: { candidate: SourceCandidate }) {
  const format = candidate.format_observed && candidate.format_observed !== "unknown" ? candidate.format_observed : candidate.format_reported && candidate.format_reported !== "unknown" ? candidate.format_reported : "待确认";
  const conflict = !!candidate.format_reported && candidate.format_reported !== "unknown" && !!candidate.format_observed && candidate.format_observed !== "unknown" && candidate.format_reported !== candidate.format_observed;
  return <PanelSection title="来源补丁详情">
    <InfoRow label="补丁">{candidate.patch_title}</InfoRow>
    <InfoRow label="来源">{candidateSource(candidate)}</InfoRow>
    <InfoRow label="补丁语言">{candidateLanguage(candidate)}</InfoRow>
    <InfoRow label="补丁版本">{candidate.version_raw || "版本待确认"}</InfoRow>
    <InfoRow label="适用条件" below>{candidateConditions(candidate)}</InfoRow>
    <InfoRow label="文件格式">{format}{conflict ? `（页面标示 ${candidate.format_reported}，上游为 ${candidate.format_observed}）` : ""}</InfoRow>
    {candidate.cache_status && <InfoRow label="来源记录">{reason(candidate.cache_status)}</InfoRow>}
    {!!candidate.version_evidence?.length && candidate.warnings?.includes("version_conflict") && <InfoRow label="页面版本描述" below>{candidate.version_evidence.filter(value => typeof value === "string").join("、")}</InfoRow>}
    <Row><Status>此来源线索用于核对游戏语言、版本与 DLC 条件。只有单独显示为已审核安装版本的补丁才能通过插件安装。</Status></Row>
    {!candidateMatched(candidate) && <Row><Status warning>此线索的游戏身份尚待确认，请核对来源中的游戏信息。</Status></Row>}
    {conflict && <Row><Status warning>索引标示与上游文件格式不一致，不能按 ZIP 安装。</Status></Row>}
    {candidate.warnings?.filter(warning => !conflict || warning !== "artifact_format_conflict").map((warning, index) => <Row key={index}><Status warning>{reason(warning)}</Status></Row>)}
    <Row><Action onClick={() => openSource(candidate.source_page)}>{candidate.source_id === "hgamer" ? "打开补丁详情 ↗" : "打开官方补丁页 ↗"}</Action></Row>
    {candidate.upstream_page && candidate.upstream_page !== candidate.source_page && <Row><Action onClick={() => openSource(candidate.upstream_page!)}>打开上游发布页 ↗</Action></Row>}
  </PanelSection>;
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
      {filter === "matched" && unqueriedCount(report?.installs ?? []) > 0 && <>
        <Row><Status>还有 {unqueriedCount(report?.installs ?? [])} 款未查询。</Status></Row>
        <Row><Action onClick={() => setListPreferences({ filter: "all", query: "" })}>查看全部游戏</Action></Row>
      </>}
    </PanelSection></div>
    <div className="patch-qam-list" ref={listRef} onScroll={e => { listScroll = e.currentTarget.scrollTop; }}>
      <PanelSection title={`游戏列表 · ${shown.length}`}>
        {shown.map(item => <Row key={item.game.instance_id}><GameButton item={item} /></Row>)}
        {report && report.installs.length > 0 && !shown.length && <Row><Status>没有符合条件的游戏。</Status><Action onClick={() => setListPreferences({ filter: "all", query: "" })}>清除筛选</Action></Row>}
        {report && !report.installs.length && <Row><Status>未找到 Steam 游戏。请检查游戏库或 SD 卡。</Status></Row>}
        {report?.diagnostics.filter(d => d.code !== "library_unavailable").map((d, i) => <Row key={i}><Status warning>{reason(d.code)}{d.path ? ` · ${d.path}` : ""}</Status></Row>)}
      </PanelSection>
    </div>
    <div className="patch-qam-controls"><PanelSection>
      <Row><Action onClick={openSources}>全局设置 ›</Action></Row>
      <Row><Action onClick={() => void session.refresh()} disabled={!!busy}>重新扫描</Action></Row>
    </PanelSection></div>
  </div>;
}

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
  return <TaskProgressView task={task} game={game} patch={patch} onCancel={cancelOrClose} />;
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
  const { report, transactions, busy, error, note, discoveries } = useSyncExternalStore(session.subscribe, session.getSnapshot);
  // The decky SidebarNavigation is route-driven: it renders the page whose
  // `route` matches the URL and switches by replacing the URL. Keep one
  // sub-route per panel so selection and content follow the URL.
  const activePanel = panel && (panelKeys as readonly string[]).includes(panel) ? panel : "overview";
  const base = `/steam-patch-manager/${seg}`;
  const navigate = useRef<((route: string) => void) | null>(null);
  const [preferredLanguage, setPreferredLanguage] = useState<string | null>(null);
  const entries = displayPatches(item, preferredLanguage);
  const [selectedKey, setSelectedKey] = useState(entries[0]?.key ?? "");
  const [plan, setPlan] = useState<Plan | null>(null);
  const [archive, setArchive] = useState("");
  const [acquisition, setAcquisition] = useState<PreparedInstall["acquisition"] | null>(null);
  const [preparedMode, setPreparedMode] = useState<PreparedInstall["mode"] | null>(null);
  const [exeConsents, setExeConsents] = useState<ExeConfirmations>({ terms_accepted: false, conditions_accepted: false, keep_backups: false });
  const [manual, setManual] = useState(false);
  const [changesOpen, setChangesOpen] = useState(false);
  const [now, setNow] = useState(Date.now());
  const [keepBackups, setKeepBackupsEnabled] = useState<boolean | null>(null);
  const panelScroll = useRef<Record<string, number>>({});
  const contentRef = useRef<HTMLDivElement>(null);
  const planRef = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);
  const game = gamePresentation(item.game.appid, item.game.name).name;
  const selected = entries.find(entry => entry.key === selectedKey);
  const patch = selected?.kind === "installable" ? selected.patch : undefined;
  const candidate = selected?.kind === "discovery" ? selected.candidate : undefined;
  const release = patch?.release_id ?? "";
  const active = plan?.instance_id === item.game.instance_id && plan.release_id === release ? plan : null;
  const blocked = patch ? installBlocker(patch) : null;
  const owned = transactions.filter(t => t.instance_id === item.game.instance_id).reverse();
  const committed = owned.filter(t => t.release_id === release && t.state === "committed");
  const recoverable = committed.filter(t => t.backups !== false);
  const canUninstall = recoverable.length > 0;
  const installedManaged = patch ? isManaged(patch.detection.status) : false;
  const installedExternal = patch ? isExternal(patch.detection.status) : false;
  const installed = !!patch && (installedManaged || installedExternal || committed.length > 0);
  const unfinished = transactions.filter(needsRecovery);
  useEffect(() => {
    if (!entries.some(entry => entry.key === selectedKey)) {
      setSelectedKey(entries[0]?.key ?? ""); setPlan(null); setAcquisition(null); setArchive(""); setPreparedMode(null);
    }
  }, [item.patches, item.discovery, selectedKey]);
  useEffect(() => {
    void session.discover(item.game.instance_id, [game, item.game.name], preferredLanguage);
  }, [item.game.instance_id, report?.generation, report?.settings_revision]);
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
  useEffect(() => { getSettings().then(value => setKeepBackupsEnabled(value.keep_backups)).catch(() => {}); }, []);
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
  function chooseRelease(next: string) { setSelectedKey(next); setPlan(null); setArchive(""); setAcquisition(null); setPreparedMode(null); setExeConsents({ terms_accepted: false, conditions_accepted: false, keep_backups: false }); setChangesOpen(false); }
  function changeGameSource(sourceId: string, subscribed: boolean) {
    setPlan(null);
    void session.work("正在更新此游戏的订阅源…", async () => {
      await setGameSource(item.game.appid, sourceId, subscribed); session.invalidateDiscovery(sourceId, item.game.appid); return readSnapshot();
    }, snapshot => {
      session.update({ ...snapshot, note: "此游戏的订阅源已更新。" });
    });
  }
  function finishTask(task: TaskSnapshot) {
    if (task.state === "completed" && task.kind === "prepare") {
      const value = task.result as PreparedInstall;
      if (value?.plan.instance_id === item.game.instance_id && value.plan.release_id === task.release_id) {
        if (task.release_id) setSelectedKey(task.release_id);
        setPlan(value.plan); setArchive(value.archive); setAcquisition(value.acquisition); setPreparedMode(value.mode ?? null); setChangesOpen(false);
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
    const isExe = patch.support_level === "publisher_exe";
    if (isExe && (manualArchive || !exeConsentReady(patch, exeConsents))) return;
    try {
      if (isExe) {
        const settings = await getSettings();
        if (!mounted.current) return;
        if (!settings.keep_backups) {
          const updated = await setKeepBackups(true);
          if (!updated.keep_backups) throw new Error("exe_backup_required");
        }
        if (!mounted.current) return;
        setKeepBackupsEnabled(true);
      }
      setPlan(null); setAcquisition(null); setPreparedMode(null);
      const id = await tasks.startPrepare(item.game.instance_id, release, manualArchive, isExe ? exeConsents : undefined);
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
      setPlan(null); setAcquisition(null); setPreparedMode(null); setArchive("");
      session.update({ note: "本次安装计划已取消。" });
    });
  }
  const finding = discoveries[item.game.instance_id];
  const common = <>{busy && <Status>{busy}</Status>}{error && <Status warning>{error}</Status>}{note && <Status>{note}</Status>}
    {finding?.busy && <Status>正在查询此游戏的补丁来源…</Status>}{finding?.error && <Status warning>{finding.error}</Status>}</>;
  const selection = entries.length > 1 ? <PanelSection title="选择补丁"><Row><DropdownItem label="补丁" layout="below"
    rgOptions={entries.map(entry => ({ data: entry.key, label: entry.kind === "installable" ? entry.patch.name : `${entry.candidate.patch_title} · ${entry.candidate.version_raw || "版本待确认"} · ${candidateLanguage(entry.candidate)} · ${candidateSource(entry.candidate)}` }))}
    selectedOption={selectedKey} onChange={option => chooseRelease(option.data)} /></Row></PanelSection> : null;
  const overview = <div className="patch-page" ref={activePanel === "overview" ? contentRef : undefined} onScroll={e => panelScroll.current.overview = e.currentTarget.scrollTop}>
    {common}
    <PanelSection>
      <InfoRow label="补丁状态">{matchSummary(item)}</InfoRow>
      <InfoRow label="匹配来源">{entries.length ? [...new Set(entries.map(entry => entry.kind === "installable" ? entry.patch.source_name || report?.sources?.find(s => s.id === entry.patch.source_id)?.name || "匹配源未知" : candidateSource(entry.candidate)))].join("、") : "—"}</InfoRow>
      {owned.some(needsRecovery) && <Row><Status warning>有未完成的操作需要处理。</Status><Action onClick={() => navigatePanel("records")}>查看操作记录</Action></Row>}
    </PanelSection>
    <PanelSection title="查找来源">
      <InfoRow label="查询状态">{discoveryStatus(item.discovery?.status)}</InfoRow>
      <Row><DropdownItem label="偏好的补丁语言" layout="below" rgOptions={[{ data: "", label: "未指定" }, { data: "zh-CN", label: "简体中文" }, { data: "zh-TW", label: "繁体中文" }, { data: "en", label: "英文" }, { data: "ja", label: "日文" }]}
        selectedOption={preferredLanguage ?? ""} onChange={option => setPreferredLanguage(option.data || null)} /></Row>
      <Row><Status>语言偏好用于查找来源，游戏实际语言与 DLC 状态仍需确认。</Status></Row>
      <Row><Action disabled={finding?.busy} onClick={() => void session.discover(item.game.instance_id, [game, item.game.name], preferredLanguage, true)}>重新查找 / 查看更多来源</Action></Row>
      {item.discovery?.diagnostics.map((diagnostic, index) => <Row key={index}><Status warning>{diagnostic.message || reason(diagnostic.code)}{diagnostic.detail && diagnostic.detail !== diagnostic.code ? ` · ${reason(diagnostic.detail)}` : ""}</Status></Row>)}
    </PanelSection>
    {(!!item.patches.length || !item.discovery?.candidates.length) && <PanelSection title="匹配到的补丁">{item.patches.length ? item.patches.map(p => <PanelSectionRow key={p.release_id}>
      <div className="patch-patch">
        <div className="patch-patch-name">{p.name}</div>
        <div className="patch-patch-source">匹配源：{sourceInfo(p, report?.sources)}</div>
        <div className="patch-actions">
          <Button onClick={() => { chooseRelease(p.release_id); navigatePanel("patches"); }}>查看补丁</Button>
          <Button onClick={() => openSource(p.source_page)}>打开来源网页 ↗</Button>
        </div>
      </div>
    </PanelSectionRow>) : <Row><Status>{discoveryStatus(item.discovery?.status)}</Status><Action onClick={() => navigatePanel("sources")}>管理订阅源</Action></Row>}</PanelSection>}
    {!!item.discovery?.candidates.length && <PanelSection title="来源候选">{entries.filter(entry => entry.kind === "discovery").map(entry => entry.kind === "discovery" && <Row key={entry.key}>
      <div className="patch-patch"><div className="patch-patch-name">{entry.candidate.patch_title}</div>
        <div className="patch-patch-source">{candidateSource(entry.candidate)} · {entry.candidate.version_raw || "版本待确认"} · {candidateLanguage(entry.candidate)}</div>
        <div className="patch-patch-source">{candidateConditions(entry.candidate)}</div>
        <Action onClick={() => { chooseRelease(entry.key); navigatePanel("patches"); }}>查看补丁条件与来源</Action>
      </div></Row>)}</PanelSection>}
    <PanelSection title="游戏信息">
      <InfoRow label="游戏编号">{item.game.appid}</InfoRow>
      <InfoRow label="游戏版本">{item.game.buildid ?? "未知"}</InfoRow>
      <InfoRow label="安装位置" below>{item.game.install_path}</InfoRow>
    </PanelSection>
  </div>;
  const patches = <div className="patch-page" ref={activePanel === "patches" ? contentRef : undefined} onScroll={e => panelScroll.current.patches = e.currentTarget.scrollTop}>
    {common}
    {selection}
    {candidate ? <CandidateDetails candidate={candidate} /> : !patch ? <PanelSection><Row><Status>{discoveryStatus(item.discovery?.status)}</Status></Row></PanelSection> : <>
      <PanelSection title="选择补丁"><InfoRow label="补丁">{patch.name}</InfoRow>
        <InfoRow label="匹配源">{sourceInfo(patch, report?.sources)}</InfoRow>
      </PanelSection>
      <PanelSection title="补丁详情">
        <InfoRow label="当前状态">{patchState(patch.detection.status)}</InfoRow>
        <InfoRow label="兼容性">{workflowState(patch.compatibility)}</InfoRow>
        <InfoRow label="安装方式">{workflowState(patch.support_level)}</InfoRow>
        {patch.support_level === "publisher_exe" && patch.exe && <>
          <InfoRow label="补丁语言">{patch.exe.language || "请核对官方说明"}</InfoRow>
          <InfoRow label="补丁版本">{patch.exe.version || "请核对官方说明"}</InfoRow>
          <InfoRow label="适用版本">{patch.exe.variant === "base" ? "游戏本体" : patch.exe.variant === "dlc" ? "DLC" : patch.exe.variant || "请核对官方说明"}</InfoRow>
          <InfoRow label="DLC 条件" below>{patch.exe.requires_dlc?.length ? `需要 DLC ${patch.exe.requires_dlc.join("、")}` : "无额外 DLC 要求"}；{patch.exe.excludes_dlc?.length ? `不可使用 DLC ${patch.exe.excludes_dlc.join("、")}` : "无排除的 DLC"}</InfoRow>
        </>}
        {patch.blockers.map((b, i) => <Row key={i}><Status warning>{reason(b)}</Status></Row>)}
        <Row><Action onClick={() => openSource(patch.source_page)}>打开来源网页 ↗</Action></Row>
      </PanelSection>
      {active ? <div ref={planRef} tabIndex={-1}><PanelSection title="安装计划">
        <InfoRow label="补丁">{patch.name} · {sourceInfo(patch, report?.sources)}</InfoRow>
        <InfoRow label="文件来源">{preparedMode === "publisher_exe" ? acquisition === "download" ? "本次下载的官方 EXE" : "本机缓存的官方 EXE" : acquisition === "download" ? "本次下载" : "本地 ZIP"}</InfoRow>
        <InfoRow label="文件变更">将修改 {active.changes.length} 个文件；覆盖文件会先备份。</InfoRow>
        {preparedMode === "publisher_exe" && <Row><Status>本次原文件备份必须保留，安装后可用于恢复；关闭全局备份设置或手动清理缓存不会删除受保护的备份。</Status></Row>}
        {active.disk_requirement != null && <InfoRow label="所需空间">至少 {size(active.disk_requirement)}</InfoRow>}
        <InfoRow label="计划有效期">{planExpired(active, now) ? "已过期，请重新准备。" : `剩余 ${Math.ceil((active.expires_at! * 1000 - now) / 1000)} 秒`}</InfoRow>
        {active.blockers.map((b, i) => <Row key={i}><Status warning>{reason(b)}</Status></Row>)}
        <Row><Action onClick={() => setChangesOpen(value => !value)}>查看文件变更（{active.changes.length}）{changesOpen ? " ▴" : " ▾"}</Action></Row>
        {changesOpen && active.changes.map((change, i) => <Row key={i}><Status>{workflowState(change.action)} · {change.path}{change.baseline_matches === false ? " · 原文件已变化" : ""}</Status></Row>)}
        <Row><Action disabled={!active.plan_id || planExpired(active, now) || !!active.blockers.length || !!blocked || !!busy} onClick={() => confirmDialog("确认安装", <p>{game} · {patch.name}<br />将修改 {active.changes.length} 个文件。请关闭游戏并暂停 Steam 更新。</p>, "已关闭游戏及更新，安装", () => void beginInstall())}>继续确认安装</Action></Row>
        <Row><Action disabled={!!busy} onClick={cancelPreparedPlan}>取消本次准备</Action></Row>
      </PanelSection></div> : installed ? <PanelSection title="已安装">
        {committed.length
          ? (canUninstall
            ? <Row><Status>此补丁由本工具安装，可恢复备份文件。</Status></Row>
            : <Row><Status warning>此补丁由本工具安装，但未保留原文件备份，无法卸载，只能重新安装。</Status></Row>)
          : installedExternal
            ? <Row><Status>通过其他方式安装；本工具没有原文件备份，不能自动卸载。</Status></Row>
            : <Row><Status warning>检测到已安装，但找不到本工具的安装记录，不能自动卸载。</Status></Row>}
        {recoverable.map(tx => <Row key={tx.transaction_id}><Action disabled={!!busy} onClick={() => confirmDialog("确认卸载", <p>{game} · {patch.name}<br />将恢复安装前的备份文件。请关闭游戏并暂停 Steam 更新。</p>, "已关闭游戏及更新，卸载", () => void session.work("正在安全卸载…", () => uninstall(tx.transaction_id, true), value => {
          record(value); refreshAfterMutation(value.state === "uninstalled" ? "补丁已卸载，原文件已恢复。" : `卸载状态：${workflowState(value.state)}。${value.error ? reason(value.error) : "请检查操作记录。"}`);
        }))}>卸载补丁</Action></Row>)}
        {!canUninstall && (blocked
          ? <Row><Status warning>无法重新安装并接管：{blocked}</Status></Row>
          : <><Row><Status>{patch.support_level === "publisher_exe" ? "此补丁需要保留原文件备份；请先核对下方适用条件和条款。" : keepBackups ? "重新安装会重新下载补丁并覆盖当前文件；成功后即可在本工具中卸载。" : "重新安装会重新下载补丁并覆盖当前文件。当前不保留原文件备份，安装后仍无法卸载。"}</Status></Row>
            {patch.support_level !== "publisher_exe" && <Row><Action disabled={!!busy || !!blocked} onClick={() => void beginPrepare()}>{patch.download_available ? "自动下载并重新安装" : "查找本地 ZIP 并重新安装"}</Action></Row>}</>)}
      </PanelSection> : <PanelSection title="安装准备"><Row><Status>{patch.support_level === "publisher_exe" ? "下载并检查已审核的官方 EXE，核对游戏后生成安装计划。准备前请阅读并确认以下条款与适用条件。" : patch.download_available ? "自动查找或下载官方 ZIP，并校验后生成安装计划。" : "查找 Downloads 内的 ZIP，并校验后生成安装计划。"}</Status></Row>
        {blocked && <Row><Status warning>无法准备安装：{blocked}</Status></Row>}
        {patch.support_level === "publisher_exe" && patch.exe && <>
          <InfoRow label="官方适用范围" below>{patch.exe.language || "语言待核对"} · {patch.exe.version || "版本待核对"} · {patch.exe.variant === "base" ? "本体" : patch.exe.variant === "dlc" ? "DLC" : patch.exe.variant || "类型待核对"}；{patch.exe.requires_dlc?.length ? `需要 DLC ${patch.exe.requires_dlc.join("、")}` : "无额外 DLC 要求"}；{patch.exe.excludes_dlc?.length ? `不可使用 DLC ${patch.exe.excludes_dlc.join("、")}` : "无排除的 DLC"}</InfoRow>
          {patch.exe.terms?.text && <InfoRow label="官方条款与说明" below>{patch.exe.terms.text}</InfoRow>}
          {patch.exe.terms?.url && <Row><Action onClick={() => openSource(patch.exe!.terms!.url!)}>阅读官方完整条款与安装说明 ↗</Action></Row>}
          <Row><ToggleField label="我已阅读并同意官方条款及安装说明" checked={exeConsents.terms_accepted} disabled={!!busy} onChange={value => setExeConsents(current => ({ ...current, terms_accepted: value }))} /></Row>
          <Row><ToggleField label="我已核对游戏语言、版本及 DLC 条件" checked={exeConsents.conditions_accepted} disabled={!!busy} onChange={value => setExeConsents(current => ({ ...current, conditions_accepted: value }))} /></Row>
          <Row><ToggleField label="启用并保留本次原文件备份，以便恢复" checked={exeConsents.keep_backups} disabled={!!busy} onChange={value => setExeConsents(current => ({ ...current, keep_backups: value }))} /></Row>
          <Row><Status>准备时将启用全局备份保留；本次 EXE 的备份受保护，之后关闭全局设置或清理缓存也会保留。</Status></Row>
        </>}
        <Row><Action disabled={!!busy || !!blocked || patch.support_level === "publisher_exe" && !exeConsentReady(patch, exeConsents)} onClick={() => void beginPrepare()}>{patch.support_level === "publisher_exe" ? "下载官方 EXE 并准备安装" : patch.download_available ? "自动下载并准备安装" : "查找本地 ZIP 并准备安装"}</Action></Row>
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
    <PanelSection title="当前游戏记录">{owned.length ? owned.map(tx => <Row key={tx.transaction_id}><Status>{tx.release_id} · {workflowState(tx.state)}{tx.error ? ` · ${reason(tx.error)}` : ""}</Status>
      {tx.state === "committed" && tx.backups !== false && <Action disabled={!!busy} onClick={() => confirmDialog("确认卸载", <p>{game}<br />将恢复此操作保存的原文件备份。请关闭游戏并暂停 Steam 更新。</p>, "已关闭游戏及更新，卸载", () => void session.work("正在安全卸载…", () => uninstall(tx.transaction_id, true), value => { record(value); refreshAfterMutation(value.state === "uninstalled" ? "补丁已卸载，原文件已恢复。" : `卸载状态：${workflowState(value.state)}。`); }))}>卸载此记录的补丁</Action>}
      </Row>)
      : <Row><Status>没有本工具的操作记录。</Status></Row>}</PanelSection>
  </div>;
  const sources = <div className="patch-page" ref={activePanel === "sources" ? contentRef : undefined} onScroll={e => { panelScroll.current.sources = e.currentTarget.scrollTop; }}>
    {common}
    <Status>选择用于此游戏的订阅源。关闭后不再从该源匹配补丁；选择按游戏编号保存。</Status>
    {item.sources?.length ? item.sources.map(source => <PanelSection key={source.id} title={source.name}>
      <Row><ToggleField label="用于此游戏" checked={source.subscribed} disabled={!!busy}
        onChange={subscribed => changeGameSource(source.id, subscribed)} /></Row>
      <InfoRow label="游戏匹配">{source.matched || item.discovery?.candidates.some(candidate => candidate.source_id === source.id && candidateMatched(candidate)) ? "已找到此游戏的补丁" : discoveryStatus(item.discovery?.status)}</InfoRow>
      {!source.enabled && <Row><Status warning>此源已在全局设置中停用，当前不会参与匹配。</Status></Row>}
      <InfoRow label="查询状态">{source.status ? reason(source.status) : "待查询"}</InfoRow>
      <Row><Action onClick={() => openSource(source.pages[0])}>打开来源网页 ↗</Action></Row>
    </PanelSection>) : <PanelSection><Row><Status>尚未配置订阅源，请从主面板的全局设置添加。</Status></Row></PanelSection>}
  </div>;
  const navPages: ComponentProps<typeof SidebarNavigation>["pages"] = [
    { identifier: `${base}/overview`, route: `${base}/overview`, title: "概览", visible: true, content: overview },
    { identifier: `${base}/patches`, route: `${base}/patches`, title: "补丁管理", visible: true, content: patches },
    { identifier: `${base}/records`, route: `${base}/records`, title: "操作记录", visible: true, content: records },
    "separator", { identifier: `${base}/sources`, route: `${base}/sources`, title: "订阅源管理", visible: true, content: sources },
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
    <Status>全局设置，对所有游戏生效。选择游戏后按需查询；HGamer 会收到该游戏的检索名称，可随时关闭。</Status>
    <PanelSection><Row><Action disabled={busy} onClick={() => void session.work("正在刷新补丁来源…", () => readSnapshot(true), session.update)}>刷新来源</Action></Row></PanelSection>
    {([{ role: "publisher", title: "发行商补丁源" }, { role: "fallback_indexer", title: "补丁索引" }]).map(group => <PanelSection key={group.role} title={group.title}>{report?.sources?.filter(source => (source.role === "fallback_indexer" || source.id === "hgamer" ? "fallback_indexer" : "publisher") === group.role).map(source => <Fragment key={source.id}>
      <InfoRow label="名称">{source.name}</InfoRow>
      <InfoRow label="站点" below>{source.pages.join(" · ")}</InfoRow>
      <InfoRow label="启用状态">{source.enabled ? "已启用" : "已停用"}</InfoRow>
      <InfoRow label="查询结果">{source.entry_count != null ? `${source.entry_count} 条` : "—"}</InfoRow>
      <InfoRow label="查询或缓存">{source.status ? reason(source.status) : "待查询"}</InfoRow>
      <InfoRow label="缓存时间">{source.cached_at ? new Date(source.cached_at * 1000).toLocaleString() : "—"}</InfoRow>
      <InfoRow label="最近查询">{source.checked_at ? new Date(source.checked_at * 1000).toLocaleString() : "尚未查询"}</InfoRow>
      <Row><ToggleField label={`启用 ${source.name}`} checked={source.enabled} disabled={busy} onChange={enabled => changeSources(() => setSourceEnabled(source.id, enabled))} /></Row>
    </Fragment>)}</PanelSection>)}
    <PanelSection title="添加源"><Row><TextField label="源规则 JSON（高级）" value={rule} onChange={e => setRule(e.target.value)} /></Row>
      <Row><Action disabled={busy || !rule.trim()} onClick={() => changeSources(() => addSource(rule))}>添加补丁源</Action></Row></PanelSection>
  </div>;
}

function GlobalCache() {
  const { busy, error, note } = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let live = true;
    getSettings().then(value => { if (live) setSettings(value); }).catch(() => { if (live) setFailed(true); });
    return () => { live = false; };
  }, []);
  const status = settings?.cache;
  const toggle = (enabled: boolean) => void session.work(enabled ? "正在开启自动清理…" : "正在关闭自动清理…",
    () => setAutoCleanup(enabled),
    value => { setSettings(value); session.update({ note: autoCleanupNote(value.auto_cleanup) }); });
  const toggleBackups = (enabled: boolean) => void session.work(enabled ? "正在保留原文件备份…" : "正在关闭备份保留…",
    () => setKeepBackups(enabled),
    value => { setSettings(value); session.update({ note: value.keep_backups ? "已保留原文件备份，安装后仍可卸载。" : "已关闭备份保留，安装后不再保存原文件备份，无法卸载、只能重新安装。" }); });
  const clean = () => {
    const amount = status ? size(status.total_bytes) : "";
    confirmDialog("清理下载缓存", <p>将删除本工具已下载的补丁缓存{amount ? `（约 ${amount}）` : ""}。受保护的 EXE 原文件备份会保留。</p>, "清理下载缓存",
      () => void session.work("正在清理下载缓存…", clearCache, value => {
        setSettings(current => current ? { ...current, cache: value } : current);
        session.update({ note: cacheClearNote(value.removed_bytes ?? 0) });
      }));
  };
  const transactionBytes = (status?.jobs.stage_bytes ?? 0) + (status?.jobs.backup_bytes ?? 0);
  const cleanTransactions = () => {
    const amount = status ? size(transactionBytes) : "";
    confirmDialog("清理事务备份与暂存", <p>将删除可清理的暂存与备份{amount ? `（约 ${amount}）` : ""}。受保护的 EXE 备份会保留；其他已安装补丁删除备份后将无法卸载。</p>, "清理事务空间",
      () => void session.work("正在清理事务空间…", clearTransactionBackups, value => {
        setSettings(current => current ? { ...current, cache: value } : current);
        session.update({ note: cacheClearNote(value.removed_bytes ?? 0) });
      }));
  };
  return <div className="patch-page">
    <div>{busy && <Status>{busy}</Status>}{error && <Status warning>{error}</Status>}{note && <Status>{note}</Status>}</div>
    <Status>全局设置，对所有游戏生效。</Status>
    <PanelSection title="下载缓存">
      {settings ? <>
        <Row><ToggleField label="安装成功后自动删除下载缓存" checked={settings.auto_cleanup} disabled={!!busy} onChange={toggle} /></Row>
        <InfoRow label="当前占用">{cacheSummary(status)}</InfoRow>
        <Row><Status>{settings.auto_cleanup ? "开启时，安装成功后会删除本次下载的 ZIP；卸载后重装需重新下载。" : "已关闭自动清理，缓存将保留直至手动清理。"}</Status></Row>
        <Row><Action disabled={!!busy || !status?.file_count} onClick={clean}>清理下载缓存</Action></Row>
      </> : <Row><Status warning={failed}>{failed ? "读取缓存设置失败。" : "正在读取缓存设置…"}</Status></Row>}
    </PanelSection>
    <PanelSection title="事务备份与暂存">
      <InfoRow label="暂存文件">{size(status?.jobs.stage_bytes ?? 0)}</InfoRow>
      <InfoRow label="原文件备份">{size(status?.jobs.backup_bytes ?? 0)}</InfoRow>
      {settings && <Row><ToggleField label="保留原文件备份（可卸载，占空间）" checked={settings.keep_backups} disabled={!!busy} onChange={toggleBackups} /></Row>}
      {settings && <Row><Status warning={!settings.keep_backups}>{settings.keep_backups ? "已保留原文件备份，安装后可卸载；备份约占与补丁文件相近的空间。" : "普通补丁不保留原文件备份，安装后无法卸载；已安装 EXE 的受保护备份仍会保留。"}</Status></Row>}
      <Row><Action disabled={!!busy || !transactionBytes} onClick={cleanTransactions}>清理事务备份与暂存</Action></Row>
    </PanelSection>
  </div>;
}

function Manager() {
  const { instance, panel } = useParams<{ instance: string; panel?: string }>();
  const { report } = useSyncExternalStore(session.subscribe, session.getSnapshot);
  useEffect(() => { void session.ensureSnapshot(); }, []);
  const target = instance === sourceRoute ? instance : decodeInstance(instance ?? "");
  const item = report?.installs.find(i => i.game.instance_id === target);
  return <Focusable className="patch-full" onCancel={() => Navigation.NavigateBack()}><style>{css}</style>
    {instance === sourceRoute ? <SidebarNavigation showTitle disableRouteReporting title={<div className="patch-nav-header"><span className="patch-nav-tool">Steam 补丁管理</span><span className="patch-nav-game">全局设置</span></div>} page={settingsPage(panel)} pages={[
      { identifier: settingsRoutes.sources, route: settingsRoutes.sources, title: "管理订阅源", visible: true, content: <GlobalSources /> },
      "separator",
      { identifier: settingsRoutes.cache, route: settingsRoutes.cache, title: "下载缓存", visible: true, content: <GlobalCache /> },
    ]} />
      : item ? <GameManager key={instance} item={item} seg={instance!} panel={panel} />
        : <div className="patch-page"><h2>Steam 补丁管理</h2><Status>{report ? "游戏已不在当前扫描结果中。" : "正在加载游戏列表…"}</Status><Action onClick={() => Navigation.NavigateBack()}>返回游戏列表</Action></div>}
  </Focusable>;
}
function GlobalSources() {
  const { report, busy, error, note } = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const [rule, setRule] = useState("");
  return <><div>{busy && <Status>{busy}</Status>}{error && <Status warning>{error}</Status>}{note && <Status>{note}</Status>}</div>
    <SourcesPage report={report} busy={!!busy} rule={rule} setRule={setRule} changeSources={action => void session.work("正在更新补丁源…", async () => { await action(); session.invalidateDiscovery(); return readSnapshot(); }, snapshot => { session.update(snapshot); setRule(""); })} />
  </>;
}

export default definePlugin(() => {
  routerHook.addRoute(route, Manager);
  return { name: "Steam 补丁管理", titleView: <div>Steam 补丁管理</div>, content: <QuickAccess />, icon: <FaPuzzlePiece />,
    onDismount() { tasks.dismount(); modal?.Close(); routerHook.removeRoute(route); },
  };
});
