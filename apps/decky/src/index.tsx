import { ScriptSourceEditor } from "./script-source-editor";
import { scriptSourceError } from "./script-source-error";
import type { AddSourceResult, ScriptSourceDocument } from "../../../packages/contracts/generated/protocol";
import { callable, definePlugin, routerHook } from "@decky/api";
import { AppUpdatePanel, startAppUpdateNotifications, waitForAppUpdate } from './app-update';
import { Button, ButtonItem, DropdownItem, Field, Focusable, Navigation, PanelSection, PanelSectionRow, SidebarNavigation, TextField, ToggleField, showModal, useParams } from "@decky/ui";
import { FaGamepad } from "react-icons/fa";
import brandLogo from "../../../docs/images/branding/logo.png";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ComponentProps, type ReactNode } from "react";
import { downloadFileName, candidateMatched, displayPatches, filterInstalls, gameDisplayName, installSummary, patchState, reason, sourceCoverageText, sourceIntervalOptions, sourceLastAttemptText, sourceLastSuccessText, sourceNextDueText, workflowState, type Filter, type Install, type Patch, type Preview, type ProjectInstancesResult, type ScanResult, type SourceCandidate, type SourceSubscription, type Transaction } from "@patchhub/ui";
import { getListPreferences, setListPreferences, subscribeListPreferences } from "./preferences";
import { gamePresentation, matchesGameQuery, transactionGameName } from "./steam";
import { installationResult } from "@patchhub/ui";
import { installInput, manualDownloadInput, manualDownloadRequestKey, primaryActionBlockers, primaryInstallAction } from "./actions";
import { createManagerSession, feedbackFor, needsRecovery } from "./session";
import { sourceCardSummary, sourceTaskStatusText, sourceTaskTargets, sourceTaskTerminal, type SourceTaskApi, type SourceTaskSnapshot } from "./source-tasks";
import { decodeInstance, encodeInstance, settingsPage, settingsRoutes } from "./route";
import { backupSummary, backupClearNote, type BackupInventory, type BackupCleanupResult, autoCleanupNote, cacheClearNote, cacheSummary, type CacheInventory, type CacheStatus, type CleanupResult, type Settings } from "./cache";
import { tasks, taskIsTerminal, type TaskSnapshot } from "./tasks";
import { TaskProgressView, TaskResultView } from "./task-progress-view";
import { candidateConditions, candidateLanguage, candidateSource, safeSourceUrl } from "./discovery";

const scan = callable<[], ScanResult>("scan");
const previewRpc = callable<[string, string | null, string | null], Preview>("preview");
const projectInstancesRpc = callable<[string[]], ProjectInstancesResult>("project_instances");
const listSourcesRpc = callable<[], SourceSubscription[]>("list_sources");
const setSourceUpdateIntervalRpc = callable<[string, number], { interval_hours: number }>("set_source_update_interval");
const updateSourceRpc = callable<[string, boolean], SourceTaskSnapshot>("update_source");
const matchGamesRpc = callable<[string[] | null, string[], boolean], SourceTaskSnapshot>("match_games");
const rematchGameRpc = callable<[string | null, number | null, string[]], SourceTaskSnapshot>("rematch_game");
const sourceTaskStatusRpc = callable<[], SourceTaskSnapshot[]>("source_task_status");
const cancelSourceTaskRpc = callable<[string], SourceTaskSnapshot>("cancel_source_task");
const sourceApi: SourceTaskApi = {
  listSources: () => listSourcesRpc(),
  updateSource: (sourceId, force = false) => updateSourceRpc(sourceId, force),
  matchGames: (instances, nameHints, forceDetails) => matchGamesRpc(instances, nameHints, forceDetails),
  rematchGame: (instance, appid, nameHints) => rematchGameRpc(instance, appid, nameHints),
  setInterval: (sourceId, intervalHours) => setSourceUpdateIntervalRpc(sourceId, intervalHours),
  projectInstances: instanceIds => projectInstancesRpc(instanceIds),
  taskStatus: () => sourceTaskStatusRpc(),
  cancelTask: taskId => cancelSourceTaskRpc(taskId),
};
const listTransactions = callable<[], Transaction[]>("list_transactions");
const recover = callable<[], Transaction[]>("recover");
const setSourceEnabled = callable<[string, boolean], SourceSubscription[]>("set_source_enabled");
const setSourceName = callable<[string, string], SourceSubscription[]>("set_source_name");
const setGameSource = callable<[number, string, boolean], { appid: number }>("set_game_source");
const addSource = callable<[string], SourceSubscription[]>("add_source");
const getScriptSource = callable<[string], ScriptSourceDocument>("get_script_source");
const importScriptSource = callable<[string, number, string, boolean], AddSourceResult>("import_script_source");
const sourceRuleExample = JSON.stringify({
  id: "my-publisher", name: "发行商补丁站", adapter: "table_html", enabled: true,
  pages: ["https://patches.vendor.example/patches/"], page_hosts: ["patches.vendor.example"],
  download_hosts: ["downloads.vendor.example"], root_layout: "game_root", table_id: "patches",
  columns: { title: 0, appid: 1, download: 2, date: 3, language: 4 },
}, null, 2);
const backupStatus = callable<[], BackupInventory>("backup_status");
const cleanupBackups = callable<[], BackupCleanupResult>("cleanup_backups");
const cacheStatus = callable<[], CacheInventory>("cache_status");
const clearCache = callable<[], CacheStatus>("clear_cache");
const getSettings = callable<[], Settings>("get_settings");
const setAutoCleanup = callable<[boolean], Settings>("set_auto_cleanup");

async function readSnapshot() {
  await waitForAppUpdate();
  const report = await scan();
  let transactions = session.getSnapshot().transactions;
  try { transactions = await listTransactions(); }
  catch { /* Keep the last transaction snapshot during a transient read failure. */ }
  return { report, transactions };
}
const session = createManagerSession(readSnapshot, sourceApi);
const route = "/patchhub/:instance/:panel?";
const sourceRoute = "sources";
const panelKeys = ["patches", "records", "sources"] as const;
const filterOptions: { data: Filter; label: string }[] = [
  { data: "all", label: "全部" }, { data: "matched", label: "有补丁" },
  { data: "installed", label: "已安装补丁" }, { data: "attention", label: "需要处理" },
];
let listScroll = 0;
let lastGame: string | null = null;
// Reuse a request identity when the start RPC reply is lost and the user retries.
const pendingRequestIds = new Map<string, string>();

function openManager(instance: string) {
  Navigation.Navigate(`/patchhub/${encodeInstance(instance)}`);
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
function QueryProgress({ task }: { task: SourceTaskSnapshot }) {
  return <p className="patch-query" role="status">{task.state === "running" && <span className="patch-query-spinner" aria-hidden="true" />}{sourceTaskStatusText(task)}</p>;
}
function Row({ children }: { children: ReactNode }) { return <PanelSectionRow>{children}</PanelSectionRow>; }
// Native left-label / right-value row. Long values opt into a stacked layout.
function InfoRow({ label, children, below = false }: { label: ReactNode; children: ReactNode; below?: boolean }) {
  return <PanelSectionRow><Field label={label} childrenLayout={below ? "below" : "inline"} spacingBetweenLabelAndChild="none"><span className="patch-kv-value">{children}</span></Field></PanelSectionRow>;
}
function sourceInfo(patch: Patch, sources: SourceSubscription[] | null = []) {
  const subscription = sources?.find(source => source.id === patch.source_id);
  return subscription?.name || "匹配源未知";
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
  .patch-query { display:flex; align-items:center; gap:8px; }
  .patch-query-spinner { width:14px; height:14px; flex:none; border:2px solid #687a8c; border-top-color:#fff; border-radius:50%; animation:patch-spin .8s linear infinite; }
  @keyframes patch-spin { to { transform:rotate(360deg); } }
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
  .patch-source-card { margin:0 0 16px; border:1px solid #627387; border-radius:8px; background:#202c39; min-width:0; }
  .patch-source-card .patch-source-heading { box-sizing:border-box; width:100%; min-width:0; height:auto; margin:0; padding:14px 16px; border-radius:7px; border:0; background:#2b3b4d; color:#f3f6f8; box-shadow:none; font-size:16px; line-height:1.35; text-align:left; white-space:normal; }
  .patch-source-card .patch-source-heading:hover { background:#344a60; }
  .patch-source-card .patch-source-heading:focus-visible, .patch-source-card .patch-source-heading.gpfocus { background:#3b536b; outline:3px solid #fff; outline-offset:2px; }
  .patch-source-heading-main { display:flex; align-items:center; gap:12px; min-width:0; }
  .patch-source-heading-name { flex:1; min-width:0; font-weight:700; overflow-wrap:anywhere; }
  .patch-source-heading-arrow { flex:none; }
  .patch-source-heading-state, .patch-source-heading-task { display:block; margin-top:6px; font-size:13px; line-height:1.4; overflow-wrap:anywhere; }
  .patch-source-heading-state { opacity:.8; }
  .patch-source-body { padding:4px 12px 12px; border-top:1px solid #627387; min-width:0; }

  @media (max-width:600px) { .patch-page { padding:0 12px 40px; } }
`;

function matchSummary(item: Install): string {
  const matching = Object.values(session.getSnapshot().sourceTasks).some(task =>
    task.state === "running" && sourceTaskTargets(task, item.game.instance_id, item.game.appid));
  return installSummary(item, matching);
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
    {!!candidate.version_evidence?.length && candidate.warnings?.includes("version_conflict") && <InfoRow label="页面版本描述" below>{candidate.version_evidence.filter(value => typeof value === "string").join("、")}</InfoRow>}
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
  const { report, busy, sourceTasks } = state;
  const { error, note } = feedbackFor(state);
  const { filter, query } = useSyncExternalStore(subscribeListPreferences, getListPreferences);
  const listRef = useRef<HTMLDivElement>(null);
  const running = Object.values(sourceTasks).filter(task => !sourceTaskTerminal(task.state));
  useEffect(() => {
    void session.ensureSnapshot();
    void tasks.resume().catch(() => {});
    return session.startSourcePolling();
  }, []);
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
    <div className="patch-qam-controls"><PanelSection title="PatchHub">
      <Row><TextField label="搜索游戏名称或编号" value={query} onChange={e => setListPreferences({ query: e.target.value })} /></Row>
      <Row><DropdownItem label="筛选" layout="below" rgOptions={filterOptions} selectedOption={filter} onChange={option => setListPreferences({ filter: option.data })} /></Row>
      {busy && <Row><Status>{busy}</Status></Row>}{error && <Row><Status warning>{error}</Status></Row>}
      {running.map(task => <Row key={task.task_id}><QueryProgress task={task} /><Action onClick={() => void session.cancelSourceTask(task.task_id)}>取消</Action></Row>)}
      {note && <Row><Status>{note}</Status></Row>}
    </PanelSection></div>
    <div className="patch-qam-list" ref={listRef} onScroll={e => { listScroll = e.currentTarget.scrollTop; }}>
      <PanelSection title={`游戏列表 · ${shown.length}`}>
        {shown.map(item => <Row key={item.game.instance_id}><GameButton item={item} /></Row>)}
        {report && report.installs.length > 0 && !shown.length && <Row><Status>没有符合条件的游戏。</Status><Action onClick={() => setListPreferences({ filter: "all", query: "" })}>清除筛选</Action></Row>}
        {report && !report.installs.length && <Row><Status>未找到 Steam 游戏。请检查游戏库或 SD 卡。</Status></Row>}
        {report?.diagnostics.filter(d => d.code !== "library_unavailable" && !/^(source_|discovery_|invalid_source_)/.test(d.code)).map((d, i) => <Row key={i}><Status warning>{reason(d.code)}{d.path ? ` · ${d.path}` : ""}</Status></Row>)}
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
  const { report, transactions, busy, sourceTasks } = state;
  const { error, note } = feedbackFor(state, item.game.instance_id);
  // The decky SidebarNavigation is route-driven: it renders the page whose
  // `route` matches the URL and switches by replacing the URL. Keep one
  // sub-route per panel so selection and content follow the URL.
  const activePanel = panel && (panelKeys as readonly string[]).includes(panel) ? panel : "patches";
  const base = `/patchhub/${seg}`;
  const navigate = useRef<((route: string) => void) | null>(null);
  const entries = displayPatches(item);
  const [selectedDownload, setSelectedDownload] = useState("");
  const [selectedKey, setSelectedKey] = useState(entries[0]?.key ?? "");
  const [archive, setArchive] = useState("");
  const [preview, setPreview] = useState<Preview | null>(null);
  const [manual, setManual] = useState(false);
  const [manualUrl, setManualUrl] = useState("");
  const [manualLinkExpanded, setManualLinkExpanded] = useState(false);
  const [pathsExpanded, setPathsExpanded] = useState(false);
  const panelScroll = useRef<Record<string, number>>({});
  const contentRef = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);
  const game = gamePresentation(item.game.appid, gameDisplayName(item.game)).name;
  const selected = entries.find(entry => entry.key === selectedKey);
  const patch = selected?.kind === "installable" ? selected.choices.find(p => (p.selection_id ?? p.release_id) === selectedDownload) ?? selected.patch : undefined;
  const candidate = selected?.kind === "discovery" ? selected.candidate : undefined;
  const identity = patch?.selection_id ?? patch?.release_id ?? "";
  useEffect(() => { setPreview(null); setArchive(""); setManual(false); setManualUrl(""); setManualLinkExpanded(false); }, [item.game.instance_id, selectedKey, identity]);
  const primaryAction = primaryInstallAction(patch, !!archive.trim());
  const primaryAvailable = primaryAction.available;
  const actionBlockers = primaryActionBlockers(patch);
  const owned = transactions.filter(t => t.instance_id === item.game.instance_id).reverse();
  const installed = primaryAction.label === "overwrite";
  const unfinished = transactions.filter(needsRecovery);
  useEffect(() => {
    if (!entries.some(entry => entry.key === selectedKey)) {
      setSelectedKey(entries[0]?.key ?? ""); setArchive("");
    }
  }, [item.patches, item.discovery, selectedKey]);
  useEffect(() => {
    mounted.current = true;
    void session.ensureSnapshot();
    const stopPolling = session.startSourcePolling();
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
      stopPolling();
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
  function chooseRelease(next: string) { setSelectedKey(next); setSelectedDownload(""); setArchive(""); setPreview(null); }
  function changeGameSource(sourceId: string, subscribed: boolean) {
    void session.work("正在更新此游戏的订阅源…", async () => {
      await setGameSource(item.game.appid, sourceId, subscribed);
      session.invalidateDiscovery(sourceId, item.game.appid);
      await session.refreshSources();
    }, () => {
      session.setFeedback({ note: "此游戏的订阅源已更新。" }, item.game.instance_id);
    }, item.game.instance_id);
  }
  function finishTask(task: TaskSnapshot) {
    const instance = task.instance_id ?? item.game.instance_id;
    if (task.result?.state === "needs_recovery" || task.result?.recovery?.required) {
      if (task.result?.transaction_id) record(task.result);
      const diagnostic = task.result?.recovery?.code ?? task.result?.error ?? task.error ?? "needs_recovery";
      refreshAfterMutation(undefined, `安装仍需恢复：${reason(diagnostic)}${task.result?.recovery?.detail ? ` · ${task.result.recovery.detail}` : ""}`, instance);
    } else if (task.state === "completed") {
      const tx = task.result;
      if (tx?.transaction_id) record(tx);
      const cleanup = task.result?.cleanup;
      const message = [tx ? installationResult(tx) : "安装完成，正在核对文件状态。",
        cleanup ? cacheClearNote(cleanup) : ""].filter(Boolean).join(" ");
      refreshAfterMutation(message, undefined, instance);
    } else if (task.state === "failed") {
      const error = task.result
        ? installationResult(task.result)
        : `${reason(task.error ?? "任务失败，请检查操作记录。")}${task.error_detail ? ` · ${task.error_detail}` : ""}`;
      session.setFeedback({ error }, instance);
      refreshAfterMutation(undefined, error, instance);
    } else if (task.state === "cancelled") {
      refreshAfterMutation("任务已安全取消。", undefined, instance);
    }
  }
  async function beginInstall(localArchive?: string) {
    const input = installInput(patch, localArchive);
    if (input === null || !patch || busy || tasks.isBusy()) return;
    try {
      const operation = primaryAction.operation;
      const key = `${item.game.instance_id}\0${identity}\0${operation}\0${input}`;
      const requestId = pendingRequestIds.get(key) ?? crypto.randomUUID();
      pendingRequestIds.set(key, requestId);
      hideTask();
      const pending = tasks.startInstall(item.game.instance_id, patch.selection_id ? null : patch.release_id, patch.selection_id ?? null, operation, requestId, input);
      showTask(game, patch.name, finishTask);
      await pending;
      pendingRequestIds.delete(key);
    }
    catch (e) { hideTask(); session.setFeedback({ error: reason(String(e)) }, item.game.instance_id); }
  }
  async function copyPath(path: string) {
    try { await navigator.clipboard.writeText(path); }
    catch (error) { session.setFeedback({ error: `复制失败：${String(error)}` }, item.game.instance_id); }
  }
  async function beginManualInstall() {
    const input = manualDownloadInput(candidate, manualUrl);
    if (!input || !candidate || busy || tasks.isBusy()) return;
    const key = manualDownloadRequestKey(item.game.instance_id, input);
    const requestId = pendingRequestIds.get(key) ?? crypto.randomUUID();
    pendingRequestIds.set(key, requestId);
    try {
      hideTask();
      const pending = tasks.startInstall(item.game.instance_id, null, null, "overwrite", requestId, "", input);
      showTask(game, candidate.patch_title, finishTask);
      await pending;
      pendingRequestIds.delete(key);
    } catch (error) { hideTask(); session.setFeedback({ error: reason(String(error)) }, item.game.instance_id); }
  }
  function viewPreview() {
    if (!patch || busy) return;
    setPreview(null);
    void session.work("正在检查文件变更…", () => previewRpc(item.game.instance_id, patch.selection_id ? null : patch.release_id, patch.selection_id ?? null), setPreview, item.game.instance_id);
  }
  const gameTask = Object.values(sourceTasks).find(task => !sourceTaskTerminal(task.state)
    && sourceTaskTargets(task, item.game.instance_id, item.game.appid));
  const matchStatus = item.discovery?.status;
  function rematch() {
    const hints = [item.game.localized_name, item.game.name, game].map(value => value?.trim() ?? "").filter(Boolean);
    void session.rematchGame(item.game.instance_id, item.game.appid, [...new Set(hints)].slice(0, 3));
  }
  const common = <>{busy && <Status>{busy}</Status>}{error && <Status warning>{error}</Status>}{note && <Status>{note}</Status>}
    </>;
  const selection = entries.length > 1 ? <PanelSection title="选择补丁"><Row><DropdownItem label="补丁" layout="below"
    rgOptions={entries.map(entry => ({ data: entry.key, label: entry.kind === "installable" ? entry.patch.name : `${entry.candidate.patch_title} · ${entry.candidate.version_raw || "版本待确认"} · ${candidateLanguage(entry.candidate)} · ${candidateSource(entry.candidate)}` }))}
    selectedOption={selectedKey} onChange={option => chooseRelease(option.data)} /></Row></PanelSection> : null;
  const patchCandidate = item.discovery?.candidates.find(value => value.candidate_id === patch?.candidate_id);
  const patches = <div className="patch-page" ref={activePanel === "patches" ? contentRef : undefined} onScroll={e => panelScroll.current.patches = e.currentTarget.scrollTop}>
    {common}
    <PanelSection title="游戏信息">
      <InfoRow label="游戏">{game}</InfoRow>
      <InfoRow label="AppID">{item.game.appid}</InfoRow>
      <InfoRow label="Build">{item.game.buildid ?? "未知"}</InfoRow>
      <Row><Action onClick={() => setPathsExpanded(value => !value)}>安装路径与游戏库{pathsExpanded ? " ▴" : " ▾"}</Action></Row>
      {pathsExpanded && <>
        <InfoRow label="安装路径" below>{item.game.install_path}</InfoRow>
        <Row><Action onClick={() => void copyPath(item.game.install_path)}>复制安装路径</Action></Row>
        <InfoRow label="游戏库" below>{item.game.library_path}</InfoRow>
        <Row><Action onClick={() => void copyPath(item.game.library_path)}>复制游戏库路径</Action></Row>
      </>}
      {owned.some(needsRecovery) && <Row><Status warning>有未完成的操作需要处理。</Status><Action onClick={() => navigatePanel("records")}>查看操作记录</Action></Row>}
    </PanelSection>
    <PanelSection title="查找补丁">
      {gameTask && <Row><QueryProgress task={gameTask} /><Action onClick={() => void session.cancelSourceTask(gameTask.task_id)}>取消查询</Action></Row>}
      <Row><Action disabled={!!busy || !!gameTask} onClick={rematch}>重新查询</Action></Row>
    </PanelSection>
    {selection}
    {candidate ? <><CandidateDetails candidate={candidate} />
      <PanelSection title="安装补丁">
        <Row><Status>未找到下载链接</Status></Row>
        <Row><Action onClick={() => setManualLinkExpanded(value => !value)}>手动输入下载链接{manualLinkExpanded ? " ▴" : " ▾"}</Action></Row>
        {manualLinkExpanded && <>
          <Row><TextField label="ZIP / EXE 下载直链" value={manualUrl} onChange={e => setManualUrl(e.target.value)} /></Row>
          <Row><Status>请输入可直接下载 ZIP / EXE 文件的 HTTP 或 HTTPS 链接；不支持网盘分享页和提取码。安装时请关闭游戏并暂停 Steam 更新。</Status></Row>
          <Row><Action disabled={!manualDownloadInput(candidate, manualUrl) || !!busy || tasks.isBusy()} onClick={() => void beginManualInstall()}>下载安装补丁</Action></Row>
        </>}
      </PanelSection></> : !patch ? <PanelSection><Row><Status>暂无补丁结果</Status><Action onClick={() => navigatePanel("sources")}>游戏订阅</Action></Row></PanelSection> : <>
      {selected?.kind === "installable" && selected.choices.length > 1 && <PanelSection title="下载文件与镜像"><Row><DropdownItem label="选择下载项" layout="below" selectedOption={patch.selection_id ?? patch.release_id} rgOptions={selected.choices.map(p => ({ data: p.selection_id ?? p.release_id, label: `${p.download_label || "来源下载"} · ${p.download_url ? downloadFileName(p.download_url) : p.name}` }))} onChange={option => { setSelectedDownload(option.data); setPreview(null); setArchive(""); }} /></Row></PanelSection>}
      <PanelSection title="选择补丁"><InfoRow label="补丁">{patch.name}</InfoRow>
        <InfoRow label="匹配源">{sourceInfo(patch, report?.sources)}</InfoRow>
      </PanelSection>
      <PanelSection title="补丁详情">
        <InfoRow label="当前状态">{patchState(patch.detection.status)}</InfoRow>
        <InfoRow label="兼容性">{workflowState(patch.compatibility)}</InfoRow>
        <InfoRow label="安装方式">{workflowState(patch.support_level)}</InfoRow>
        {(patch.support_level === "publisher_exe" || patch.compatibility === "manual_selection_required") && patchCandidate && <>
          <InfoRow label="补丁语言">{candidateLanguage(patchCandidate)}</InfoRow>
          <InfoRow label="补丁版本">{patchCandidate.version_raw || "请核对来源说明"}</InfoRow>
          <InfoRow label="适用条件" below>{candidateConditions(patchCandidate)}</InfoRow>
        </>}
        {patch.download_url && <>
          <InfoRow label="下载文件" below>{downloadFileName(patch.download_url)}</InfoRow>
          {patch.download_label && <InfoRow label="下载项">{patch.download_label}</InfoRow>}
          <InfoRow label="下载地址" below>{patch.download_url}</InfoRow>
        </>}
        <Row><Action onClick={() => openSource(patch.source_page)}>打开来源网页 ↗</Action></Row>
      </PanelSection>
      <PanelSection title={installed ? "覆盖安装" : "安装补丁"}>
        <Row><Status>安装时请关闭游戏并暂停 Steam 更新。</Status></Row>
        {actionBlockers.map((blocker, i) => <Row key={`action-${blocker.code}-${i}`}><Status warning>{reason(blocker.code)}{blocker.detail ? ` · ${blocker.detail}` : ""}</Status></Row>)}
        <Row><Action disabled={!!busy || tasks.isBusy()} onClick={viewPreview}>查看文件变更（只读）</Action></Row>
        {preview && <Row><Status>{preview.changes.length ? preview.changes.map(change => `${change.action}：${change.path}`).join("；") : preview.blockers.length ? preview.blockers.map(reason).join("；") : "暂无文件变更。"}</Status></Row>}
        <Row><Action disabled={!primaryAvailable || !!busy || tasks.isBusy()} onClick={() => void beginInstall()}>{primaryAction.label === "overwrite" ? "覆盖安装补丁" : "安装补丁"}</Action></Row>
        {!patch.download_url && !primaryAvailable && <Row><Status>未找到下载链接</Status></Row>}
        {patch.actions.local_archive.available && <Row><Action onClick={() => setManual(value => !value)}>导入本地 ZIP（可选）{manual ? " ▴" : " ▾"}</Action></Row>}
        {(manual && patch.actions.local_archive.available) && <><Row><TextField label="Downloads 内的 ZIP 路径" value={archive} onChange={e => setArchive(e.target.value)} /></Row>
          <Row><Action disabled={!archive.trim() || !patch.actions.local_archive.available || !!busy || tasks.isBusy()} onClick={() => void beginInstall(archive)}>使用本地 ZIP {primaryAction.label === "overwrite" ? "覆盖安装" : "安装"}</Action></Row></>}
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
    {matchStatus === "fallback_match" && <Row><Status>普通来源未命中，补丁由 HGamer 补查得到。</Status></Row>}
    {matchStatus === "no_match" && <Row><Status>已启用来源中未找到补丁。</Status></Row>}
    {item.sources?.length ? item.sources.map(source => <PanelSection key={source.id} title={report?.sources?.find(current => current.id === source.id)?.name ?? source.name}>
      <Row><ToggleField label="用于此游戏" checked={source.subscribed} disabled={!!busy}
        onChange={subscribed => changeGameSource(source.id, subscribed)} /></Row>
      <InfoRow label="游戏匹配">{source.matched || item.discovery?.candidates.some(candidate => candidate.source_id === source.id && candidateMatched(candidate))
        ? "已找到此游戏的补丁"
        : source.role === "fallback_indexer" ? "本次未在此来源找到候选" : "未在此来源找到候选"}</InfoRow>
      {source.role === "fallback_indexer" && <Row><Status>补查来源：仅在重新匹配时按游戏查询，不参与自动更新。</Status></Row>}
      {!source.enabled && <Row><Status warning>此源已在全局设置中停用，当前不会参与匹配。</Status></Row>}
      <Row><Action onClick={() => openSource(source.pages[0])}>打开来源网页 ↗</Action></Row>
    </PanelSection>) : <PanelSection><Row><Status>尚未配置订阅源，请从主面板的全局设置添加。</Status></Row></PanelSection>}
  </div>;
  const navPages: ComponentProps<typeof SidebarNavigation>["pages"] = [
    { identifier: `${base}/patches`, route: `${base}/patches`, title: "补丁管理", visible: true, content: patches },
    { identifier: `${base}/records`, route: `${base}/records`, title: "操作记录", visible: true, content: records },
    "separator", { identifier: `${base}/sources`, route: `${base}/sources`, title: "游戏订阅", visible: true, content: sources },
  ];
  // The decky SidebarNavigation resolves the visible page from page.route
  // against the URL and switches by replacing the URL. `fnSetNavigateToPage`
  // receives the internal switcher, so panel jumps reuse it instead of pushing
  // history, and `disableRouteReporting` leaves Steam's own router untouched.
  // `showTitle` renders the non-focusable two-line header at the top of the
  // page list (native PagedSettings only renders `title` when it is true).
  const navigation = { title: <div className="patch-nav-header"><span className="patch-nav-tool">PatchHub</span><span className="patch-nav-game" title={game}>{game}</span></div>, showTitle: true, page: `${base}/${activePanel}`, pages: navPages,
    fnSetNavigateToPage: (setter: (route: string) => void) => { navigate.current = setter; },
    disableRouteReporting: true } as unknown as ComponentProps<typeof SidebarNavigation>;
  return <SidebarNavigation {...navigation} />;
}

function SourceCard({ source, busy, expanded, onToggle, task, changeSources, onUpdateSource, onCancelTask, onInterval, onEditScript }: {
  source: SourceSubscription; busy: boolean; expanded: boolean; onToggle: () => void; task?: SourceTaskSnapshot;
  changeSources: (action: () => Promise<SourceSubscription[]>, affectsMatching?: boolean) => void;
  onUpdateSource: (sourceId: string, force: boolean) => void;
  onCancelTask: (taskId: string) => void;
  onInterval: (sourceId: string, hours: number) => void;
  onEditScript: (sourceId?: string) => void;
}) {
  const publisher = source.role !== "fallback_indexer";
  const [name, setName] = useState(source.name);
  useEffect(() => setName(source.name), [source.name]);
  const summary = sourceCardSummary(source, task);
  const contentId = `patch-source-${encodeURIComponent(source.id)}`;
  return <div className="patch-source-card">
    <Button className="patch-source-heading" aria-expanded={expanded} aria-controls={contentId} onClick={onToggle}>
      <span className="patch-source-heading-main"><span className="patch-source-heading-name">{source.name}</span>
        <span className="patch-source-heading-arrow" aria-hidden="true">{expanded ? "▾" : "▸"}</span></span>
      <span className="patch-source-heading-state">{source.enabled ? "已启用" : "已停用"}</span>
      {summary && <span className={`patch-source-heading-task${summary.warning ? " patch-warning" : ""}`}>{summary.text}</span>}
    </Button>
    {expanded && <div id={contentId} className="patch-source-body" role="group" aria-label={source.name}>
      <Row><TextField label="来源名称" value={name} disabled={busy} onChange={e => setName(e.target.value)} /></Row>
      <Row><Action disabled={busy || !name.trim() || name.trim() === source.name}
        onClick={() => changeSources(() => setSourceName(source.id, name), false)}>保存名称</Action></Row>
      {source.script && <>
        <InfoRow label="脚本" below>Lua · API {source.script.api_version} · SHA-256 {source.script.sha256}</InfoRow>
        <Row><Action disabled={busy} onClick={() => onEditScript(source.id)}>编辑脚本</Action></Row>
      </>}
      <InfoRow label="站点" below>{source.pages.join(" · ")}</InfoRow>
      {publisher && <>
        <InfoRow label="索引条目">{source.entry_count} 条 · 修订 {source.revision}</InfoRow>
        <InfoRow label="索引覆盖">{sourceCoverageText(source)}</InfoRow>
        <InfoRow label="最近成功">{sourceLastSuccessText(source)}</InfoRow>
        <InfoRow label="自动更新">{sourceNextDueText(source)}</InfoRow>
      </>}
      <InfoRow label="最近尝试">{sourceLastAttemptText(source)}</InfoRow>
      <Row><ToggleField label={`启用 ${source.name}`} checked={source.enabled} disabled={busy}
        onChange={enabled => changeSources(() => setSourceEnabled(source.id, enabled))} /></Row>
      {publisher ? <>
        <Row><DropdownItem label="自动更新间隔" layout="below" rgOptions={sourceIntervalOptions()}
          selectedOption={source.interval_hours} onChange={option => onInterval(source.id, option.data)} /></Row>
        {task && !sourceTaskTerminal(task.state)
          ? <Row><Status>{sourceTaskStatusText(task)}</Status><Action onClick={() => onCancelTask(task.task_id)}>取消更新</Action></Row>
          : <Row><Action disabled={busy} onClick={() => onUpdateSource(source.id, false)}>手动更新</Action></Row>}
      </> : <Row><Status>补查来源：仅对单个游戏查询，不参与定时更新，也没有更新间隔。</Status></Row>}
    </div>}
  </div>;
}

function SourcesPage({ report, busy, tasks, expandedSources, onToggleSource, rule, setRule, changeSources, onUpdateSource, onCancelTask, onInterval, onMatchGames, onEditScript }: {
  expandedSources: Record<string, boolean>; onToggleSource: (sourceId: string) => void;
  report: ScanResult | null; busy: boolean; tasks: SourceTaskSnapshot[]; rule: string; setRule: (value: string) => void;
  changeSources: (action: () => Promise<SourceSubscription[]>, affectsMatching?: boolean) => void;
  onUpdateSource: (sourceId: string, force: boolean) => void;
  onCancelTask: (taskId: string) => void;
  onInterval: (sourceId: string, hours: number) => void;
  onMatchGames: () => void;
  onEditScript: (sourceId?: string) => void;
}) {
  const [showExample, setShowExample] = useState(false);
  async function copyExample() {
    try {
      await navigator.clipboard.writeText(sourceRuleExample);
      session.setFeedback({ note: "示例已复制，请修改后添加", error: null });
    } catch (error) {
      session.setFeedback({ error: `复制失败：${String(error)}`, note: null });
    }
  }
  const sources = report?.sources ?? [];
  const publishers = sources.filter(source => source.role !== "fallback_indexer");
  const fallbacks = sources.filter(source => source.role === "fallback_indexer");
  const taskFor = (sourceId: string) => tasks.find(task => task.kind === "update_source" && task.key === sourceId);
  return <div className="patch-page">
    <Status>全局设置，对所有游戏生效。普通来源按间隔自动更新；补查来源（包括 HGamer 与脚本来源）按游戏查询，不参与索引定时更新。</Status>
    <PanelSection title="匹配游戏"><Row><Action disabled={busy} onClick={onMatchGames}>匹配全部游戏</Action></Row>
      <Row><Status>来源更新后使用已更新的索引增量匹配当前游戏库；单个游戏可在游戏详情重新匹配。</Status></Row></PanelSection>
    {!!tasks.length && <PanelSection title="来源任务">{tasks.map(task => <Row key={task.task_id}>
      <Status>{sourceTaskStatusText(task)}</Status><Action onClick={() => onCancelTask(task.task_id)}>取消</Action>
    </Row>)}</PanelSection>}
    <PanelSection title="发行商补丁源">
      {publishers.map(source => <SourceCard key={source.id} source={source} busy={busy}
        expanded={expandedSources[source.id] === true} onToggle={() => onToggleSource(source.id)} task={taskFor(source.id)}
        changeSources={changeSources} onUpdateSource={onUpdateSource} onCancelTask={onCancelTask} onInterval={onInterval} onEditScript={onEditScript} />)}
      {!publishers.length && <Row><Status>尚未配置发行商补丁源。</Status></Row>}
    </PanelSection>
    <PanelSection title="补丁索引（补查来源）">
      {fallbacks.map(source => <SourceCard key={source.id} source={source} busy={busy}
        expanded={expandedSources[source.id] === true} onToggle={() => onToggleSource(source.id)}
        changeSources={changeSources} onUpdateSource={onUpdateSource} onCancelTask={onCancelTask} onInterval={onInterval} onEditScript={onEditScript} />)}
      {!fallbacks.length && <Row><Status>尚未配置补查来源。</Status></Row>}
    </PanelSection>
    <PanelSection title="脚本来源"><Row><Action disabled={busy} onClick={() => onEditScript()}>导入脚本来源</Action></Row></PanelSection>
    <PanelSection title="添加源"><Row><TextField label="源规则 JSON（高级）" value={rule} onChange={e => setRule(e.target.value)} /></Row>
      <Row><Action disabled={busy || !rule.trim()} onClick={() => changeSources(() => addSource(rule))}>添加补丁源</Action>
        <Action aria-expanded={showExample} aria-controls="source-rule-example" onClick={() => setShowExample(value => !value)}>{showExample ? "收起规则示例" : "查看规则示例"}</Action></Row>
      {showExample && <div id="source-rule-example" role="group" aria-label="来源规则示例">
        <Row><Status>适用于表格形式的补丁列表。请替换示例 ID、名称和域名，并按实际网页调整表格 ID 和列号。列号从 0 开始；title 和 download 必填，其他列可省略。</Status></Row>
        <Row><pre style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere", margin: 0 }}>{sourceRuleExample}</pre></Row>
        <Row><Action onClick={() => void copyExample()}>复制示例</Action></Row>
      </div>}
    </PanelSection>
  </div>;
}

function GlobalCache() {
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { busy } = state;
  const { error, note } = feedbackFor(state);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [cache, setCache] = useState<CacheInventory | null>(null);
  const [cacheError, setCacheError] = useState("");
  const [backups, setBackups] = useState<BackupInventory | null>(null);
  const [backupError, setBackupError] = useState("");
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let live = true;
    getSettings().then(value => { if (live) setSettings(value); }).catch(() => { if (live) setFailed(true); });
    cacheStatus().then(value => { if (live) setCache(value); }).catch(() => { if (live) setCacheError("读取下载缓存占用失败。"); });
    backupStatus().then(value => { if (live) setBackups(value); }).catch(() => { if (live) setBackupError("读取安装备份占用失败。"); });
    return () => { live = false; };
  }, []);
  const toggle = (enabled: boolean) => void session.work(enabled ? "正在开启自动清理…" : "正在关闭自动清理…",
    () => setAutoCleanup(enabled),
    value => { setSettings(value); session.update({ note: autoCleanupNote(value.auto_cleanup) }); });
  const clean = () => {
    void session.work("正在清理下载缓存…", async () => {
      const result = await clearCache();
      try { setCache(await cacheStatus()); setCacheError(""); }
      catch { setCache(null); setCacheError("清理已返回，但刷新下载缓存占用失败，请重开页面查看。"); }
      return result;
    }, value => {
        session.update({ note: cacheClearNote(value) });
      });
  };
  const cleanBackups = () => void session.work("正在清理全部安装备份…", async () => {
    const result = await cleanupBackups();
    try { setBackups(await backupStatus()); setBackupError(""); }
    catch { setBackups(null); setBackupError("清理已返回，但刷新备份占用失败，请重开页面查看。"); }
    return result;
  }, value => session.update({ note: backupClearNote(value) }));
  return <div className="patch-page">
    <div>{busy && <Status>{busy}</Status>}{error && <Status warning>{error}</Status>}{note && <Status>{note}</Status>}</div>
    <Status>全局设置，对所有游戏生效。</Status>
    <PanelSection title="下载缓存">
      {settings ? <>
        <Row><ToggleField label="安装成功后自动删除下载缓存" checked={settings.auto_cleanup} disabled={!!busy} onChange={toggle} /></Row>
        <InfoRow label="当前占用">{cache ? cacheSummary(cache) : cacheError || "正在统计下载缓存…"}</InfoRow>
        <Row><Status>{settings.auto_cleanup ? "安装成功后删除本次下载的补丁包。" : "缓存将保留直至手动清理。"}</Status></Row>
        {cache?.diagnostics.map((failure, index) => <Row key={index}><Status warning>{failure.detail || failure.code}</Status></Row>)}
        <Row><Action disabled={!!busy} onClick={clean}>清理下载缓存</Action></Row>
      </> : <Row><Status warning={failed}>{failed ? "读取缓存设置失败。" : "正在读取缓存设置…"}</Status></Row>}
    </PanelSection>
    <PanelSection title="安装备份">
      <InfoRow label="当前占用">{backups ? backupSummary(backups) : backupError || "正在统计安装备份…"}</InfoRow>
      <Row><Status>删除所有安装备份，包括中断任务留下的备份；清理后无法用这些备份恢复原文件。</Status></Row>
      {backups?.failures.map((failure, index) => <Row key={index}><Status warning>{failure.detail || failure.code}</Status></Row>)}
      <Row><Action disabled={!!busy} onClick={cleanBackups}>清理全部安装备份</Action></Row>
    </PanelSection>
    <AppUpdatePanel />
  </div>;
}

function Manager() {
  const { instance, panel } = useParams<{ instance: string; panel?: string }>();
  const { report } = useSyncExternalStore(session.subscribe, session.getSnapshot);
  useEffect(() => { void session.ensureSnapshot(); }, []);
  const target = instance === sourceRoute ? instance : decodeInstance(instance ?? "");
  const item = report?.installs.find(i => i.game.instance_id === target);
  return <Focusable className="patch-full" onCancel={() => Navigation.NavigateBack()}><style>{css}</style>
    {instance === sourceRoute ? <SidebarNavigation showTitle disableRouteReporting title={<div className="patch-nav-header"><span className="patch-nav-tool">PatchHub</span><span className="patch-nav-game">全局设置</span></div>} page={settingsPage(panel)} pages={[
      { identifier: settingsRoutes.sources, route: settingsRoutes.sources, title: "管理订阅源", visible: true, content: <GlobalSources /> },
      "separator",
      { identifier: settingsRoutes.cache, route: settingsRoutes.cache, title: "缓存、备份与更新", visible: true, content: <GlobalCache /> },
    ]} />
      : item ? <GameManager key={instance} item={item} seg={instance!} panel={panel} />
        : <div className="patch-page"><h2>PatchHub</h2><Status>{report ? "游戏已不在当前扫描结果中。" : "正在加载游戏列表…"}</Status><Action onClick={() => Navigation.NavigateBack()}>返回游戏列表</Action></div>}
  </Focusable>;
}
function GlobalSources() {
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot);
  const { report, busy, sourceTasks } = state;
  const { error, note } = feedbackFor(state);
  const [rule, setRule] = useState("");
  const [editor, setEditor] = useState<{ document?: ScriptSourceDocument } | null>(null);
  const editorSection = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (editor) editorSection.current?.scrollIntoView({ block: "start" });
  }, [editor]);
  async function editScript(sourceId?: string) {
    if (session.getSnapshot().busy) return;
    session.update({ busy: "正在读取脚本…", busySince: Date.now() });
    try { setEditor(sourceId ? { document: await getScriptSource(sourceId) } : {}); }
    catch (failure) { session.setFeedback({ error: `读取脚本失败：${scriptSourceError(failure)}`, note: null }); }
    finally { session.update({ busy: null, busySince: null }); }
  }
  async function saveScript(ruleJson: string, apiVersion: number, code: string, replace: boolean) {
    if (session.getSnapshot().busy) throw Error("其他操作正在进行，请稍后保存");
    session.update({ busy: "正在保存脚本…", busySince: Date.now() });
    try {
      const result = await importScriptSource(ruleJson, apiVersion, code, replace);
      session.commitScriptSource(session.getSnapshot().report?.sources ?? [], result.settings_revision);
      session.setFeedback({ note: "脚本已保存，尚未执行。请更新来源或重新匹配游戏。", error: null });
      try {
        session.commitSources(await listSourcesRpc());
        await session.projectRows(session.getSnapshot().report?.installs.map(row => row.game.instance_id) ?? []);
      } catch (failure) { session.setFeedback({ error: `脚本已保存，但刷新来源或游戏状态失败：${scriptSourceError(failure)}` }); }
    } finally { session.update({ busy: null, busySince: null }); }
  }
  const tasks = Object.values(sourceTasks).filter(task => !sourceTaskTerminal(task.state));
  useEffect(() => session.startSourcePolling(), []);
  const changeSources = (action: () => Promise<SourceSubscription[]>, affectsMatching = true) => void session.work("正在更新补丁源…", async () => {
    const sources = await action();
    session.commitSources(sources);
    if (affectsMatching) session.invalidateDiscovery();
    return sources;
  }, () => setRule(""));
  return <><div>{busy && <Status>{busy}</Status>}{error && <Status warning>{error}</Status>}{note && <Status>{note}</Status>}</div>
    {editor && <div ref={editorSection}><ScriptSourceEditor document={editor.document} onSave={saveScript} onClose={() => setEditor(null)} /></div>}
    <SourcesPage report={report} busy={!!busy || !!editor} tasks={tasks} expandedSources={state.expandedSources} onToggleSource={session.toggleSourceExpanded} rule={rule} setRule={setRule} changeSources={changeSources}
      onUpdateSource={(sourceId, force) => void session.updateSource(sourceId, force)}
      onCancelTask={taskId => void session.cancelSourceTask(taskId)}
      onInterval={(sourceId, hours) => void session.setSourceInterval(sourceId, hours)}
      onMatchGames={() => void session.matchGames(null)} onEditScript={sourceId => void editScript(sourceId)} />
  </>;
}

export default definePlugin(() => {
  const stopUpdateNotifications = startAppUpdateNotifications();
  routerHook.addRoute(route, Manager);
  return { name: "PatchHub", titleView: <div>PatchHub</div>, content: <QuickAccess />, icon: <img src={brandLogo} alt="" width={24} height={24} style={{ objectFit: "contain" }} />,
    onDismount() { stopUpdateNotifications(); tasks.dismount(); hideTask(); routerHook.removeRoute(route); },
  };
});
