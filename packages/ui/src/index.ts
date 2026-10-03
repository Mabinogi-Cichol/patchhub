export type Patch = {
  release_id: string;
  name: string;
  source_page: string;
  source_id?: string;
  source_name?: string;
  source_status?: string;
  source_cached_at?: number | null;
  support_level: string;
  download_available?: boolean;
  compatibility: string;
  blockers: string[];
  detection: { status: string; files: { path: string; status: string }[] };
};

export type Install = {
  game: {
    instance_id: string;
    appid: number;
    name: string;
    library_path: string;
    install_path: string;
    buildid: string | null;
    availability: string;
    kind: string;
  };
  match_status: string;
  patches: Patch[];
};

export type ScanResult = {
  roots: string[];
  catalog_revision: number;
  catalog_trust: string;
  platform: string;
  installs: Install[];
  diagnostics: { code: string; path: string; message: string }[];
  sources?: SourceSubscription[];
};

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
};

export type Filter = "all" | "matched" | "installed" | "attention";

export function isManaged(status: string): boolean {
  return ["managed", "installed_managed", "installed_owned"].includes(status);
}

export function isExternal(status: string): boolean {
  return ["external", "installed_external"].includes(status);
}

export function needsAttention(row: Install): boolean {
  return row.game.availability !== "installed" || row.patches.some(p =>
    p.blockers.length > 0 || ["partial", "conflict", "partial_or_conflict", "unknown", "needs_revalidation"].includes(p.detection.status)
  );
}

export function filterInstalls(rows: Install[], filter: Filter, query: string): Install[] {
  const search = query.trim().toLocaleLowerCase();
  return rows.filter(row => {
    if (search && !`${row.game.name} ${row.game.appid}`.toLocaleLowerCase().includes(search)) return false;
    if (filter === "matched") return row.patches.length > 0;
    if (filter === "installed") return row.patches.some(p => isManaged(p.detection.status) || isExternal(p.detection.status));
    if (filter === "attention") return needsAttention(row);
    return true;
  });
}

export function reason(code: string): string {
  const normalized = code.trim().replace(/^(Error|RuntimeError):\s*/, "");
  const labels: Record<string, string> = {
    source_refreshed: "已实时查询",
    source_not_modified: "官网内容未变化",
    source_offline_cached: "查询失败，使用上次成功查询的缓存",
    source_refresh_failed: "补丁源查询失败，暂无可用缓存",
    cached: "已缓存，等待实时查询",
    not_refreshed: "尚未查询",
    invalid_source_cache: "源缓存无效，请重新查询",
    source_not_enabled: "补丁源已停用，请重新订阅后查询",
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
    archive_required: "请先导入从官方来源下载的 ZIP。",
    artifact_requires_manual_download: "此来源需要浏览器下载；打开官方来源后导入 ZIP。",
    build_not_verified: "当前游戏 build 尚未审核，自动安装已禁用。",
    platform_not_verified: "当前平台尚未审核，自动安装已禁用。",
    source_requires_guidance: "此条目提供官方来源引导，尚无可安装配方。",
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
    artifact_hash_mismatch: "ZIP 哈希与已审核配方不符，请重新核对官方下载包。",
    artifact_size_mismatch: "ZIP 大小与已审核记录不符，请确认下载完整。",
    state_overlaps_game: "状态目录与游戏目录重叠，已阻止写入。",
    backup_missing_or_changed: "原版备份丢失或已变化，无法安全还原。",
    archive_outside_downloads: "请将 ZIP 放入 Deck 用户的 Downloads 目录再导入。",
    invalid_archive: "所选文件不是可读取的普通 ZIP。"
    ,source_not_released: "官方页面尚未发布此补丁，暂不能下载。"
    ,publisher_prepare_first: "请先自动下载并准备安装，获取文件变更计划。"
    ,publisher_state_unavailable: "本机官方 ZIP 安装记录不可读取，请检查状态目录。"
    ,publisher_existing_transaction: "已有安装或待恢复记录，请先卸载或修复。"
    ,publisher_match_changed: "官方游戏匹配已变化，请重新扫描。"
    ,download_cancelled: "已取消获取或校验，没有执行安装。"
    ,download_http_error: "官方下载服务返回错误，请稍后重试。"
    ,download_network_error: "补丁下载网络连接失败，请检查网络后重试。"
    ,download_not_zip: "下载内容不是有效 ZIP，未执行安装。"
    ,local_archive_search_limit: "下载目录文件过多，请使用手动 ZIP 备用方式。"
  };
  return labels[normalized] ?? normalized.replaceAll("_", " ");
}

export function workflowState(value: string): string {
  const labels: Record<string, string> = {
    discover_only: "官方来源引导", manual_download: "手动下载导入", managed_zip: "审核 ZIP 安装", unsupported: "暂不支持",
    publisher_zip: "官方 ZIP · 默认解压至游戏根目录", publisher_default: "按根目录生成计划，效果待验证",
    verified_metadata: "目录条件已核对", needs_review: "尚待审核", signed: "已验证签名", unsigned_development: "未签名开发目录",
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
