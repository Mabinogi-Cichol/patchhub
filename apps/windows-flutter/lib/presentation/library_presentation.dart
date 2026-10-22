import '../models/library.dart';

enum GameFilter {
  matched('有补丁'),
  all('全部游戏'),
  installed('已安装补丁'),
  attention('需处理');

  const GameFilter(this.label);
  final String label;
}

bool installedStatus(dynamic status) =>
    ['installed_managed', 'installed_external'].contains(status);
bool needsAttention(GameRow row) =>
    row.game['availability'] != 'installed' ||
    row.patches.any(
      (patch) =>
          (patch['blockers'] as List).isNotEmpty ||
          patch['detection']['status'] == 'partial_or_conflict' ||
          (patch['detection']['status'] == 'unknown' &&
              patch['support_level'] != 'discover_only'),
    );

List<GameRow> filterGames(List<GameRow> rows, GameFilter filter, String query) {
  final search = query.trim().toLowerCase();
  return rows.where((row) {
    if (search.isNotEmpty &&
        !'${row.game['name']} ${row.game['localized_name'] ?? ''} ${row.game['appid']}'
            .toLowerCase()
            .contains(search)) {
      return false;
    }
    return switch (filter) {
      GameFilter.all => true,
      GameFilter.matched => row.matched,
      GameFilter.installed => row.patches.any(
        (patch) => installedStatus(patch['detection']['status']),
      ),
      GameFilter.attention => needsAttention(row),
    };
  }).toList();
}

String rowSummary(GameRow row) {
  if (row.patches.any(
    (patch) => installedStatus(patch['detection']['status']),
  )) {
    return '已安装补丁';
  }
  if (row.patches.any(
    (patch) =>
        patch['actions']['overwrite']['available'] == true &&
        patch['detection']['status'] == 'partial_or_conflict',
  )) {
    return '补丁文件不完整，可覆盖安装';
  }
  if (row.patches.any(
    (patch) =>
        patch['actions']['automatic']['available'] == true &&
        patch['compatibility'] != 'manual_selection_required',
  )) {
    return '有可用补丁';
  }
  if (row.patches.any(
    (patch) =>
        patch['actions']['automatic']['available'] == true &&
        patch['compatibility'] == 'manual_selection_required',
  )) {
    return '可手动选择补丁';
  }
  if (row.matched) return '找到补丁来源';
  return switch (row.discovery?['status']) {
    'publisher_match' => '找到补丁来源',
    'fallback_match' => '补丁索引（HGamer）找到来源',
    'no_match' => '已启用来源中未找到补丁',
    'incomplete' => '来源覆盖不完整，尚需更新来源',
    'failed' => '来源查询失败',
    _ => '尚未匹配，请重新匹配',
  };
}

class DisplayEntry {
  DisplayEntry.installable(this.key, this.patch, this.choices)
    : candidate = null;
  DisplayEntry.discovery(this.key, this.candidate)
    : patch = null,
      choices = const [];
  final String key;
  final Map<String, dynamic>? patch;
  final List<Map<String, dynamic>> choices;
  final Map<String, dynamic>? candidate;
  bool get installable => patch != null;
  String get title => (patch?['name'] ?? candidate!['patch_title']) as String;
}

/// Port of packages/ui displayPatches: preserve candidates, variants and mirrors.
List<DisplayEntry> displayEntries(GameRow row, [String? preferredLanguage]) {
  final candidates = ((row.discovery?['candidates'] as List?) ?? [])
      .cast<Map<String, dynamic>>();
  final discovery = candidates
      .where(
        (candidate) =>
            (row.discovery?['status'] != 'publisher_match' ||
                candidate['source_id'] != 'hgamer') &&
            !row.patches.any(
              (patch) => patch['candidate_id'] == candidate['candidate_id'],
            ),
      )
      .toList();
  final discoveryOrder = {
    for (var i = 0; i < discovery.length; i++) discovery[i]: i,
  };
  discovery.sort((a, b) {
    int rank(Map<String, dynamic> c) =>
        (c['source_id'] == 'hgamer' ? 2 : 0) +
        (preferredLanguage != null &&
                !(c['languages'] as List).contains(preferredLanguage)
            ? 1
            : 0);
    final compare = rank(a).compareTo(rank(b));
    return compare != 0
        ? compare
        : discoveryOrder[a]!.compareTo(discoveryOrder[b]!);
  });
  final patches = row.patches
      .where(
        (patch) =>
            patch['compatibility'] == 'manual_selection_required' ||
            patch['actions']['overwrite']['available'] == true ||
            !row.patches.any(
              (other) =>
                  other['candidate_id'] == patch['candidate_id'] &&
                  other['compatibility'] == 'manual_selection_required' &&
                  other['actions']['overwrite']['available'] == true,
            ),
      )
      .toList();
  final order = {for (var i = 0; i < patches.length; i++) patches[i]: i};
  patches.sort((a, b) {
    int rank(Map<String, dynamic> patch) {
      final candidate = candidates
          .where((c) => c['candidate_id'] == patch['candidate_id'])
          .firstOrNull;
      return switch (candidate?['variant']) {
        'base' => 0,
        'unknown' => 1,
        _ => 2,
      };
    }

    final compare = rank(a).compareTo(rank(b));
    return compare != 0 ? compare : order[a]!.compareTo(order[b]!);
  });
  final choices = patches.where(
    (patch) =>
        patch['compatibility'] != 'manual_selection_required' ||
        patch['download_url'] == null ||
        patch['download_url'] == '' ||
        !patches.any(
          (other) =>
              other['candidate_id'] == patch['candidate_id'] &&
              other['compatibility'] != 'manual_selection_required' &&
              other['actions']['overwrite']['available'] == true &&
              other['download_url'] == patch['download_url'],
        ),
  );
  final groups = <String, List<Map<String, dynamic>>>{};
  for (final patch in choices) {
    final key = patch['candidate_id'] != null && patch['candidate_id'] != ''
        ? 'candidate:${patch['source_id'] ?? ''}:${patch['candidate_id']}'
        : (patch['selection_id'] ?? patch['release_id']) as String;
    (groups[key] ??= []).add(patch);
  }
  return [
    for (final group in groups.entries)
      DisplayEntry.installable(group.key, {
        ...group.value.first,
        'name':
            candidates
                .where(
                  (c) => c['candidate_id'] == group.value.first['candidate_id'],
                )
                .firstOrNull?['patch_title'] ??
            group.value.first['name'],
      }, group.value),
    for (final candidate in discovery)
      DisplayEntry.discovery(candidate['candidate_id'] as String, candidate),
  ];
}

String languageLabel(List<dynamic>? languages) =>
    languages == null || languages.isEmpty
    ? '语言待确认'
    : languages
          .map(
            (code) =>
                const {
                  'zh-CN': '简体中文',
                  'zh-TW': '繁体中文',
                  'zh': '中文',
                  'en': '英文',
                  'en-US': '英文',
                  'ja': '日文',
                  'ja-JP': '日文',
                }[code] ??
                code,
          )
          .join('、');

String candidateConditions(Map<String, dynamic>? candidate) {
  if (candidate == null) return '适用版本与 DLC 条件待确认';
  return [
    const {
          'base': '游戏本体',
          'dlc': '扩展 / DLC',
          'demo': '试玩版',
          'unknown': '版本类型待确认',
        }[candidate['variant']] ??
        '版本类型待确认',
    if ((candidate['requires_dlc'] as List?)?.isNotEmpty == true)
      '需要 DLC ${(candidate['requires_dlc'] as List).join('、')}（拥有及安装状态待确认）',
    if ((candidate['excludes_dlc'] as List?)?.isNotEmpty == true)
      '仅适用于未使用 DLC ${(candidate['excludes_dlc'] as List).join('、')} 的版本',
    if (candidate['requires_dlc'] == null && candidate['excludes_dlc'] == null)
      'DLC 条件待确认',
  ].join(' · ');
}

String statusLabel(dynamic value) =>
    const {
      'unknown': '状态未知',
      'not_detected': '未检测到补丁',
      'installed_managed': '已安装（托管）',
      'installed_external': '已安装（外部）',
      'partial_or_conflict': '部分安装或文件冲突',
      'committed': '安装完成',
      'rolled_back': '已回滚',
      'needs_recovery': '需要恢复',
      'planned': '已生成计划',
      'applying': '正在写入',
      'backing_up': '正在备份',
      'rolling_back': '正在回滚',
      'failed': '未完成',
      'superseded': '已被后续操作替代',
      'uninstalled': '已卸载',
      'uninstalling': '正在卸载',
      'pending': '尚未匹配',
      'publisher_match': '发行商来源找到补丁',
      'fallback_match': '补查来源找到补丁',
      'no_match': '已启用来源中未找到补丁',
      'incomplete': '来源覆盖不完整',
      'source_refreshed': '已更新',
      'source_not_modified': '来源未变化',
      'source_partial': '部分更新',
      'source_offline_cached': '更新失败，保留缓存',
      'source_refresh_failed': '来源更新失败',
      'not_refreshed': '尚未更新',
      'cached': '已缓存',
      'task_cancelled': '任务已取消',
      'download_cancelled': '已取消获取或校验，未执行安装',
      'cancelled_incomplete':
          '安装已取消，但已写入的文件未恢复。可重新安装补丁；如需恢复游戏本体，请使用 Steam 验证游戏文件。',
      'install_incomplete':
          '安装未完成，已写入的文件未恢复。可重新安装补丁；如需恢复游戏本体，请使用 Steam 验证游戏文件。',
      'manual_selection_required': '需要人工确认适用条件',
      'check_required': '安装时检查条件',
      'verified_metadata': '目录条件已核对',
      'verified': '条件已核对',
      'outside_japan_only': '仅适用于日本以外地区，请核对来源说明',
      'dlc_unknown': 'DLC 拥有及安装状态待确认',
      'dlc_conditions_unknown': 'DLC 适用条件待确认',
      'language_unknown': '语言待确认',
      'cancelled': '已取消',
      'local_archive_signed_only': '此来源不支持导入本地 ZIP',
      'source_not_subscribed': '尚未订阅此来源',
      'source_not_enabled': '来源已停用',
      'exe_sandbox_unavailable': '本机 EXE 安装隔离不可用',
    }[value] ??
    '$value';

String sourceName(String? id, List<Map<String, dynamic>> sources) =>
    sources.where((source) => source['id'] == id).firstOrNull?['name']
        as String? ??
    id ??
    '未知来源';

String formatLabel(
  Map<String, dynamic>? candidate,
  Map<String, dynamic>? patch,
) {
  final observed = candidate?['format_observed'];
  final reported = candidate?['format_reported'];
  if (observed != null && observed != 'unknown') {
    return '$observed'.toUpperCase();
  }
  if (reported != null && reported != 'unknown') {
    return '$reported'.toUpperCase();
  }
  return switch (patch?['support_level']) {
    'publisher_exe' => 'EXE',
    'publisher_zip' || 'managed_zip' => 'ZIP',
    _ => '格式待确认',
  };
}
