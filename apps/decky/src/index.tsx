import { callable, definePlugin, routerHook } from "@decky/api";
import { Button, ButtonItem, DropdownItem, Field, Focusable, Navigation, PanelSection, PanelSectionRow, SidebarNavigation, TextField, ToggleField, showModal, useParams } from "@decky/ui";
import { FaGamepad, FaPuzzlePiece } from "react-icons/fa";
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ComponentProps, type ReactNode } from "react";
import { candidateMatched, displayPatches, patchInstallable, filterInstalls, gameDisplayName, installSummary, patchState, reason, workflowState, type DiscoveryResult, type Filter, type Install, type InstallerLinks, type Patch, type Preview, type ScanResult, type SourceCandidate, type SourceSubscription, type Transaction } from "@steam-patch/ui";
import { getListPreferences, setListPreferences, subscribeListPreferences } from "./preferences";
import { gamePresentation, matchesGameQuery, transactionGameName } from "./steam";
import { transactionResult } from "./install";
import { createManagerSession, feedbackFor, needsRecovery } from "./session";
import { decodeInstance, encodeInstance, settingsPage, settingsRoutes } from "./route";
import { autoCleanupNote, cacheClearNote, cacheSummary, type CacheStatus, type Settings } from "./cache";
import { tasks, taskIsTerminal, type TaskSnapshot } from "./tasks";
import { TaskProgressView, TaskResultView } from "./task-progress-view";
import { candidateConditions, candidateLanguage, candidateSource, discoveryStatus, safeSourceUrl, unqueriedCount } from "./discovery";

const scan = callable<[boolean], ScanResult>("scan");
const scanStatusRpc = callable<[number], ScanResult>("scan_status");
const capabilities = callable<[], { installer?: { links?: InstallerLinks } }>("get_capabilities");
const previewRpc = callable<[string, string | null, string | null], Preview>("preview");
let installerLinks: InstallerLinks | null = null;
let catalogSigned = false;
function projectPatch(patch: Patch): Patch {
  return { ...patch, auto_installable: patchInstallable(patch, installerLinks, catalogSigned) };
}
function projectReport(report: ScanResult): ScanResult {
  catalogSigned = report.catalog_trust === "signed";
  return { ...report, installs: report.installs.map(row => ({ ...row, patches: row.patches.map(projectPatch) })) };
}
const scanStatus = async (generation: number) => projectReport(await scanStatusRpc(generation));
const discoverGameRpc = callable<[string, string[], string | null, boolean], DiscoveryResult>("discover_game");
const discoverGame = async (instance: string, names: string[], language: string | null, force: boolean): Promise<DiscoveryResult> => {
  const result = await discoverGameRpc(instance, names, language, force);
  return { ...result, patches: result.patches?.map(projectPatch) };
};
const listTransactions = callable<[], Transaction[] | { transactions: Transaction[] }>("list_transactions");
const recover = callable<[], Transaction[]>("recover");
const setSourceEnabled = callable<[string, boolean], SourceSubscription[]>("set_source_enabled");
const setGameSource = callable<[number, string, boolean], { appid: number }>("set_game_source");
const addSource = callable<[string], SourceSubscription[]>("add_source");
const clearCache = callable<[], CacheStatus>("clear_cache");
const getSettings = callable<[], Settings>("get_settings");
const setAutoCleanup = callable<[boolean], Settings>("set_auto_cleanup");

async function readSnapshot(refreshSources = false) {
  const [rawReport, caps] = await Promise.all([scan(refreshSources), capabilities()]);
  installerLinks = caps.installer?.links ?? null;
  const report = projectReport(rawReport);
  let transactions = session.getSnapshot().transactions;
  try { const result = await listTransactions(); transactions = Array.isArray(result) ? result : result.transactions ?? []; }
  catch { /* Keep the last transaction snapshot during a transient read failure. */ }
  return { report, transactions };
}
const session = createManagerSession(readSnapshot, discoverGame, scanStatus);
const route = "/steam-patch-manager/:instance/:panel?";
const sourceRoute = "sources";
const panelKeys = ["overview", "patches", "records", "sources"] as const;
const filterOptions: { data: Filter; label: string }[] = [
  { data: "all", label: "全部" }, { data: "matched", label: "有补丁" },
  { data: "installed", label: "已安装补丁" }, { data: "attention", label: "需要处理" },
];
let listScroll = 0;
let lastGame: string | null = null;
// Reuse a request identity when the start RPC reply is lost and the user retries.
const pendingRequestIds = new Map<string, string>();

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
function refreshAfterMutation(note?: string, error?: string, instance: string | null = null) {
  void session.refresh().then(() => { if (note || error) session.setFeedback({ ...(note ? { note } : {}), ...(error ? { error } : {}) }, instance); });
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
  return installSummary({ ...item, patches: item.patches.map(projectPatch) }, !!session.getSnapshot().report?.scan_progress?.running);
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
    <Row><Status>此来源线索用于核对游戏语言、版本与 DLC 条件。可安装补丁会单独列出，安装时自动检查实际文件和条件。</Status></Row>
    {!candidateMatched(candidate) && <Row><Status warning>此线索的游戏身份尚待确认，请核对来源中的游戏信息。</Status></Row>}
    {conflict && <Row><Status warning>索引标示与上游文件格式不一致，不能按 ZIP 安装。</Status></Row>}
    {candidate.warnings?.filter(warning => !conflict || warning !== "artifact_format_conflict").map((warning, index) => <Row key={index}><Status warning>{reason(warning)}</Status></Row>)}
    <Row><Action onClick={() => openSource(candidate.source_page)}>{candidate.source_id === "hgamer" ? "打开补丁详情 ↗" : "打开补丁来源页 ↗"}</Action></Row>
    {candidate.upstream_page && candidate.upstream_page !== candidate.source_page && <Row><Action onClick={() => openSource(candidate.upstream_page!)}>打开上游发布页 ↗</Action></Row>}
  </PanelSection>;
}
function GameButton({ item }: { item: Install }) {
  const { name, icon } = gamePresentation(item.game.appid, gameDisplayName(item.game));
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
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { report, busy } = state;
  const { error, note } = feedbackFor(state);
  const { filter, query } = useSyncExternalStore(subscribeListPreferences, getListPreferences);
  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => { void session.ensureSnapshot(); void tasks.resume().catch(() => {}); }, []);
  useLayoutEffect(() => { if (listRef.current) listRef.current.scrollTop = listScroll; }, [report, filter, query]);
  const shown = useMemo(() => filterInstalls(report?.installs ?? [], filter, "").filter(item => {
    return matchesGameQuery(item, query);
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
      {report?.scan_progress?.running && <Row><Status>正在查找补丁 · {report.scan_progress.completed}/{report.scan_progress.total}</Status></Row>}
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

function TaskModal({ game, patch, onTerminal, onExit }: { game: string; patch: string; onTerminal: (task: TaskSnapshot) => void; onExit: () => void }) {
  useEffect(() => onExit, [onExit]);
  const task = useSyncExternalStore(tasks.subscribe, tasks.getSnapshot);
  useEffect(() => {
    if (task && tasks.claimTerminal(task)) onTerminal(task);
  }, [task, onTerminal]);
  const cancelOrClose = () => {
    if (!task || !taskIsTerminal(task.state)) { void tasks.requestCancel(); return; }
    hideTask(); tasks.clear();
  };
  return task && taskIsTerminal(task.state)
    ? <TaskResultView task={task} onClose={cancelOrClose} />
    : <TaskProgressView task={task} game={game} patch={patch} onClose={cancelOrClose} />;
}
let modal: ReturnType<typeof showModal> | null = null;
let modalOwner: object | null = null;
function hideTask() {
  const previous = modal;
  modalOwner = null; modal = null;
  previous?.Close();
}
function showTask(game: string, patch: string, onTerminal: (task: TaskSnapshot) => void) {
  if (modal) return;
  const owner = {};
  modalOwner = owner;
  modal = showModal(<TaskModal game={game} patch={patch} onTerminal={onTerminal} onExit={() => {
    // Programmatic replacement must not let an old modal cancel a new task.
    if (modalOwner !== owner) return;
    modalOwner = null; modal = null; tasks.dismount();
  }} />, undefined, { strTitle: "补丁安装", bHideMainWindowForPopouts: false });
}

function GameManager({ item, seg, panel }: { item: Install; seg: string; panel?: string }) {
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { report, transactions, busy, discoveries } = state;
  const { error, note } = feedbackFor(state, item.game.instance_id);
  // The decky SidebarNavigation is route-driven: it renders the page whose
  // `route` matches the URL and switches by replacing the URL. Keep one
  // sub-route per panel so selection and content follow the URL.
  const activePanel = panel && (panelKeys as readonly string[]).includes(panel) ? panel : "overview";
  const base = `/steam-patch-manager/${seg}`;
  const navigate = useRef<((route: string) => void) | null>(null);
  const [preferredLanguage, setPreferredLanguage] = useState<string | null>(null);
  const entries = displayPatches(item, preferredLanguage);
  const [selectedKey, setSelectedKey] = useState(entries[0]?.key ?? "");
  const [archive, setArchive] = useState("");
  const [preview, setPreview] = useState<Preview | null>(null);
  const [manual, setManual] = useState(false);
  const panelScroll = useRef<Record<string, number>>({});
  const contentRef = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);
  const game = gamePresentation(item.game.appid, gameDisplayName(item.game)).name;
  const selected = entries.find(entry => entry.key === selectedKey);
  const patch = selected?.kind === "installable" ? selected.patch : undefined;
  const candidate = selected?.kind === "discovery" ? selected.candidate : undefined;
  const identity = patch?.selection_id ?? patch?.release_id ?? "";
  const available = !!patch && patchInstallable(patch, installerLinks, report?.catalog_trust === "signed");
  const link = patch?.support_level === "publisher_exe" ? installerLinks?.exe : installerLinks?.zip;
  const blocked = patch && !available ? [...patch.blockers, ...(link?.blockers ?? []).map(value => value.code)].map(reason).join(" ") || "此来源目前不能自动安装。" : null;
  const manualBlocked = !patch || patch.support_level !== "managed_zip" || patch.blockers.length > 0 || report?.catalog_trust !== "signed" || installerLinks?.zip?.available !== true;
  const owned = transactions.filter(t => t.instance_id === item.game.instance_id).reverse();
  const installed = !!patch && (patch.reinstall_available === true || ["managed", "installed_managed", "installed_owned", "external", "installed_external"].includes(patch.detection.status));
  const unfinished = transactions.filter(needsRecovery);
  useEffect(() => {
    if (!entries.some(entry => entry.key === selectedKey)) {
      setSelectedKey(entries[0]?.key ?? ""); setArchive("");
    }
  }, [item.patches, item.discovery, selectedKey]);
  useEffect(() => {
    mounted.current = true;
    void session.ensureSnapshot();
    const restoreTask = () => {
      if (document.hidden) return;
      void tasks.resume().then(() => {
        const task = tasks.getSnapshot();
        if (!mounted.current || document.hidden || !task || task.instance_id !== item.game.instance_id) return;
        const name = item.patches.find(p => p.selection_id ? p.selection_id === task.selection_id : p.release_id === task.release_id)?.name ?? task.selection_id ?? task.release_id ?? "补丁";
        showTask(game, name, finishTask);
      }).catch(() => {});
    };
    const leave = () => { tasks.dismount(); hideTask(); };
    const visibility = () => { if (document.hidden) leave(); else restoreTask(); };
    restoreTask();
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("pagehide", leave);
    return () => {
      mounted.current = false;
      document.removeEventListener("visibilitychange", visibility);
      window.removeEventListener("pagehide", leave);
      leave();
    };
  }, []);
  useLayoutEffect(() => { if (contentRef.current) contentRef.current.scrollTop = panelScroll.current[activePanel] ?? 0; }, [activePanel]);
  function navigatePanel(next: string) {
    if (next === activePanel) return;
    const target = `${base}/${next}`;
    // The sidebar exposes its own switcher, which replaces the URL instead of
    // growing browser history; fall back to a normal navigation before it
    // registers on the first render.
    if (navigate.current) navigate.current(target); else Navigation.Navigate(target);
  }
  function chooseRelease(next: string) { setSelectedKey(next); setArchive(""); setPreview(null); }
  function changeGameSource(sourceId: string, subscribed: boolean) {
    void session.work("正在更新此游戏的订阅源…", async () => {
      await setGameSource(item.game.appid, sourceId, subscribed); session.invalidateDiscovery(sourceId, item.game.appid); return readSnapshot();
    }, snapshot => {
      session.acceptSnapshot(snapshot); session.setFeedback({ note: "此游戏的订阅源已更新。" }, item.game.instance_id);
    }, item.game.instance_id);
  }
  function finishTask(task: TaskSnapshot) {
    const instance = task.instance_id ?? item.game.instance_id;
    if (task.result?.state === "needs_recovery" || task.result?.recovery?.required) {
      if (task.result?.transaction_id) record(task.result);
      const diagnostic = task.result?.recovery?.code ?? task.result?.error ?? task.error ?? "needs_recovery";
      refreshAfterMutation(undefined, `安装仍需恢复：${reason(diagnostic)}${task.result?.recovery?.detail ? ` · ${task.result.recovery.detail}` : ""}`, instance);
    } else if (task.state === "completed") {
      const tx = task.result as Transaction;
      if (tx?.transaction_id) record(tx);
      refreshAfterMutation(tx ? transactionResult(tx) : "安装完成，正在核对文件状态。", undefined, instance);
    } else if (task.state === "failed") {
      const error = `${reason(task.error ?? "任务失败，请检查操作记录。")}${task.error_detail ? ` · ${task.error_detail}` : ""}`;
      session.setFeedback({ error }, instance);
      refreshAfterMutation(undefined, error, instance);
    } else if (task.state === "cancelled") {
      refreshAfterMutation("任务已安全取消。", undefined, instance);
    }
  }
  async function beginInstall(manualArchive = "") {
    if (!patch || busy || tasks.isBusy() || (manualArchive ? manualBlocked : !!blocked)) return;
    try {
      const operation = installed ? "reinstall" : "install";
      const key = `${item.game.instance_id}\0${identity}\0${operation}\0${manualArchive}`;
      const requestId = pendingRequestIds.get(key) ?? crypto.randomUUID();
      pendingRequestIds.set(key, requestId);
      hideTask();
      const pending = tasks.startInstall(item.game.instance_id, patch.selection_id ? null : patch.release_id, patch.selection_id ?? null, operation, requestId, manualArchive);
      // Show the foreground UI before the start RPC returns, so B can cancel startup.
      showTask(game, patch.name, finishTask);
      await pending;
      pendingRequestIds.delete(key);
    }
    catch (e) { hideTask(); session.setFeedback({ error: reason(String(e)) }, item.game.instance_id); }
  }
  function viewPreview() {
    if (!patch || busy) return;
    setPreview(null);
    void session.work("正在检查文件变更…", () => previewRpc(item.game.instance_id, patch.selection_id ? null : patch.release_id, patch.selection_id ?? null), setPreview, item.game.instance_id);
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
      <Row><Action disabled={!!busy} onClick={() => void session.refresh()}>重新查找补丁</Action></Row>
      {item.discovery?.diagnostics.map((diagnostic, index) => <Row key={index}><Status warning>{diagnostic.message || reason(diagnostic.code)}{diagnostic.detail && diagnostic.detail !== diagnostic.code ? ` · ${reason(diagnostic.detail)}` : ""}</Status></Row>)}
    </PanelSection>
    {(!!item.patches.length || !item.discovery?.candidates.length) && <PanelSection title="匹配到的补丁">{item.patches.length ? item.patches.map(p => <PanelSectionRow key={p.selection_id ?? p.release_id}>
      <div className="patch-patch">
        <div className="patch-patch-name">{p.name}</div>
        <div className="patch-patch-source">匹配源：{sourceInfo(p, report?.sources)}</div>
        <div className="patch-actions">
          <Button onClick={() => { chooseRelease(p.selection_id ?? p.release_id); navigatePanel("patches"); }}>查看补丁</Button>
          <Button onClick={() => openSource(p.source_page)}>打开来源网页 ↗</Button>
        </div>
      </div>
    </PanelSectionRow>) : <Row><Status>{discoveryStatus(item.discovery?.status)}</Status><Action onClick={() => navigatePanel("sources")}>管理订阅源</Action></Row>}</PanelSection>}
    {entries.some(entry => entry.kind === "discovery") && <PanelSection title="来源候选">{entries.filter(entry => entry.kind === "discovery").map(entry => entry.kind === "discovery" && <Row key={entry.key}>
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
  const patchCandidate = item.discovery?.candidates.find(value => value.candidate_id === patch?.candidate_id);
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
        {patch.support_level === "publisher_exe" && patchCandidate && <>
          <InfoRow label="补丁语言">{candidateLanguage(patchCandidate)}</InfoRow>
          <InfoRow label="补丁版本">{patchCandidate.version_raw || "请核对来源说明"}</InfoRow>
          <InfoRow label="适用条件" below>{candidateConditions(patchCandidate)}</InfoRow>
        </>}
        {patch.blockers.map((b, i) => <Row key={i}><Status warning>{reason(b)}</Status></Row>)}
        <Row><Action onClick={() => openSource(patch.source_page)}>打开来源网页 ↗</Action></Row>
      </PanelSection>
      <PanelSection title={installed ? "重新安装" : "安装补丁"}>
        <Row><Status>安装时请关闭游戏并暂停 Steam 更新。</Status></Row>
        {blocked && <Row><Status warning>{blocked}</Status></Row>}
        <Row><Action disabled={!!busy || tasks.isBusy()} onClick={viewPreview}>查看文件变更（只读）</Action></Row>
        {preview && <Row><Status>{preview.changes.length ? preview.changes.map(change => `${change.action}：${change.path}`).join("；") : preview.blockers.length ? preview.blockers.map(reason).join("；") : "暂无文件变更。"}</Status></Row>}
        <Row><Action disabled={!!busy || !!blocked || tasks.isBusy()} onClick={() => void beginInstall()}>{installed ? "重新安装补丁" : "安装补丁"}</Action></Row>
        {patch.support_level === "managed_zip" && <Row><Action onClick={() => setManual(value => !value)}>使用本地 ZIP（高级）{manual ? " ▴" : " ▾"}</Action></Row>}
        {manual && patch.support_level === "managed_zip" && <><Row><TextField label="Downloads 内的 ZIP 路径" value={archive} onChange={e => setArchive(e.target.value)} /></Row>
          <Row><Action disabled={!archive.trim() || manualBlocked || !!busy} onClick={() => void beginInstall(archive)}>使用本地 ZIP {installed ? "重新安装" : "安装"}</Action></Row></>}
      </PanelSection>
    </>}
  </div>;
  const records = <div className="patch-page" ref={activePanel === "records" ? contentRef : undefined} onScroll={e => panelScroll.current.records = e.currentTarget.scrollTop}>
    {common}
    {unfinished.length > 0 && <PanelSection title="未完成的操作"><Row><Status warning>以下操作需要恢复；请关闭相关游戏并暂停更新。</Status></Row>
      {unfinished.map(tx => <Row key={tx.transaction_id}><Status>{transactionGameName(report?.installs ?? [], tx.instance_id)} · {workflowState(tx.state)}</Status></Row>)}
      <Row><Action disabled={!!busy} onClick={() => void session.work("正在恢复…", recover, values => {
        values.forEach(record); refreshAfterMutation("恢复已执行，请检查各项操作结果。", undefined, item.game.instance_id);
      }, item.game.instance_id)}>修复未完成的操作</Action></Row>
    </PanelSection>}
    <PanelSection title="当前游戏记录">{owned.length ? owned.map(tx => <Row key={tx.transaction_id}><Status>{tx.selection_id ?? tx.release_id ?? tx.content_id ?? tx.transaction_id} · {workflowState(tx.state)}{tx.error ? ` · ${reason(tx.error)}` : ""}</Status>
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
    <PanelSection><Row><Action disabled={busy} onClick={() => void session.work("正在刷新补丁来源…", () => readSnapshot(true), session.acceptSnapshot)}>刷新来源</Action></Row></PanelSection>
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
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { busy } = state;
  const { error, note } = feedbackFor(state);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let live = true;
    getSettings().then(value => { if (live) setSettings(value); }).catch(() => { if (live) setFailed(true); });
    return () => { live = false; };
  }, []);
  const toggle = (enabled: boolean) => void session.work(enabled ? "正在开启自动清理…" : "正在关闭自动清理…",
    () => setAutoCleanup(enabled),
    value => { setSettings(value); session.update({ note: autoCleanupNote(value.auto_cleanup) }); });
  const clean = () => {
    void session.work("正在清理下载缓存…", clearCache, value => {
        session.update({ note: cacheClearNote(value.removed_bytes, value.performed) });
      });
  };
  return <div className="patch-page">
    <div>{busy && <Status>{busy}</Status>}{error && <Status warning>{error}</Status>}{note && <Status>{note}</Status>}</div>
    <Status>全局设置，对所有游戏生效。</Status>
    <PanelSection title="下载缓存">
      {settings ? <>
        <Row><ToggleField label="安装成功后自动删除下载缓存" checked={settings.auto_cleanup} disabled={!!busy} onChange={toggle} /></Row>
        <InfoRow label="当前占用">{cacheSummary()}</InfoRow>
        <Row><Status>{settings.auto_cleanup ? "安装成功后删除本次下载的补丁包。" : "缓存将保留直至手动清理。"}</Status></Row>
        <Row><Action disabled={!!busy} onClick={clean}>清理下载缓存</Action></Row>
      </> : <Row><Status warning={failed}>{failed ? "读取缓存设置失败。" : "正在读取缓存设置…"}</Status></Row>}
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
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { report, busy } = state;
  const { error, note } = feedbackFor(state);
  const [rule, setRule] = useState("");
  return <><div>{busy && <Status>{busy}</Status>}{error && <Status warning>{error}</Status>}{note && <Status>{note}</Status>}</div>
    <SourcesPage report={report} busy={!!busy} rule={rule} setRule={setRule} changeSources={action => void session.work("正在更新补丁源…", async () => { await action(); session.invalidateDiscovery(); return readSnapshot(); }, snapshot => { session.acceptSnapshot(snapshot); setRule(""); })} />
  </>;
}

export default definePlugin(() => {
  routerHook.addRoute(route, Manager);
  return { name: "Steam 补丁管理", titleView: <div>Steam 补丁管理</div>, content: <QuickAccess />, icon: <FaPuzzlePiece />,
    onDismount() { tasks.dismount(); hideTask(); routerHook.removeRoute(route); },
  };
});
