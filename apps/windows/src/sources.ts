import {
  candidateMatched,
  gameMatchStatusLabel,
  gameMatchStatusTone,
  installSummary,
  isExternal,
  isManaged,
  reason,
  sourcePhaseLabel,
  type Install,
  type SourceCandidate,
} from "@steam-patch/ui";

/**
 * Windows projection of the shared source/match presentation.
 *
 * All six-state labels, interval presets, coverage text, schedule text and
 * phase names come from `@steam-patch/ui` so both hosts describe the same
 * states the same way. Only the Windows-specific chip/tone adaptation and the
 * layout wording live here.
 */

export type Tone = "ok" | "warn" | "fail" | "busy" | "idle";

export { primaryInstallAction, actionBlockers, safeSourceUrl } from "@steam-patch/ui";

// Shared presentation helpers (single source of Chinese copy).
export {
  fallbackDisplayStatus,
  gameMatchStatusHint,
  gameMatchStatusLabel,
  gameMatchStatusTone,
  sourceCoverageComplete,
  sourceCoverageText,
  sourceIntervalLabel,
  sourceIntervalOptions,
  sourceLastAttemptText,
  sourceLastSuccessText,
  sourceNextDueText,
  sourcePhaseLabel,
  SOURCE_INTERVAL_PRESETS,
} from "@steam-patch/ui";

// ---- host command results -------------------------------------------------

export type { Capabilities, SourceSummary } from "../../../packages/contracts/generated/protocol";
import type {
  GameMatch,
  GameSourceSummary,
  MatchGamesResult,
  MatchProgressEvent,
  MatchUpdateEvent,
  ProjectInstancesResult,
  SetSourceIntervalResult,
  SourceProgressEvent,
  SourceSummary,
  SourceUpdateEvent,
  SourceUpdateResult,
} from "../../../packages/contracts/generated/protocol";

export type {
  GameMatch,
  MatchGamesResult,
  MatchProgressEvent,
  MatchUpdateEvent,
  ProjectInstancesResult,
  SetSourceIntervalResult,
  SourceProgressEvent,
  SourceUpdateEvent,
  SourceUpdateResult,
};
export type GameSourceSubscription = GameSourceSummary;

// ---- source task registry (host snapshot) --------------------------------

export type SourceTaskKind = "update_source" | "match_games" | "rematch_game";
export type SourceTaskProgress = SourceProgressEvent | MatchProgressEvent;
export type SourceTaskEvent =
  | SourceProgressEvent
  | SourceUpdateEvent
  | MatchProgressEvent
  | MatchUpdateEvent;
export type SourceTaskResult = SourceUpdateResult | MatchGamesResult;
export type SourceTaskStatus = {
  task_id: string;
  kind: SourceTaskKind;
  key: string;
  running: boolean;
  cancelled: boolean;
  progress: SourceTaskProgress | null;
  last_event: SourceTaskEvent | null;
  result: SourceTaskResult | null;
  error: { code: string; detail?: string | null } | null;
};

export function taskKindLabel(kind: SourceTaskKind): string {
  const labels: Record<SourceTaskKind, string> = {
    update_source: "更新来源",
    match_games: "匹配补丁",
    rematch_game: "重新匹配",
  };
  return labels[kind] ?? kind;
}

export function taskProgressText(task: SourceTaskStatus): string {
  const progress = task.progress;
  if (!progress) return "正在准备";
  const label = sourcePhaseLabel(progress.phase);
  return progress.total > 0
    ? `${label} · ${progress.completed} / ${progress.total}`
    : label;
}

export function taskForSource(tasks: SourceTaskStatus[], sourceId: string): SourceTaskStatus | undefined {
  return tasks.find(task => task.kind === "update_source" && task.key === sourceId);
}

export function taskForInstance(tasks: SourceTaskStatus[], instanceId: string): SourceTaskStatus | undefined {
  return tasks.find(task => task.kind === "rematch_game" && task.key === `instance:${instanceId}`);
}

export function runningTasks(tasks: SourceTaskStatus[]): SourceTaskStatus[] {
  return tasks.filter(task => task.running);
}

// ---- game match status -> Windows tone / chip ----------------------------

/** Map the shared six-state tone vocabulary onto the Windows chip tones. */
export function statusTone(code?: string | null): Tone {
  switch (gameMatchStatusTone(code)) {
    case "ok": return "ok";
    case "error": return "fail";
    case "pending": return "idle";
    default: return "warn";
  }
}

/** Compact chip text for the game list; idle copy comes from the shared map. */
export function chipLabel(code?: string | null, running = false): string {
  if (running) return "匹配中";
  return gameMatchStatusLabel(code);
}

/** Full sentence for a game match status (shared six-state label). */
export function statusLabel(code?: string | null, running = false): string {
  if (running) return "正在匹配补丁来源…";
  return gameMatchStatusLabel(code);
}

// ---- per-source update status -> Windows tone / chip ---------------------

/** Tone for the source card; the status copy comes from the shared `reason`. */
export function sourceStatusTone(code?: string | null): Tone {
  switch (code) {
    case "source_refreshed":
    case "source_not_modified":
      return "ok";
    case "source_partial":
    case "source_offline_cached":
      return "warn";
    case "source_refresh_failed":
      return "fail";
    default:
      return "idle";
  }
}

export function sourceChipLabel(code?: string | null, running = false): string {
  if (running) return "更新中";
  return reason(code ?? "not_refreshed");
}

export function sourceStatusLabel(code?: string | null, running = false): string {
  if (running) return "正在更新来源…";
  return reason(code ?? "not_refreshed");
}

// ---- game row summaries --------------------------------------------------

/**
 * Statement row summary for the game list; the six-state discovery status only
 * applies once no installable patch is present.
 */
export function rowSummary(row: Install, matching: boolean): string {
  return installSummary(row, matching);
}

/** Primary list status describes the patch, independently of source lookup. */
export function patchChip(row: Install, running: boolean): { tone: Tone; label: string } {
  if (row.patches.some(patch => isManaged(patch.detection.status) || isExternal(patch.detection.status))) {
    return { tone: "ok", label: "已安装补丁" };
  }
  if (row.patches.some(patch => ["partial", "conflict", "partial_or_conflict", "needs_revalidation"].includes(patch.detection.status))) {
    return { tone: "warn", label: "补丁需检查" };
  }
  if (row.patches.some(patch => patch.actions.automatic.available)) return { tone: "ok", label: "有可用补丁" };
  if (row.patches.length || row.discovery?.candidates.some(candidateMatched)) {
    return { tone: "warn", label: "补丁待核对" };
  }
  const status = row.discovery?.status;
  return { tone: statusTone(status), label: chipLabel(status, running) };
}

/** Compact source affordance for a game list row, or null when nothing to add. */
export function sourceChip(row: Install, running: boolean): { tone: Tone; label: string } | null {
  if (!row.discovery && !running) return null;
  const candidates = row.discovery?.candidates ?? [];
  if (candidates.some(candidateMatched)) return { tone: "ok", label: gameMatchStatusLabel("publisher_match") };
  if (row.patches.length) return null; // the summary already conveys installable patches
  const status = row.discovery?.status;
  return { tone: statusTone(status), label: chipLabel(status, running) };
}

export function matchedCandidateCount(row: Install): number {
  return (row.discovery?.candidates ?? []).filter(candidateMatched).length;
}

export function unqueriedCount(rows: Install[]): number {
  return rows.filter(row => !row.patches.length
    && !(row.discovery?.candidates ?? []).some(candidateMatched)
    && (!row.discovery || row.discovery.status === "pending")).length;
}

/**
 * Instances a scan should hand to an automatic incremental `match_games` run:
 * installed games that have no match record yet. Auto-matching needs an enabled
 * publisher source (HGamer is only queried per game during matching) and does
 * not stack behind a library-wide match task that is already running.
 */
export function pendingMatchInstances(
  rows: Install[],
  sources: SourceSummary[] | null | undefined,
  tasks: SourceTaskStatus[],
): string[] {
  const hasPublisher = (sources ?? []).some(source => source.enabled && sourceRoleOf(source) === "publisher");
  if (!hasPublisher) return [];
  if (tasks.some(task => task.kind === "match_games" && task.key === "match" && task.running)) return [];
  return rows
    .filter(row => row.game.availability === "installed")
    .filter(row => !row.discovery || row.discovery.status === "pending")
    .map(row => row.game.instance_id);
}

// ---- candidate presentation ----------------------------------------------

const LANGUAGE_LABELS: Record<string, string> = {
  "zh-CN": "简体中文",
  "zh-TW": "繁体中文",
  zh: "中文",
  en: "英文",
  "en-US": "英文",
  ja: "日文",
  "ja-JP": "日文",
};

const SOURCE_NAMES: Record<string, string> = {
  "kagura-cn": "Kagura Games 国内",
  "kagura-en": "Kagura Games 海外",
  hgamer: "HGamer",
};

export function candidateSourceLabel(candidate: SourceCandidate): string {
  const name = SOURCE_NAMES[candidate.source_id] || candidate.source_id;
  return `${name} · ${candidate.source_id === "hgamer" ? "补丁索引" : "发行商补丁源"}`;
}

export function candidateLanguage(candidate: SourceCandidate): string {
  return candidate.languages?.length
    ? candidate.languages.map(language => LANGUAGE_LABELS[language] ?? language).join("、")
    : "语言待确认";
}

export function variantLabel(variant?: string | null): string {
  const labels: Record<string, string> = { base: "游戏本体", dlc: "扩展 / DLC", demo: "试玩版", unknown: "版本类型待确认" };
  return labels[variant ?? "unknown"] ?? (variant || "版本类型待确认");
}

export function candidateConditions(candidate: SourceCandidate): string {
  const parts: string[] = [variantLabel(candidate.variant)];
  if (candidate.requires_dlc?.length) parts.push(`需要 DLC ${candidate.requires_dlc.join("、")}（拥有及安装状态待确认）`);
  if (candidate.excludes_dlc?.length) parts.push(`仅适用于未使用 DLC ${candidate.excludes_dlc.join("、")} 的版本`);
  if (candidate.requires_dlc == null && candidate.excludes_dlc == null) parts.push("DLC 条件待确认");
  return parts.join(" · ");
}

export function candidateFormat(candidate: SourceCandidate): { label: string; conflict: boolean } {
  const observed = candidate.format_observed && candidate.format_observed !== "unknown" ? candidate.format_observed : null;
  const reported = candidate.format_reported && candidate.format_reported !== "unknown" ? candidate.format_reported : null;
  const conflict = !!observed && !!reported && observed !== reported;
  const labels: Record<string, string> = { zip: "ZIP 压缩包", exe: "EXE 安装程序", web: "网页页面", mixed: "混合格式" };
  const value = observed ?? reported;
  return { label: value ? labels[value] ?? value : "待确认", conflict };
}

export function matchBasisLabel(candidate: SourceCandidate): string {
  const labels: Record<string, string> = {
    appid: "按 AppID 匹配",
    verified_alias: "按已核实别名匹配",
    official_title: "按官方标题匹配",
    title: "仅标题相似",
    none: "身份未确认",
  };
  return labels[candidate.match_basis ?? "none"] ?? "身份未确认";
}

export function versionEvidence(candidate: SourceCandidate): string[] {
  return (candidate.version_evidence ?? []).filter((value): value is string => typeof value === "string" && value.trim().length > 0);
}

export function sourceRoleOf(source: SourceSummary): "publisher" | "fallback_indexer" {
  return source.role === "fallback_indexer" || source.id === "hgamer" ? "fallback_indexer" : "publisher";
}
