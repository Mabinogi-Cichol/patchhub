import {
  candidateMatched,
  installSummary,
  isExternal,
  isManaged,
  reason,
  sourceRefreshFailed,
  sourceRefreshSucceeded,
  sourceQueryComplete,
  type GameSourceSubscription,
  type Install,
  type SourceCandidate,
} from "@steam-patch/ui";

/**
 * Shared source-discovery helpers for the Windows host.
 *
 * The Rust core owns identity, cache and status semantics; this module only
 * projects the host/CLI DTOs into short, grounded Chinese labels and tonal
 * categories. The small candidate copy mirrors the Decky presentation layer so
 * both clients describe the same states the same way.
 */

export type Tone = "ok" | "warn" | "fail" | "busy" | "idle";

export { primaryInstallAction, actionBlockers, safeSourceUrl } from "@steam-patch/ui";

// ---- host command results -------------------------------------------------

export type { Capabilities, SourceSummary } from "../../../packages/contracts/generated/protocol";
import type { SourceSummary, ScanSourcesResult, SourceProgressEvent, GameSourceUpdateEvent, RefreshSourceUpdateEvent } from "../../../packages/contracts/generated/protocol";
export type SourceScanResult = ScanSourcesResult;
export type SourceScanItem = ScanSourcesResult["results"][number];

export type SourceScanStatus = {
  running: boolean;
  cancelled: boolean;
  progress: { completed: number; total: number; running: boolean; settings_revision?: number } | null;
  results: SourceScanResult | null;
};

export type SourceEvent = SourceProgressEvent | GameSourceUpdateEvent | RefreshSourceUpdateEvent;

// ---- status -> tone / label ----------------------------------------------

const IDLE_CODES = new Set(["empty", "not_queried", "not_refreshed", "queued", "source_not_enabled"]);

/** Tonal category used for the status chips. */
export function statusTone(code?: string | null): Tone {
  const value = code ?? "not_refreshed";
  if (/^discovery_http_\d{3}$/.test(value)) return "fail";
  if (sourceRefreshFailed(value)) return "fail";
  if (sourceQueryComplete(value) || sourceRefreshSucceeded(value)) return "ok";
  if (IDLE_CODES.has(value)) return "idle";
  return "warn";
}

/** Short chip text for the game list and source cards. */
export function chipLabel(code?: string | null, running = false): string {
  if (running) return "查源中";
  switch (code ?? "not_queried") {
    case "failed":
    case "source_refresh_failed":
      return "来源失败";
    case "source_offline_cached":
      return "失败·用缓存";
    case "partial":
    case "cache_partial":
    case "source_partial":
    case "discovery_index_incomplete":
      return "部分来源";
    case "stale":
    case "cached":
    case "source_not_modified":
      return "缓存结果";
    case "empty":
      return "来源无补丁";
    case "complete":
    case "cache_complete":
    case "source_refreshed":
    case "fresh":
      return "来源已查询";
    case "not_queried":
    case "queued":
    case "not_refreshed":
      return "未查询";
    case "source_not_enabled":
      return "来源已停用";
    default:
      return "待核对";
  }
}

/** Full sentence for a discovery/source status code. */
export function statusLabel(code?: string | null, running = false): string {
  if (running) return "正在查找补丁来源…";
  return reason(code ?? "not_queried");
}

/**
 * Decky's list summary says "正在查找补丁" for `not_queried`/`stale` rows even
 * when nothing is running. Make the idle case explicit so a stale row never
 * looks like a permanent in-progress state.
 */
export function rowSummary(row: Install, scanning: boolean): string {
  const summary = installSummary(row, scanning);
  if (summary !== "正在查找补丁" || scanning) return summary;
  const status = row.discovery?.status;
  if (status === "stale" || status === "cached") return "显示上次来源结果";
  return "未查询补丁来源";
}

/** Primary list status describes the patch, independently of source lookup. */
export function patchChip(row: Install, scanning: boolean): { tone: Tone; label: string } {
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
  if (scanning && (!status || ["not_queried", "queued", "stale"].includes(status))) {
    return { tone: "busy", label: "正在查找补丁" };
  }
  if (!status || ["not_queried", "queued", "stale"].includes(status)) {
    return { tone: "idle", label: "补丁待查询" };
  }
  if (["empty", "complete", "cache_complete"].includes(status)) {
    return { tone: "idle", label: "未找到补丁" };
  }
  return { tone: "warn", label: "补丁查询未完成" };
}

/** Compact source affordance for a game list row, or null when nothing to add. */
export function sourceChip(row: Install, running: boolean): { tone: Tone; label: string } | null {
  if (!row.discovery && !running) return null;
  const candidates = row.discovery?.candidates ?? [];
  if (candidates.some(candidateMatched)) return { tone: "ok", label: "已找到来源" };
  if (row.patches.length) return null; // the summary already conveys installable patches
  const status = row.discovery?.status;
  if (running && (!status || status === "not_queried" || status === "queued")) return { tone: "busy", label: "查源中" };
  return { tone: statusTone(status), label: chipLabel(status) };
}

export function matchedCandidateCount(row: Install): number {
  return (row.discovery?.candidates ?? []).filter(candidateMatched).length;
}

export function unqueriedCount(rows: Install[]): number {
  return rows.filter(row => !row.patches.length
    && !(row.discovery?.candidates ?? []).some(candidateMatched)
    && (!row.discovery || row.discovery.status === "not_queried")).length;
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

export function formatTime(value?: number | null): string {
  if (!value) return "—";
  return new Date(value * 1000).toLocaleString();
}

export function sourceRoleOf(source: SourceSummary): "publisher" | "fallback_indexer" {
  return source.role === "fallback_indexer" || source.id === "hgamer" ? "fallback_indexer" : "publisher";
}

export type { GameSourceSubscription };
