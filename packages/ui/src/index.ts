export type Patch = {
  release_id: string;
  candidate_id?: string;
  name: string;
  source_page: string;
  source_id?: string;
  source_name?: string;
  source_status?: string;
  source_cached_at?: number | null;
  support_level: string;
  download_available?: boolean;
  auto_installable?: boolean;
  reinstall_available?: boolean;
  exe?: { language?: string; version?: string; variant?: string; requires_dlc?: number[]; excludes_dlc?: number[]; profile_id: string };
  compatibility: string;
  blockers: string[];
  detection: { status: string; files: { path: string; status: string }[] };
};

export type Install = {
  game: {
    instance_id: string;
    appid: number;
    name: string;
    localized_name?: string | null;
    name_language?: string | null;
    library_path: string;
    install_path: string;
    buildid: string | null;
    availability: string;
    kind: string;
  };
  match_status: string;
  patches: Patch[];
  sources?: GameSourceSubscription[];
  discovery?: GameDiscovery;
  exe_reviews?: ExeReview[];
};

// Discovery identities belong to the Decky presentation layer. They never
// become an installation release_id or authorize a prepare request.
export type SourceCandidate = {
  candidate_id: string;
  source_id: string;
  source_name?: string;
  source_record_id?: string;
  game_appid?: number | null;
  parent_appid?: number | null;
  game_title: string;
  aliases?: string[];
  patch_title: string;
  variant?: string;
  languages?: string[] | null;
  version_raw?: string | null;
  version_evidence?: unknown[];
  requires_dlc?: number[] | null;
  excludes_dlc?: number[] | null;
  platforms?: string[] | null;
  patch_kind?: string;
  source_page: string;
  upstream_page?: string | null;
  mirrors?: { url: string; label?: string; format?: string }[];
  format_reported?: string | null;
  format_observed?: string | null;
  capability: string;
  match_basis?: string;
  identity_evidence?: unknown[];
  provenance?: unknown[];
  observed_at?: number | string | null;
  cache_status?: string;
  warnings?: string[];
};
export type DiscoveryDiagnostic = { code: string; message?: string; detail?: string; source_id?: string };
export type GameDiscovery = { status: string; candidates: SourceCandidate[]; diagnostics: DiscoveryDiagnostic[] };
export type ExeReview = { candidate_id: string; release_id: string | null; profile_id: string | null; status: "guidance" | "installable" | "check_required"; blockers: string[] };
export type DiscoveryResult = GameDiscovery & { instance_id: string; appid: number; generation: number; settings_revision: number; exe_reviews?: ExeReview[]; exe_patches?: Patch[]; patches?: Patch[] };
export type DisplayPatch = { kind: "installable"; key: string; patch: Patch } | { kind: "discovery"; key: string; candidate: SourceCandidate };

export function candidateMatched(candidate: SourceCandidate): boolean {
  return ["appid", "verified_alias", "official_title"].includes(candidate.match_basis ?? "");
}
export function displayPatches(row: Install, preferredLanguage: string | null = null): DisplayPatch[] {
  const discovery: DisplayPatch[] = [...(row.discovery?.candidates ?? [])]
    .filter(candidate => !row.patches.some(patch => patch.candidate_id === candidate.candidate_id))
    .sort((a, b) => Number(a.source_id === "hgamer") - Number(b.source_id === "hgamer")
      || Number(!!preferredLanguage && !a.languages?.includes(preferredLanguage)) - Number(!!preferredLanguage && !b.languages?.includes(preferredLanguage)))
    .map(candidate => ({ kind: "discovery", key: candidate.candidate_id, candidate }));
  return [...row.patches.map(patch => ({ kind: "installable" as const, key: patch.release_id, patch })), ...discovery];
}
export function hasMatchedPatch(row: Install): boolean {
  return row.patches.length > 0 || !!row.discovery?.candidates.some(candidateMatched);
}

export type ScanResult = {
  roots: string[];
  catalog_revision: number;
  catalog_trust: string;
  platform: string;
  installs: Install[];
  diagnostics: { code: string; path: string; message: string }[];
  sources?: SourceSubscription[];
  generation?: number;
  settings_revision?: number;
  scan_progress?: { completed: number; total: number; running: boolean };
};

export function canAutoInstall(patch: Patch): boolean {
  return patch.auto_installable === true && patch.blockers.length === 0 && patch.source_id !== "hgamer";
}

export function installSummary(row: Install, scanning = false): string {
  if (row.patches.some(p => isManaged(p.detection.status) || isExternal(p.detection.status))) return "已安装补丁";
  if (row.patches.some(p => p.reinstall_available && ["partial", "conflict", "partial_or_conflict", "needs_revalidation"].includes(p.detection.status))) return "补丁文件不完整，需重新安装";
  if (row.patches.some(canAutoInstall)) return "有可用补丁";
  if (row.patches.length || row.discovery?.candidates.some(candidateMatched)) return "找到补丁来源";
  if (["failed", "source_refresh_failed", "source_offline_cached"].includes(row.discovery?.status ?? "")) return "来源查询失败";
  if (scanning && !["complete", "empty", "cache_complete"].includes(row.discovery?.status ?? "")) return "正在查找补丁";
  if (["partial", "cache_partial", "source_partial"].includes(row.discovery?.status ?? "")) return "来源查询不完整";
  if (["not_queried", "queued", "stale"].includes(row.discovery?.status ?? "not_queried")) return "正在查找补丁";
  return "暂未找到补丁";
}

export type SourceSubscription = {
  id: string;
  name: string;
  adapter: string;
  enabled: boolean;
  pages: string[];
  status?: string;
  cached_at?: number;
  checked_at?: number;
  entry_count?: number;
  role?: string;
};

export type GameSourceSubscription = SourceSubscription & {
  subscribed: boolean;
  matched: boolean;
};

export type Change = { path: string; action: string; baseline_matches?: boolean };
export type Preview = {
  instance_id: string;
  release_id: string;
  catalog_revision: number;
  executable: boolean;
  blockers: string[];
  changes: Change[];
};
export type Plan = Omit<Preview, "executable"> & { plan_id?: string; expires_at?: number; preconditions?: string[]; disk_requirement?: number };
export type Transaction = {
  transaction_id: string;
  instance_id: string;
  release_id: string;
  state: string;
  error?: string | null;
  backups?: boolean;
};

export type Filter = "all" | "matched" | "installed" | "attention";

export function gameDisplayName(game: Install["game"]): string {
  return game.localized_name?.trim() || game.name;
}

export function isManaged(status: string): boolean {
  return ["managed", "installed_managed", "installed_owned"].includes(status);
}

export function isExternal(status: string): boolean {
  return ["external", "installed_external"].includes(status);
}

export function needsAttention(row: Install): boolean {
  return row.game.availability !== "installed" || row.patches.some(p =>
    p.blockers.length > 0 || ["partial", "conflict", "partial_or_conflict", "needs_revalidation"].includes(p.detection.status)
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
  const normalized = code.trim().replace(/^(Error|RuntimeError|ValueError):\s*/, "");
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
    failed: "来源暂不可用，显示上次结果",
    stale: "显示上次结果，等待重新查询",
    queued: "等待查询来源",
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
    discovery_retry_deferred: "来源要求稍后重试，显示已取得的结果。",
    discovery_retry_exhausted: "来源仍暂不可用，显示上次成功结果。",
    discovery_challenge: "来源网页需要验证码或访问确认，可在浏览器中打开。",
    discovery_compressed_response: "来源网页的压缩格式暂不支持，保留上次查询结果。",
    discovery_html_too_large: "来源网页超过查询大小限制，显示已取得的结果。",
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
    archive_required: "请先导入从补丁来源下载的 ZIP。",
    artifact_requires_manual_download: "此来源需要浏览器下载；打开补丁来源后导入 ZIP。",
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
    exe_backups_required: "此补丁必须保留原文件备份，请启用备份后重试。",
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
    exe_manifest_limit: "游戏版本信息超出安全限制。",
    exe_baseline_invalid: "已审核的游戏文件条件无效，暂不能安装。",
    exe_game_file_changed: "游戏原文件已变化，请重新准备安装。",
    exe_game_facts_changed: "游戏状态已变化，请重新准备安装。",
    exe_game_identity_changed: "游戏实例已变化，请重新扫描。",
    exe_linked_game_file: "游戏文件存在不安全的链接，已停止安装。",
    exe_game_file_limit: "游戏文件超出检查限制，暂不能安装。",
    exe_state_insufficient_space: "插件状态目录空间不足，请清理空间后重试。",
    exe_target_insufficient_space: "游戏所在磁盘空间不足，请清理空间后重试。",
    exe_original_size_limit: "原游戏文件超过备份安全限制，暂不能安装。",
    exe_payload_changed: "安装包内容已变化，请重新查找补丁。",
    exe_manifest_changed: "安装包文件清单已变化，请重新安装。",
    exe_worker_evidence_changed: "安装包检查结果不一致，请重新安装。",
    exe_local_authorization_changed: "本机安装计划授权已变化，请重新准备。",
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
    exe_backup_required: "此补丁必须保留原文件备份，请启用备份后重试。",
    exe_insufficient_space: "检查或安装此补丁所需空间不足，请清理空间后重试。",
    exe_sandbox_unavailable: "安全提取环境不可用，暂不能自动安装此补丁。",
    game_not_ready: "游戏尚未完整安装或库不可访问，请检查 Steam 下载状态。",
    signed_catalog_required: "需要有效的签名目录才能安装。",
    catalog_expired: "签名目录已到期，请更新目录后重新生成计划。",
    catalog_rollback: "目录版本低于本机已验证版本，请使用较新的可信目录。",
    release_revoked: "此补丁版本已被维护端撤销，不能安装。",
    plan_expired: "安装计划已过期，请重新预检。",
    plan_authorization_changed: "目录授权或配方已变化，请重新生成计划。",
    plan_has_blockers: "安装计划存在阻止条件，请先处理列出的问题。",
    game_closed_confirmation_required: "请关闭游戏并暂停 Steam 更新后确认。",
    game_changed_or_unavailable: "游戏版本或库状态已变化，请重新扫描和预检。",
    installer_busy: "另一项安装或恢复正在执行，请等待完成。",
    game_busy: "此游戏正在被另一项事务处理，请等待完成。",
    insufficient_space: "可用磁盘空间不足，请清理空间后重新预检。",
    managed_install_exists_or_needs_recovery: "已有托管安装或待恢复事务，请先卸载或恢复。",
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
    ,download_cancelled: "已取消获取或校验，没有执行安装。"
    ,download_http_error: "下载服务返回错误，请稍后重试。"
    ,download_network_error: "补丁下载网络连接失败，请检查网络后重试。"
    ,download_not_zip: "下载内容不是有效 ZIP，未执行安装。"
    ,local_archive_search_limit: "下载目录文件过多，请使用手动 ZIP 备用方式。"
  };
  return labels[normalized] ?? (normalized.startsWith("exe_") ? "此 EXE 补丁暂不能安全安装，请查看来源说明或稍后重试。" : normalized.replaceAll("_", " "));
}

export function workflowState(value: string): string {
  const labels: Record<string, string> = {
    discover_only: "查看补丁来源", manual_download: "手动下载导入", managed_zip: "审核 ZIP 安装", unsupported: "暂不支持",
    publisher_zip: "ZIP · 默认解压至游戏根目录", publisher_exe: "自动提取并安装", publisher_default: "按根目录生成计划，效果待验证",
    check_required: "安装时自动检查条件", conditions_checked: "安装条件已核验", verified_metadata: "目录条件已核对", needs_review: "尚待审核", signed: "已验证签名", unsigned_development: "未签名开发目录",
    planned: "已生成计划", backing_up: "正在备份", backed_up: "备份完成", applying: "正在写入", committed: "安装完成",
    rolling_back: "正在回滚", rolled_back: "已回滚", uninstalling: "正在卸载", uninstalled: "已卸载", needs_recovery: "需要恢复",
    add: "新增", replace: "替换"
  };
  return labels[value] ?? value;
}

export function patchState(status: string): string {
  const labels: Record<string, string> = {
    absent: "未检测到", missing: "未检测到", not_detected: "未检测到", managed: "已安装 / 托管",
    installed_managed: "已安装 / 托管", installed_owned: "已安装 / 托管",
    external: "已安装 / 外部", installed_external: "已安装 / 外部",
    partial: "部分安装", conflict: "冲突", partial_or_conflict: "部分安装或文件冲突", unknown: "状态未知",
    needs_revalidation: "需重新验证"
  };
  return labels[status] ?? status;
}
