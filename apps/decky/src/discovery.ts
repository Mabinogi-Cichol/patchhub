import { type SourceCandidate } from "@patchhub/ui";

export function candidateSource(candidate: SourceCandidate): string {
  const labels: Record<string, string> = { "kagura-cn": "Kagura Games 国内", "kagura-en": "Kagura Games 海外", hgamer: "HGamer" };
  const name = labels[candidate.source_id] || candidate.source_id;
  return `${name}${candidate.source_id === "hgamer" ? " · 补丁索引" : " · 发行商补丁源"}`;
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
export { safeSourceUrl } from "@patchhub/ui";
