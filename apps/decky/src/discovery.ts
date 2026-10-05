import { candidateMatched, hasMatchedPatch, type Install, type SourceCandidate } from "@steam-patch/ui";

export function discoveryStatus(status?: string): string {
  const labels: Record<string, string> = {
    not_queried: "尚未查询，选择游戏后查找", complete: "已查询来源", cache_complete: "已读取成功缓存", cached: "已读取成功缓存",
    partial: "部分来源已查询，仍有信息待核实", cache_partial: "缓存结果不完整，可重新查询", empty: "已查询的来源未找到补丁",
    failed: "来源暂不可用，显示上次结果", source_offline_cached: "来源暂不可用，显示上次结果", source_refresh_failed: "来源暂不可用，稍后重试",
    stale: "显示上次结果，可重新查询", queued: "等待查询来源",
  };
  return labels[status ?? "not_queried"] ?? "来源信息待核实";
}
export function discoverySummary(row: Install): string {
  const count = row.discovery?.candidates.filter(candidateMatched).length ?? 0;
  if (count) return `已找到补丁 · ${count} 个来源候选`;
  if (row.discovery?.candidates.length) return "待确认来源";
  if (["complete", "cache_complete", "empty"].includes(row.discovery?.status ?? "")) return "已查询的来源未找到补丁";
  return discoveryStatus(row.discovery?.status);
}
export function unqueriedCount(rows: Install[]): number {
  return rows.filter(row => !hasMatchedPatch(row) && (!row.discovery || row.discovery.status === "not_queried")).length;
}
export function candidateSource(candidate: SourceCandidate): string {
  const labels: Record<string, string> = { "kagura-cn": "Kagura Games 国内", "kagura-en": "Kagura Games 海外", hgamer: "HGamer" };
  const name = candidate.source_name || labels[candidate.source_id] || candidate.source_id;
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
export function safeSourceUrl(value: string): boolean {
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" || url.username || url.password || (url.port && url.port !== "443")) return false;
    const host = url.hostname.toLowerCase();
    if (!host.includes(".") || host === "localhost" || host.endsWith(".localhost") || host.endsWith(".local") || host.includes(":")) return false;
    if (/^\d+\.\d+\.\d+\.\d+$/.test(host)) return false;
    return true;
  } catch { return false; }
}
