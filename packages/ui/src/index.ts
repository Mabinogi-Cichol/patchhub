// Wire DTOs are generated from the Rust protocol/core types.
import type { ActionBlocker, InstallerOutcome, PatchMatch, InstanceProjection, ScanReport, SourceCandidate, SourceSummary } from "../../contracts/generated/protocol";
export type { ManualDownloadInput } from "../../contracts/generated/protocol";
export type { ActionBlockerCode, ActionBlocker, ActionAvailability, PatchActions, SourceCandidate, DiscoveryDiagnostic, GameDiscovery, DiscoveryResult, GameMatch, GameMatchStatus, MatchGamesResult, MatchProgressEvent, MatchUpdateEvent, ProjectInstancesResult, SetSourceIntervalResult, SourceMatch, SourceProgressEvent, SourceRole, SourceStatus, SourceUpdateEvent, SourceUpdateResult, Transaction, InstallerOutcome, CacheInventory, BackupInventory, BackupCleanupResult, CleanupResult, InstallStage, DomainError } from "../../contracts/generated/protocol";
export type { PatchMatch as Patch, InstanceProjection as Install, SourceSummary as SourceSubscription, GameSourceSummary as GameSourceSubscription, PreviewReport as Preview, ProposedChange as Change } from "../../contracts/generated/protocol";
type Patch = PatchMatch;
type Install = InstanceProjection;
// Only host scan lifecycle metadata is added to the generated projection.
export type ScanResult = ScanReport & { generation?: number; scan_progress?: { completed: number; total: number; running: boolean } };
export type DisplayPatch = { kind: "installable"; key: string; patch: Patch; choices: Patch[] } | { kind: "discovery"; key: string; candidate: SourceCandidate };

export function candidateMatched(candidate: SourceCandidate): boolean {
  return ["appid", "verified_alias", "official_title"].includes(candidate.match_basis ?? "");
}
export function displayPatches(row: Install, preferredLanguage: string | null = null): DisplayPatch[] {
  // A trusted publisher match is the display basis; the fallback indexer's
  // candidates must not be shown next to it (p1-contract §7.1 rule 1).
  const candidates = (row.discovery?.candidates ?? [])
    .filter(candidate => row.discovery?.status !== "publisher_match" || candidate.source_id !== "hgamer");
  const discovery: DisplayPatch[] = [...candidates]
    .filter(candidate => !row.patches.some(patch => patch.candidate_id === candidate.candidate_id))
    .sort((a, b) => Number(a.source_id === "hgamer") - Number(b.source_id === "hgamer")
      || Number(!!preferredLanguage && !a.languages?.includes(preferredLanguage)) - Number(!!preferredLanguage && !b.languages?.includes(preferredLanguage)))
    .map(candidate => ({ kind: "discovery", key: candidate.candidate_id, candidate }));
  const patches = row.patches.filter(patch => patch.compatibility === "manual_selection_required" || patch.actions.overwrite.available
    || !row.patches.some(other => other.candidate_id === patch.candidate_id && other.compatibility === "manual_selection_required" && other.actions.overwrite.available)).sort((a, b) => {
    const variant = (p: Patch) => row.discovery?.candidates.find(c => c.candidate_id === p.candidate_id)?.variant;
    const rank = (p: Patch) => variant(p) === "base" ? 0 : variant(p) === "unknown" ? 1 : 2;
    return rank(a) - rank(b);
  });
  const groups = new Map<string, Patch[]>();
  const choicesForDisplay = patches.filter(p => p.compatibility !== "manual_selection_required" || !p.download_url
    || !patches.some(other => other.candidate_id === p.candidate_id && other.compatibility !== "manual_selection_required" && other.actions.overwrite.available && other.download_url === p.download_url));
  for (const patch of choicesForDisplay) {
    const key = patch.candidate_id ? `candidate:${patch.source_id ?? ""}:${patch.candidate_id}` : patch.selection_id ?? patch.release_id;
    const choices = groups.get(key) ?? []; choices.push(patch); groups.set(key, choices);
  }
  return [...Array.from(groups, ([key, choices]) => {
    const patch = choices[0];
    const title = row.discovery?.candidates.find(c => c.candidate_id === patch.candidate_id)?.patch_title;
    return { kind: "installable" as const, key, patch: title ? { ...patch, name: title } : patch, choices };
  }), ...discovery];
}
export function hasMatchedPatch(row: Install): boolean {
  return row.patches.length > 0 || !!row.discovery?.candidates.some(candidateMatched);
}

export function installSummary(row: Install, scanning = false): string {
  if (row.patches.some(p => isManaged(p.detection.status) || isExternal(p.detection.status))) return "已安装补丁";
  if (row.patches.some(p => p.actions.overwrite.available && p.detection.status === "partial_or_conflict")) return "补丁文件不完整，可覆盖安装";
  if (row.patches.some(p => p.actions.automatic.available)) return "有可用补丁";
  if (row.patches.length || row.discovery?.candidates.length) return "找到补丁来源";
  if (scanning) return "正在匹配补丁";
  return "暂无补丁结果";
}

export type Filter = "all" | "matched" | "installed" | "attention";

export function gameDisplayName(game: Install["game"]): string {
  return game.localized_name?.trim() || game.name;
}

export function isManaged(status: string): boolean {
  return status === "installed_managed";
}

export function isExternal(status: string): boolean {
  return status === "installed_external";
}

export function needsAttention(row: Install): boolean {
  return row.game.availability !== "installed" || row.patches.some(p =>
    p.blockers.length > 0 || p.detection.status === "partial_or_conflict"
    || (p.detection.status === "unknown" && p.support_level !== "discover_only")
  );
}

export function filterInstalls(rows: Install[], filter: Filter, query: string): Install[] {
  const search = query.trim().toLocaleLowerCase();
  return rows.filter(row => {
    if (search && !`${row.game.name} ${row.game.localized_name ?? ""} ${row.game.appid}`.toLocaleLowerCase().includes(search)) return false;
    if (filter === "matched") return hasMatchedPatch(row);
    if (filter === "installed") return row.patches.some(p => isManaged(p.detection.status) || isExternal(p.detection.status));
    if (filter === "attention") return needsAttention(row);
    return true;
  });
}

export function reason(code: string): string {
  // Display-only normalization of host exception labels; never used as protocol authority.
  const normalized = code.replace(/^(Error|RuntimeError|ValueError):\s*/, "");
  if (normalized.startsWith("exe_lua_call_unsupported:")) return "安装器包含暂未支持的执行动作，无法自动安装。";
  if (normalized.startsWith("exe_lua_control_flow_unsupported:")) return "安装器包含暂未支持的动态流程，无法自动安装。";
  if (/^discovery_http_\d{3}$/.test(normalized)) return `来源网页返回 HTTP ${normalized.slice(-3)}，请稍后重试。`;
  const labels: Record<string, string> = {
    source_refreshed: "已实时查询",
    source_not_modified: "官网内容未变化",
    source_offline_cached: "查询失败，使用上次成功查询的缓存",
    source_refresh_failed: "补丁源查询失败，暂无可用缓存",
    source_partial: "仅完成部分来源查询",
    cached: "已缓存，等待实时查询",
    not_refreshed: "尚未查询",
    invalid_source_cache: "源缓存无效，请重新查询",
    source_not_enabled: "补丁源已停用，请重新订阅后查询",
    complete: "已查询来源",
    cache_complete: "已读取成功缓存",
    cache_partial: "已读取部分缓存",
    not_queried: "尚未查询，选择游戏后查找",
    partial: "查询尚未完整完成",
    empty: "已查询的来源未找到补丁",
    failed: "来源查询失败，显示上次结果",
    stale: "显示上次结果，等待重新查询",
    queued: "等待查询来源",
    // v5 six-state game match (p1-contract §7.1) plus per-source match records.
    pending: "尚未匹配，请重新匹配",
    publisher_match: "发行商来源找到补丁",
    fallback_match: "补丁索引（HGamer）找到补丁",
    no_match: "已启用来源中未找到补丁",
    incomplete: "来源覆盖不完整，尚需更新来源",
    matched: "已匹配",
    no_index: "来源尚无可用索引，请先更新来源",
    index_incomplete: "来源索引覆盖不完整，请先更新来源",
    no_candidate: "来源索引中未找到候选",
    detail_incomplete: "来源详情尚未查完",
    detail_failed: "来源详情查询失败",
    disabled: "补查来源已关闭",
    not_needed: "无需补查",
    artifact_format_conflict: "索引标示与上游文件格式不一致，当前仅提供来源引导。",
    version_conflict: "来源中的补丁版本描述不一致，请核对原始说明。",
    dlc_unknown: "DLC 拥有和安装状态待确认，请按实际版本选择。",
    dlc_conditions_unknown: "DLC 适用条件待确认，请核对发布页说明。",
    outside_japan_only: "发布页注明仅适用于日本以外地区，请核对原始说明。",
    fresh: "已实时核查",
    language_unknown: "补丁语言待确认，请核对来源说明。",
    source_access_restricted: "网站访问受限，可打开来源网页查看。",
    source_structure_changed: "网站页面结构已变化，保留上次查询结果。",
    discovery_budget_exhausted: "本次查询时间已用尽，显示已取得的结果。",
    discovery_detail_budget_exhausted: "本次详情查询已达到上限，显示已取得的结果。",
    discovery_index_incomplete: "来源目录尚未查完，可重新查找以继续获取。",
    discovery_retry_exhausted: "来源仍暂不可用，显示上次成功结果。",
    discovery_challenge: "来源网页需要验证码或访问确认，可在浏览器中打开。",
    discovery_compressed_response: "来源网页的压缩格式暂不支持，保留上次查询结果。",
    discovery_pagination_cycle: "来源网页的分页链接重复，查询已停止。",
    discovery_request_cycle: "来源网页的跳转链接重复，查询已停止。",
    discovery_search_structure_changed: "来源搜索页面结构已变化，保留上次查询结果。",
    discovery_structure_changed: "来源页面结构已变化，保留上次查询结果。",
    invalid_source_rule: "补丁源规则格式不正确",
    invalid_source_columns: "表格列配置不正确",
    unsupported_source_adapter: "此网站的解析适配器尚未提供",
    source_exists_or_adapter_unavailable: "源已存在，或解析适配器不可用",
    library_unavailable: "Steam 库不可访问，请检查磁盘或 SD 卡是否已挂载。",
    instance_not_found: "游戏实例已变化，请重新扫描。",
    release_not_found: "补丁版本不在当前目录中，请重新扫描。",
    version_incompatible: "游戏版本尚未验证，不能自动安装。",
    catalog_untrusted: "补丁目录尚未通过签名验证，不能安装。",
    hash_mismatch: "导入包哈希与审核记录不符。",
    file_changed: "游戏文件已变化，请重新预览。",
    recovery_conflict: "检测到恢复冲突，已保留后续修改。",
    unsupported_method: "当前核心版本尚不支持此操作。",
    no_archive_selected: "请先选择本地 ZIP 文件。",
    artifact_requires_manual_download: "未找到下载链接",
    build_not_verified: "当前游戏 build 尚未审核，自动安装已禁用。",
    platform_not_verified: "当前平台尚未审核，自动安装已禁用。",
    source_requires_guidance: "此条目提供补丁来源引导，尚不能自动安装。",
    exe_profile_missing: "此 EXE 补丁尚未完成安装审核，请查看来源说明。",
    exe_profile_not_reviewed: "此 EXE 补丁尚未完成安装审核，请查看来源说明。",
    exe_pipeline_unavailable: "此 EXE 补丁的安全安装流程尚未开放，请查看来源说明。",
    exe_profile_changed: "此补丁的审核记录已变化，请重新查询。",
    exe_release_invalid: "补丁选择已失效，请重新查询。",
    exe_candidate_changed: "补丁信息已变化，请重新查询。",
    exe_candidate_stale: "补丁信息已过期，请重新查询。",
    exe_source_changed: "补丁源规则已变化，请重新查询。",
    exe_source_disabled: "补丁源已停用，暂不能安装。",
    exe_no_direct_download: "补丁暂不能直接下载，请查看来源说明。",
    exe_download_url_invalid: "下载地址不符合安全要求，请查看来源说明。",
    exe_manual_import_not_supported: "此 EXE 补丁需要从官方来源自动下载。",
    exe_prepare_first: "请先下载并检查 EXE 补丁，生成安装计划。",
    exe_confirmation_invalid: "安装状态已失效，请重新安装。",
    exe_confirmation_changed: "安装确认已失效，请重新准备安装。",
    exe_conditions_confirmation_required: "当前游戏的安装条件未能自动核验。",
    exe_installer_condition_mismatch: "当前游戏文件不满足安装器的前置条件，请检查游戏版本、语言及 DLC。",
    exe_analysis_required: "需要重新检查官方安装包后才能安装或重装。",
    exe_event_order_unsupported: "安装器事件结构暂未支持。",
    exe_object_class_unsupported: "安装器使用了暂未支持的对象类型。",
    exe_project_options_unsupported: "安装器使用了暂未支持的项目设置。",
    exe_file_actions_or_options_unsupported: "安装器文件动作或覆盖设置暂未支持。",
    exe_user_data_target_unsupported: "安装包包含存档或用户数据覆盖，已停止安装。",
    exe_terms_changed: "安装包已变化，请重新安装。",
    exe_language_mismatch: "游戏语言与此补丁的适用语言不符。",
    exe_dlc_required: "此补丁需要指定的 DLC，当前游戏不符合条件。",
    exe_dlc_excluded: "此补丁不适用于当前安装的 DLC。",
    exe_manifest_unavailable: "无法读取游戏版本信息，请检查 Steam 游戏安装。",
    exe_manifest_invalid: "游戏版本信息无效，请检查 Steam 游戏安装。",
    exe_manifest_duplicate: "游戏版本信息冲突，请重新扫描。",
    exe_baseline_invalid: "已审核的游戏文件条件无效，暂不能安装。",
    exe_game_file_changed: "游戏原文件已变化，请重新准备安装。",
    exe_game_facts_changed: "游戏状态已变化，请重新准备安装。",
    exe_game_identity_changed: "游戏实例已变化，请重新扫描。",
    exe_linked_game_file: "游戏文件存在不安全的链接，已停止安装。",
    exe_game_file_limit: "游戏文件超出检查限制，暂不能安装。",
    exe_state_insufficient_space: "插件状态目录空间不足，请清理空间后重试。",
    exe_target_insufficient_space: "游戏所在磁盘空间不足，请清理空间后重试。",
    exe_payload_changed: "安装包内容已变化，请重新查找补丁。",
    exe_extract_failed: "从安装包提取文件失败，安装未完成。",
    exe_manifest_changed: "安装包文件清单已变化，请重新安装。",
    exe_worker_evidence_changed: "安装包检查结果不一致，请重新安装。",
    exe_policy_identity_changed: "安装计划关联的游戏或补丁已变化，请重新准备。",
    exe_policy_invalid: "安装计划记录无效，请重新准备。",
    exe_policy_unavailable: "安装计划记录不可用，请重新准备。",
    exe_existing_state_invalid: "已有安装记录无法安全读取，请先处理操作记录。",
    exe_core_job_invalid: "安装任务记录无效，请重新准备。",
    exe_core_plan_invalid: "安装计划无效，请重新准备。",
    invalid_exe_profile: "补丁审核资料无效，暂不能自动安装。",
    exe_profile_mismatch: "安装包已变化，需要重新审核后才能安装。",
    exe_registry_changed: "补丁来源或审核记录已变化，请重新查询。",
    zip_wrapped_layout_unsupported: "无法确认此压缩包的安装目录结构，请查看来源安装说明。",
    exe_worker_unavailable: "无法安全检查此安装包，请稍后重试或查看来源说明。",
    exe_worker_failed: "检查安装包失败，未修改游戏文件。",
    exe_worker_invalid_response: "安装包检查结果无效，未修改游戏文件。",
    exe_worker_timeout: "检查安装包超时，请稍后重试。",
    exe_worker_response_limit: "安装包检查结果超出安全限制。",
    exe_worker_input_output_overlap: "检查目录不安全，已停止处理安装包。",
    exe_worker_path_invalid: "安装包检查路径无效，已停止处理。",
    exe_descriptor_limit: "无法安全限制检查任务，暂不能自动安装。",
    exe_baseline_mismatch: "游戏版本或原文件与已审核版本不符，请核对游戏版本。",
    exe_dlc_mismatch: "当前 DLC 状态不符合此补丁的适用条件。",
    exe_conditions_required: "请确认游戏语言、版本与 DLC 适用条件。",
    exe_insufficient_space: "检查或安装此补丁所需空间不足，请清理空间后重试。",
    game_not_ready: "游戏尚未完整安装或库不可访问，请检查 Steam 下载状态。",
    signed_catalog_required: "需要有效的签名目录才能安装。",
    catalog_expired: "签名目录已到期，请更新目录后重新生成计划。",
    catalog_rollback: "目录版本低于本机已验证版本，请使用较新的可信目录。",
    release_revoked: "此补丁版本已被维护端撤销，不能安装。",
    plan_has_blockers: "安装计划存在阻止条件，请先处理列出的问题。",
    game_closed_confirmation_required: "请关闭游戏并暂停 Steam 更新后确认。",
    game_changed_or_unavailable: "游戏版本或库状态已变化，请重新扫描和预检。",
    installer_busy: "另一项安装或恢复正在执行，请等待完成。",
    game_busy: "此游戏正在被另一项事务处理，请等待完成。",
    insufficient_space: "安装所需磁盘空间不足；请查看所需、可用空间与对应磁盘，清理后重试。",
    resource_limit: "ZIP 目录或安装计划的元数据超出当前资源预算，请检查压缩包结构。",
    zip_metadata_invalid: "ZIP 目录信息无效，请检查补丁文件是否完整。",
    download_size_overflow: "补丁文件尺寸超出系统可表示范围，无法安全下载。",
    zip_size_overflow: "ZIP 文件尺寸超出系统可表示范围，无法安全安装。",
    managed_install_exists_or_needs_recovery: "已有托管安装或待恢复事务；请先恢复未完成事务，或使用覆盖安装。",
    artifact_hash_mismatch: "安装包与已审核记录不符，请重新核对下载包。",
    artifact_size_mismatch: "安装包大小与已审核记录不符，请确认下载完整。",
    state_overlaps_game: "状态目录与游戏目录重叠，已阻止写入。",
    backup_missing_or_changed: "原版备份丢失或已变化，无法安全还原。",
    archive_outside_downloads: "请将 ZIP 放入 Deck 用户的 Downloads 目录再导入。",
    invalid_archive: "所选文件不是可读取的普通 ZIP。"
    ,source_not_released: "来源页面尚未发布此补丁，暂不能下载。"
    ,publisher_prepare_first: "请先自动下载并准备安装，获取文件变更计划。"
    ,publisher_state_unavailable: "本机补丁安装记录不可读取，请检查状态目录。"
    ,publisher_existing_transaction: "已有安装或待恢复记录，请先卸载或修复。"
    ,publisher_match_changed: "来源中的游戏匹配已变化，请重新扫描。"
    ,cancelled: "安装已取消，没有修改游戏文件。"
    ,cancelled_incomplete: "安装已取消，但已写入的文件未恢复。可重新安装补丁；如需恢复游戏本体，请使用 Steam 验证游戏文件。"
    ,install_incomplete: "安装未完成，已写入的文件未恢复。可重新安装补丁；如需恢复游戏本体，请使用 Steam 验证游戏文件。"
    ,game_write_failed: "写入游戏文件失败，安装未完成。"
    ,game_sync_failed: "同步游戏文件失败，安装未完成。"
    ,download_cancelled: "已取消获取或校验，没有执行安装。"
    ,download_http_error: "下载服务返回错误，请稍后重试。"
    ,download_network_error: "补丁下载网络连接失败，请检查网络后重试。"
    ,download_not_zip: "下载内容不是有效 ZIP，未执行安装。"
     ,task_cancelled: "已取消安装，未提交更改。"
     ,install_in_progress: "已有安装任务正在进行，请等待完成。"
     ,request_id_required: "安装请求缺少标识，请重试。"
     ,invalid_request_id: "安装请求标识无效，请重试。"
     ,request_id_binding_conflict: "安装请求与当前选择不一致，请重试。"
     ,install_request_in_progress_or_interrupted: "上一次安装请求未完成，请稍后重试或先恢复。"
     ,request_transaction_missing: "安装结果记录缺失，请重新扫描。"
     ,unsupported_protocol: "核心版本与客户端协议不一致，请更新客户端。"
     ,invalid_request: "安装请求格式无效。"
     ,core_process_failed: "核心进程未能完成操作，请重试。"
     ,core_binary_missing: "缺少核心程序，请重新安装客户端。"
     ,state_dir_required: "状态目录不可用，请检查应用数据目录。"
     ,capability_mismatch: "核心能力与客户端要求不一致，请更新客户端或核心后重试。"
     ,contract_mismatch: "客户端与核心的安装契约不一致，请更新客户端。"
     ,installer_schema_unsupported: "安装器数据结构版本不受支持，请更新客户端与核心。"
     ,cleanup_failed: "临时文件清理失败；安装已完成，可稍后重新扫描确认。"
     ,task_required: "此操作必须通过安装任务执行，请从安装按钮重新发起。"
     ,invalid_selection_id: "补丁选择标识无效，请重新查找来源。"
     ,selection_not_found: "补丁选择已不在来源结果中，请重新查找。"
     ,discovery_candidate_revalidation_incomplete: "部分已知补丁尚未完成核验，保留上次候选，请继续查找。"
     ,selection_stale: "补丁来源记录与当前解析版本不兼容，请重新查找来源。"
     ,local_archive_signed_only: "此来源不支持导入本地 ZIP，请使用来源自动安装。"
     ,selection_or_release_required: "缺少补丁选择或发行版本，请重新选择。"
     ,archive_not_allowed_for_selection: "动态来源补丁不能导入本地 ZIP，请从来源直接安装。"
     ,zip_download_unavailable: "此来源未提供可下载的 ZIP，请查看来源说明。"
     ,zip_download_ambiguous: "来源提供了多个 ZIP 下载，无法唯一确定，请查看来源说明。"
     ,exe_candidate_variant_unsupported: "此补丁的版本类型暂不支持自动安装，请查看来源说明。"
     ,exe_candidate_ambiguous: "来源为此补丁提供了多个可下载项，无法唯一确定。"
     ,exe_download_ambiguous: "来源提供了多个 EXE 下载地址，无法唯一确定。"
     ,exe_conditions_not_satisfied: "当前游戏不满足安装器的条件，请检查游戏版本、语言与 DLC。"
     ,exe_route_unimplemented: "此平台暂不支持 EXE 自动安装，请查看来源说明。"
     ,download_hash_mismatch: "下载文件与来源记录不一致，已停止安装。"
     ,download_failed: "补丁下载失败，请检查网络后重试。"
     ,analysis_failed: "安装包分析失败，请查看来源说明。"
     ,plan_failed: "生成安装计划失败，请重试或查看来源说明。"
     ,reinstall_content_identity_changed: "补丁内容已更新，暂不支持直接叠加安装。"
     ,reinstall_unowned_or_incomplete: "无法确认上次安装记录，暂不能直接重装。"
     ,source_not_subscribed: "尚未订阅此来源，请在来源管理中订阅后重试。"
  };
  return labels[normalized] ?? normalized;
}

export function workflowState(value: string): string {
  const labels: Record<string, string> = {
    manual_selection_required: "按所选下载项安装",
    discover_only: "查看补丁来源", manual_download: "手动下载导入", managed_zip: "审核 ZIP 安装", unsupported: "暂不支持",
    publisher_zip: "ZIP · 默认解压至游戏根目录", publisher_exe: "自动提取并安装", publisher_default: "按根目录生成计划，效果待验证",
    check_required: "安装时自动检查条件", conditions_checked: "安装条件已核验", verified_metadata: "目录条件已核对", needs_review: "尚待审核", signed: "已验证签名", unsigned_development: "未签名开发目录",
    planned: "已生成计划", backing_up: "正在备份", backed_up: "备份完成", applying: "正在写入", committed: "安装完成",
    failed: "安装未完成",
    rolling_back: "正在回滚", rolled_back: "已回滚", uninstalling: "正在卸载", uninstalled: "已卸载", needs_recovery: "需要恢复",
    add: "新增", replace: "替换"
  };
  return labels[value] ?? value;
}

export function patchState(status: string): string {
  const labels: Record<string, string> = {
    not_detected: "未检测到", installed_managed: "已安装 / 托管", installed_external: "已安装 / 外部",
    partial_or_conflict: "部分安装或文件冲突", unknown: "状态未知"
  };
  return labels[status] ?? status;
}


/** Input choice uses authoritative snapshot facts; archive selection remains local UI state. */
export function primaryInstallAction(patch: Patch | null | undefined, archiveSelected: boolean, route: "primary" | "local_archive" = "primary") {
  const primary = patch?.actions.primary;
  const input = route === "local_archive" ? "local_archive" : primary?.input === "automatic" ? "automatic" : "unavailable";
  const available = !!patch && (input === "automatic" ? patch.actions.automatic.available
    : input === "local_archive" ? patch.actions.local_archive.available && archiveSelected : false);
  return { operation: primary?.operation ?? "overwrite", label: primary?.label ?? "install", input, available };
}
export function installInput(patch: Patch | null | undefined, localArchive?: string): string | null {
  if (!patch) return null;
  if (localArchive !== undefined) return localArchive.trim() && patch.actions.local_archive.available ? localArchive : null;
  return patch.actions.primary.input === "automatic" && patch.actions.automatic.available ? "" : null;
}
export function actionBlockers(patch: Patch | null | undefined): ActionBlocker[] {
  if (!patch) return [];
  const input = patch.actions.primary.input;
  const selected = input === "automatic" ? patch.actions.automatic.blockers : patch.actions.overwrite.blockers;
  const seen = new Set<string>();
  return selected.filter(blocker => { const key = `${blocker.code}\0${blocker.detail ?? ""}`;
    if (seen.has(key)) return false; seen.add(key); return true; });
}
export function progressPercent(done: number, total: number | null | undefined): number | null {
  if (total == null || !Number.isFinite(total) || total <= 0 || !Number.isFinite(done)) return null;
  return Math.min(100, Math.max(0, done / total * 100));
}
export function progressStageLabel(stage: string, substage?: string | null): string {
  const labels: Record<string, string> = { connecting:"正在连接", downloading:"正在下载", validating:"正在核对", analyzing_installer:"正在分析安装包", extracting:"正在提取", planning:"正在准备", applying:"正在写入", rolling_back:"正在回滚", cleanup:"正在清理", backing_up:"正在备份", writing:"正在写入", committing:"正在提交" };
  return labels[substage ?? stage] ?? stage;
}
export function safeSourceUrl(value: string): boolean {
  try { const url = new URL(value);
    if (url.protocol !== "https:" || url.username || url.password || (url.port && url.port !== "443")) return false;
    const host = url.hostname.toLowerCase();
    if (host.endsWith(".")) return false;
    return host.includes(".") && host !== "localhost" && !host.endsWith(".localhost") && !host.endsWith(".local") && !host.includes(":") && !/^\d+\.\d+\.\d+\.\d+$/.test(host);
  } catch { return false; }
}
export function sourceRefreshFailed(status: string | null | undefined): boolean {
  return ["failed", "source_refresh_failed", "source_offline_cached"].includes(status ?? "") || /^discovery_http_\d{3}$/.test(status ?? "");
}
export function sourceQueryComplete(status: string | null | undefined): boolean {
  return ["complete", "empty", "cache_complete", "publisher_match", "fallback_match", "no_match"].includes(status ?? "");
}
/** New invocation outcomes use core disposition; historical transaction display is separate. */
export function installationResult(outcome: InstallerOutcome, overwrite = false): string {
  if (outcome.disposition === "completed") return overwrite ? "覆盖安装完成" : "安装完成";
  if (outcome.recovery?.required) return "操作需要修复，请运行修复后重试。";
  if (outcome.disposition === "cancelled") return "任务已安全取消。";
  const failure = outcome.cause?.code ?? outcome.recovery?.code ?? "installation_failed";
  const detail = outcome.cause?.detail ?? outcome.recovery?.detail;
  // Direct EXE installs leave already-written files in place on failure or
  // cancellation; the warning itself is the complete message.
  if (failure === "install_incomplete" || failure === "cancelled_incomplete") {
    return reason(failure);
  }
  return `安装未完成：${reason(failure)}${detail ? ` · ${detail}` : ""}`;
}

export function sourceRefreshSucceeded(status: string | null | undefined): boolean {
  return ["fresh", "source_refreshed", "source_not_modified"].includes(status ?? "");
}

/** Display an untrusted URL filename without letting malformed escapes break rendering. */
export function downloadFileName(value: string): string {
  try {
    const raw = new URL(value).pathname.split("/").pop() || value;
    try { return decodeURIComponent(raw); } catch { return raw; }
  } catch { return value; }
}

// -- v5 game match / source schedule display (shared by both host UIs) ------

export function sourceCoverageComplete(source: Pick<SourceSummary, "complete"> | null | undefined): boolean {
  return source?.complete === true;
}

export function sourceCoverageText(source: Pick<SourceSummary, "complete" | "status"> | null | undefined): string {
  if (!source) return "来源状态未知";
  if (source.complete) return "索引覆盖完整";
  return "索引覆盖不完整，尚需更新来源";
}

export const SOURCE_INTERVAL_PRESETS: { hours: number; label: string }[] = [
  { hours: 0, label: "关闭" },
  { hours: 6, label: "6 小时" },
  { hours: 12, label: "12 小时" },
  { hours: 24, label: "24 小时" },
  { hours: 72, label: "3 天" },
  { hours: 168, label: "7 天" },
  { hours: 336, label: "14 天" },
];

export function sourceIntervalOptions(): { data: number; label: string }[] {
  return SOURCE_INTERVAL_PRESETS.map(({ hours, label }) => ({ data: hours, label }));
}

export function sourceIntervalLabel(hours: number | null | undefined): string {
  const value = hours ?? 0;
  return SOURCE_INTERVAL_PRESETS.find(preset => preset.hours === value)?.label ?? `${value} 小时`;
}

export function formatUnixSeconds(value: number | null | undefined): string {
  return value == null ? "—" : new Date(value * 1000).toLocaleString();
}

export function sourceLastSuccessText(source: { cached_at?: number | null }): string {
  return source.cached_at == null ? "尚未成功更新" : formatUnixSeconds(source.cached_at);
}

export function sourceLastAttemptText(source: { checked_at?: number | null; status?: string | null }): string {
  if (source.checked_at == null) return "尚未尝试";
  return `${formatUnixSeconds(source.checked_at)} · ${reason(source.status ?? "")}`;
}

export function sourceNextDueText(
  source: { interval_hours?: number | null; next_due_at?: number | null },
  now = Date.now(),
): string {
  const hours = source.interval_hours ?? 0;
  if (!hours) return "自动更新已关闭";
  if (source.next_due_at == null) return `每 ${sourceIntervalLabel(hours)}；等待首次成功更新`;
  return source.next_due_at * 1000 <= now
    ? `已到更新周期（每 ${sourceIntervalLabel(hours)}）`
    : `下次 ${formatUnixSeconds(source.next_due_at)}（每 ${sourceIntervalLabel(hours)}）`;
}

/** Task phase for `source_progress` and `match_progress` events. */
export function sourcePhaseLabel(phase: string | null | undefined): string {
  const labels: Record<string, string> = {
    inventory: "扫描游戏库", index: "更新来源索引", details: "获取补丁详情",
    matched: "匹配游戏", fallback: "补查补丁索引",
  };
  return labels[phase ?? ""] ?? "准备中";
}
