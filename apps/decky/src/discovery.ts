import { candidateMatched, gameMatchStatusHint, gameMatchStatusLabel, hasMatchedPatch, type Install, type SourceCandidate } from "@steam-patch/ui";

// `InstanceProjection.discovery.status` is the v5 six-state game match
// (p1-contract §7.1). Source-level status codes remain for legacy rows.
export function discoveryStatus(status?: string): string {
  switch (status) {
    case "pending":
    case "not_queried":
      return "尚未匹配，请重新匹配";
    case "publisher_match":
      return "发行商来源命中";
    case "fallback_match":
      return "补丁索引（HGamer）命中";
    case "no_match":
      return "已启用来源中未找到补丁";
    case "incomplete":
      return "来源覆盖不完整，尚需更新来源";
    case "failed":
      return "来源查询失败，显示上次结果";
    case "complete":
    case "empty":
    case "cache_complete":
    case "cached":
      return "已查询来源";
    case "partial":
    case "cache_partial":
    case "source_partial":
      return "部分来源已查询，仍有信息待核实";
    case "source_offline_cached":
      return "来源暂不可用，显示上次结果";
    case "source_refresh_failed":
      return "来源暂不可用，稍后重试";
    default:
      return "来源信息待核实";
  }
}

/** Extra explanation for coverage gaps and fallback query states. */
export function discoveryNotice(row: Install): string | null {
  return gameMatchStatusHint(row.discovery?.status);
}

export function discoverySummary(row: Install): string {
  const count = row.discovery?.candidates.filter(candidateMatched).length ?? 0;
  if (count) return `已找到补丁 · ${count} 个来源候选`;
  if (row.discovery?.candidates.length) return "待确认来源";
  return gameMatchStatusLabel(row.discovery?.status);
}

export function unqueriedCount(rows: Install[]): number {
  return rows.filter(row => !hasMatchedPatch(row)
    && (!row.discovery || ["pending", "not_queried"].includes(row.discovery.status))).length;
}

export function candidateSource(candidate: SourceCandidate): string {
  const labels: Record<string, string> = { "kagura-cn": "Kagura Games 国内", "kagura-en": "Kagura Games 海外", hgamer: "HGamer" };
  const name = labels[candidate.source_id] || candidate.source_id;
  return `${name}${candidate.source_id === "hgamer" ? " · 补丁索引" : " · 发行商补丁源"}${candidateMatched(candidate) ? "" : " · 游戏身份待确认"}`;
}
export function candidateLanguage(candidate: SourceCandidate): string {
  const labels: Record<string, string> = { "zh-CN": "简体中文", "zh-TW": "繁体中文", zh: "中文", en: "英文", ja: "日文" };
  return candidate.languages?.length ? candidate.languages.map(language => labels[language] || language).join("、") : "语言待确认";
}
export function candidateConditions(candidate: SourceCandidate): string {
  const parts: string[] = [];
  const variants: Record<string, string> = { base: "本体", dlc: "扩展 / DLC", demo: "试玩版", unknown: "版本类型待确认" };
  parts.push(variants[candidate.variant ?? "unknown"] ?? candidate.variant!);
  if (candidate.requires_dlc?.length) parts.push(`需要 DLC ${candidate.requires_dlc.join("、")}（拥有及安装状态待确认）`);
  if (candidate.excludes_dlc?.length) parts.push(`仅适用于未使用 DLC ${candidate.excludes_dlc.join("、")} 的版本`);
  if (candidate.requires_dlc == null && candidate.excludes_dlc == null) parts.push("DLC 条件待确认");
  return parts.join(" · ");
}
export { safeSourceUrl } from "@steam-patch/ui";
