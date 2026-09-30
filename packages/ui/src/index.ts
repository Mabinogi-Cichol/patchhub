export type Patch = {
  release_id: string;
  name: string;
  source_page: string;
  support_level: string;
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
  const labels: Record<string, string> = {
    library_unavailable: "Steam 库不可访问，请检查磁盘或 SD 卡是否已挂载。",
    instance_not_found: "游戏实例已变化，请重新扫描。",
    release_not_found: "补丁版本不在当前目录中，请重新扫描。",
    version_incompatible: "游戏版本尚未验证，不能自动安装。",
    catalog_untrusted: "补丁目录尚未通过签名验证，不能安装。",
    hash_mismatch: "导入包哈希与审核记录不符。",
    file_changed: "游戏文件已变化，请重新预览。",
    recovery_conflict: "检测到恢复冲突，已保留后续修改。",
    unsupported_method: "当前核心版本尚不支持此操作。",
    no_archive_selected: "请先选择本地 ZIP 文件。"
  };
  return labels[code] ?? code.replaceAll("_", " ");
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
