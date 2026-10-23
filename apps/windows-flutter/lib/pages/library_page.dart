import 'package:flutter/material.dart';

import '../app.dart';
import '../host/steam_artwork.dart';
import '../presentation/game_artwork.dart';
import '../presentation/game_status.dart';
import '../presentation/desktop_toolbar.dart';
import '../models/app_model.dart';
import '../models/library.dart';
import '../presentation/library_presentation.dart';
import 'patch_management_page.dart';
import 'records_page.dart';
import 'source_pages.dart';

class LibraryPage extends StatefulWidget {
  const LibraryPage({super.key, required this.model, this.artwork});
  final AppModel model;
  final SteamArtworkStore? artwork;
  @override
  State<LibraryPage> createState() => _LibraryPageState();
}

class _LibraryPageState extends State<LibraryPage> {
  GameFilter filter = GameFilter.matched;
  String query = '';
  String? selectedId;
  String? selectedKey;
  String? mirror;
  String? selectionNotice;
  bool globalSources = false;
  int tab = 0;
  final scroll = ScrollController();
  late final artwork = widget.artwork ?? SteamArtworkStore();
  ScanSnapshot? artworkSnapshot;

  @override
  void initState() {
    super.initState();
    widget.model.addListener(changed);
    widget.model.initialize();
  }

  @override
  void dispose() {
    widget.model.removeListener(changed);
    scroll.dispose();
    super.dispose();
  }

  void changed() {
    if (artworkSnapshot != widget.model.snapshot) {
      artworkSnapshot = widget.model.snapshot;
      artwork.refresh();
    }
    final row = widget.model.snapshot?.rows
        .where((row) => row.id == selectedId)
        .firstOrNull;
    if (selectedId != null && row == null) {
      selectedId = null;
      selectedKey = null;
      mirror = null;
      tab = 0;
    } else if (row != null) {
      final entries = displayEntries(row, widget.model.preferredLanguage);
      if (selectedKey != null &&
          !entries.any((entry) => entry.key == selectedKey)) {
        selectedKey = entries.firstOrNull?.key;
        mirror = null;
        selectionNotice = '原补丁已不在当前来源结果中，请重新核对选择。';
      }
      final entry = entries
          .where((entry) => entry.key == selectedKey)
          .firstOrNull;
      if (mirror != null &&
          entry != null &&
          !entry.choices.any(
            (patch) => (patch['selection_id'] ?? patch['release_id']) == mirror,
          )) {
        mirror = null;
        selectionNotice = '原下载项已变化，请重新核对选择。';
      }
    }
    if (mounted) setState(() {});
  }

  void selectGame(GameRow row) => setState(() {
    selectedId = row.id;
    selectedKey = displayEntries(
      row,
      widget.model.preferredLanguage,
    ).firstOrNull?.key;
    mirror = null;
    tab = 0;
    globalSources = false;
    selectionNotice = null;
  });
  void selectEntry(String key) => setState(() {
    if (selectedKey != key) mirror = null;
    selectedKey = key;
    tab = 1;
    selectionNotice = null;
  });

  Future<void> showIssues(List<(String, Object)> issues) => showDialog<void>(
    context: context,
    builder: (context) => AlertDialog(
      title: const Text('读取与刷新详情'),
      content: SizedBox(
        width: 600,
        child: SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              for (final (title, failure) in issues) ...[
                Text(title, style: Theme.of(context).textTheme.titleMedium),
                const SizedBox(height: 8),
                SelectableText('$failure'),
                const SizedBox(height: 16),
              ],
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('关闭'),
        ),
      ],
    ),
  );

  @override
  Widget build(BuildContext context) {
    final model = widget.model;
    final issues = <(String, Object)>[
      if (model.stateRefreshError case final failure?) ('状态刷新失败', failure),
      if (model.error case final failure?) ('游戏库读取失败', failure),
      if (model.snapshot?.catalogError case final failure?)
        ('补丁目录不可用', failure),
    ];
    final rows = model.snapshot?.rows ?? <GameRow>[];
    final visible = filterGames(rows, filter, query);
    final row = rows.where((row) => row.id == selectedId).firstOrNull;
    final scheme = Theme.of(context).colorScheme;
    final width = MediaQuery.sizeOf(context).width;
    return Scaffold(
      body: Column(
        children: [
          DesktopToolbar(
            actions: [
              TextButton.icon(
                onPressed: () => setState(() => globalSources = !globalSources),
                icon: Icon(
                  globalSources ? Icons.arrow_back : Icons.folder_open_outlined,
                  size: 20,
                ),
                label: Text(globalSources ? '返回游戏' : '全局来源'),
              ),
              PopupMenuButton<ThemeMode>(
                tooltip: '外观',
                icon: const Icon(Icons.tune_outlined, size: 20),
                onSelected: (mode) => SteamPatchApp.setThemeMode(context, mode),
                itemBuilder: (context) => [
                  for (final entry in const {
                    ThemeMode.system: '跟随系统',
                    ThemeMode.light: '浅色',
                    ThemeMode.dark: '深色',
                  }.entries)
                    CheckedPopupMenuItem(
                      value: entry.key,
                      checked: SteamPatchApp.themeModeOf(context) == entry.key,
                      child: Text(entry.value),
                    ),
                ],
              ),
            ],
          ),
          Divider(height: 1, color: scheme.outlineVariant),
          if (issues.isNotEmpty)
            MaterialBanner(
              content: Text(issues.map((issue) => issue.$1).join('；')),
              actions: [
                TextButton(
                  onPressed: () => showIssues(issues),
                  child: const Text('查看原因'),
                ),
                TextButton(
                  onPressed: model.scanning ? null : model.refresh,
                  child: const Text('重新读取'),
                ),
              ],
            ),
          Expanded(
            child: Row(
              children: [
                SizedBox(
                  width: width < 1050
                      ? 228
                      : width >= 1440
                      ? 296
                      : 272,
                  child: ColoredBox(
                    color: scheme.surface,
                    child: LayoutBuilder(
                      builder: (context, sidebar) => Column(
                        children: [
                          ConstrainedBox(
                            constraints: BoxConstraints(
                              maxHeight: sidebar.maxHeight * 0.65,
                            ),
                            child: SingleChildScrollView(
                              child: Column(
                                mainAxisSize: MainAxisSize.min,
                                children: [
                                  Padding(
                                    padding: const EdgeInsets.all(16),
                                    child: TextField(
                                      decoration: const InputDecoration(
                                        hintText: '搜索名称或 AppID',
                                        prefixIcon: Icon(
                                          Icons.search,
                                          size: 20,
                                        ),
                                      ),
                                      onChanged: (value) =>
                                          setState(() => query = value),
                                    ),
                                  ),
                                  for (final value in [
                                    GameFilter.matched,
                                    GameFilter.installed,
                                    GameFilter.all,
                                    GameFilter.attention,
                                  ])
                                    Padding(
                                      padding: const EdgeInsets.fromLTRB(
                                        12,
                                        0,
                                        12,
                                        4,
                                      ),
                                      child: Material(
                                        color: filter == value
                                            ? scheme.primaryContainer
                                            : Colors.transparent,
                                        borderRadius: BorderRadius.circular(10),
                                        child: InkWell(
                                          borderRadius: BorderRadius.circular(
                                            10,
                                          ),
                                          onTap: () =>
                                              setState(() => filter = value),
                                          child: Padding(
                                            padding: const EdgeInsets.symmetric(
                                              horizontal: 12,
                                              vertical: 8,
                                            ),
                                            child: Row(
                                              children: [
                                                Icon(
                                                  switch (value) {
                                                    GameFilter.matched =>
                                                      Icons.science_outlined,
                                                    GameFilter.installed =>
                                                      Icons.task_alt,
                                                    GameFilter.all =>
                                                      Icons.grid_view_rounded,
                                                    GameFilter.attention =>
                                                      Icons.error_outline,
                                                  },
                                                  size: 18,
                                                  color: filter == value
                                                      ? scheme
                                                            .onPrimaryContainer
                                                      : scheme.onSurfaceVariant,
                                                ),
                                                const SizedBox(width: 10),
                                                Expanded(
                                                  child: Text(
                                                    value.label,
                                                    style: TextStyle(
                                                      fontSize: 14,
                                                      fontWeight:
                                                          filter == value
                                                          ? FontWeight.w600
                                                          : FontWeight.w400,
                                                    ),
                                                  ),
                                                ),
                                                const SizedBox(width: 4),
                                                Text(
                                                  '${filterGames(rows, value, query).length}',
                                                  style: TextStyle(
                                                    fontSize: 12,
                                                    color:
                                                        scheme.onSurfaceVariant,
                                                  ),
                                                ),
                                              ],
                                            ),
                                          ),
                                        ),
                                      ),
                                    ),
                                  const SizedBox(height: 8),
                                ],
                              ),
                            ),
                          ),
                          if (model.scanning)
                            const LinearProgressIndicator(minHeight: 2),
                          Expanded(
                            child: visible.isEmpty
                                ? SingleChildScrollView(
                                    child: Padding(
                                      padding: const EdgeInsets.all(16),
                                      child: Column(
                                        mainAxisSize: MainAxisSize.min,
                                        children: [
                                          const Icon(
                                            Icons.videogame_asset_off_outlined,
                                            size: 32,
                                          ),
                                          const SizedBox(height: 12),
                                          Text(
                                            model.snapshot == null
                                                ? (model.scanning
                                                      ? '正在扫描游戏库'
                                                      : '尚未扫描')
                                                : rows.isEmpty
                                                ? '未发现已安装游戏'
                                                : '没有符合条件的游戏',
                                          ),
                                          if (filter != GameFilter.all &&
                                              rows.isNotEmpty)
                                            TextButton(
                                              onPressed: () => setState(
                                                () => filter = GameFilter.all,
                                              ),
                                              child: const Text('查看全部游戏'),
                                            ),
                                        ],
                                      ),
                                    ),
                                  )
                                : ListView.builder(
                                    controller: scroll,
                                    padding: const EdgeInsets.symmetric(
                                      horizontal: 10,
                                    ),
                                    itemCount: visible.length,
                                    itemBuilder: (context, index) {
                                      final item = visible[index];
                                      final multiple =
                                          rows
                                              .where(
                                                (r) =>
                                                    r.game['appid'] ==
                                                    item.game['appid'],
                                              )
                                              .length >
                                          1;
                                      final selected = selectedId == item.id;
                                      return Padding(
                                        padding: const EdgeInsets.only(
                                          bottom: 4,
                                        ),
                                        child: Material(
                                          color: selected
                                              ? scheme.primaryContainer
                                                    .withValues(alpha: 0.65)
                                              : Colors.transparent,
                                          shape: RoundedRectangleBorder(
                                            borderRadius: BorderRadius.circular(
                                              12,
                                            ),
                                            side: BorderSide(
                                              color: selected
                                                  ? scheme.secondary
                                                  : Colors.transparent,
                                            ),
                                          ),
                                          clipBehavior: Clip.antiAlias,
                                          child: InkWell(
                                            onTap: () => selectGame(item),
                                            child: Padding(
                                              padding: const EdgeInsets.all(8),
                                              child: Row(
                                                children: [
                                                  ClipRRect(
                                                    borderRadius:
                                                        BorderRadius.circular(
                                                          8,
                                                        ),
                                                    child: SizedBox(
                                                      width: 48,
                                                      height: 54,
                                                      child: GameArtwork(
                                                        artwork: artwork.load(
                                                          item.game['appid']
                                                              as int,
                                                        ),
                                                      ),
                                                    ),
                                                  ),
                                                  const SizedBox(width: 10),
                                                  Expanded(
                                                    child: Column(
                                                      crossAxisAlignment:
                                                          CrossAxisAlignment
                                                              .start,
                                                      children: [
                                                        Text(
                                                          item.name,
                                                          maxLines: 2,
                                                          overflow: TextOverflow
                                                              .ellipsis,
                                                          style: TextStyle(
                                                            fontSize: 14,
                                                            fontWeight: selected
                                                                ? FontWeight
                                                                      .w600
                                                                : FontWeight
                                                                      .w400,
                                                          ),
                                                        ),
                                                        const SizedBox(
                                                          height: 4,
                                                        ),
                                                        GameStatus(row: item),
                                                        if (multiple)
                                                          Text(
                                                            '${item.game['library_path']}',
                                                            style: TextStyle(
                                                              fontSize: 12,
                                                              color: scheme
                                                                  .onSurfaceVariant,
                                                            ),
                                                          ),
                                                      ],
                                                    ),
                                                  ),
                                                ],
                                              ),
                                            ),
                                          ),
                                        ),
                                      );
                                    },
                                  ),
                          ),
                          Padding(
                            padding: const EdgeInsets.all(12),
                            child: OutlinedButton.icon(
                              onPressed: model.scanning ? null : model.refresh,
                              icon: const Icon(Icons.refresh, size: 18),
                              label: Text(
                                model.scanning ? '正在扫描' : '重新扫描游戏库',
                                style: const TextStyle(fontSize: 13),
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ),
                const VerticalDivider(width: 1),
                Expanded(
                  child: IndexedStack(
                    index: globalSources ? 1 : 0,
                    children: [
                      row == null
                          ? Center(
                              child: Column(
                                mainAxisSize: MainAxisSize.min,
                                children: [
                                  Icon(
                                    Icons.sports_esports_outlined,
                                    size: 56,
                                    color: scheme.onSurfaceVariant,
                                  ),
                                  const SizedBox(height: 16),
                                  Text(
                                    '从游戏库选择游戏',
                                    style: Theme.of(context)
                                        .textTheme
                                        .titleLarge,
                                  ),
                                  const SizedBox(height: 8),
                                  Text(
                                    '查看补丁、安装状态与游戏订阅',
                                    style: TextStyle(
                                      color: scheme.onSurfaceVariant,
                                    ),
                                  ),
                                ],
                              ),
                            )
                          : gameDetails(row),
                      GlobalSourcesPage(model: model),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget gameDetails(GameRow row) {
    final entries = displayEntries(row, widget.model.preferredLanguage);
    final entry =
        entries.where((entry) => entry.key == selectedKey).firstOrNull ??
        entries.firstOrNull;
    final patch =
        entry?.choices
            .where(
              (patch) =>
                  (patch['selection_id'] ?? patch['release_id']) == mirror,
            )
            .firstOrNull ??
        entry?.patch;
    final candidate =
        entry?.candidate ??
        ((row.discovery?['candidates'] as List?) ?? [])
            .cast<Map<String, dynamic>>()
            .where((value) => value['candidate_id'] == patch?['candidate_id'])
            .firstOrNull;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        LayoutBuilder(
          builder: (context, constraints) {
            final scheme = Theme.of(context).colorScheme;
            final compact = constraints.maxWidth < 800;
            return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SizedBox(
                  height: compact ? 112 : 158,
                  child: Stack(
                    fit: StackFit.expand,
                    children: [
                      GameArtwork(
                        artwork: artwork.load(row.game['appid'] as int),
                        kind: ArtworkKind.hero,
                      ),
                      DecoratedBox(
                        decoration: BoxDecoration(
                          gradient: LinearGradient(
                            colors: [
                              scheme.surface,
                              scheme.surface.withValues(alpha: 0.85),
                              scheme.surface.withValues(alpha: 0.15),
                            ],
                            stops: const [0, 0.45, 1],
                          ),
                        ),
                      ),
                      DecoratedBox(
                        decoration: BoxDecoration(
                          gradient: LinearGradient(
                            begin: Alignment.topCenter,
                            end: Alignment.bottomCenter,
                            colors: [
                              Colors.transparent,
                              Theme.of(context).scaffoldBackgroundColor,
                            ],
                          ),
                        ),
                      ),
                      Padding(
                        padding: EdgeInsets.fromLTRB(
                          compact ? 20 : 28,
                          20,
                          24,
                          12,
                        ),
                        child: SingleChildScrollView(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Wrap(
                                spacing: 12,
                                runSpacing: 8,
                                crossAxisAlignment: WrapCrossAlignment.center,
                                children: [
                                  SelectionArea(
                                    child: Text(
                                      row.name,
                                      style: TextStyle(
                                        fontSize: compact ? 24 : 32,
                                        fontWeight: FontWeight.w700,
                                        height: 1.25,
                                        letterSpacing: -0.5,
                                      ),
                                    ),
                                  ),
                                  GameStatus(row: row, emphasized: true),
                                ],
                              ),
                              if (entry != null) ...[
                                const SizedBox(height: 6),
                                Text(
                                  '${sourceName((patch?['source_id'] ?? candidate?['source_id']) as String?, widget.model.sources)} · '
                                  '${languageLabel(candidate?['languages'] as List?)} · ${formatLabel(candidate, patch)}',
                                  style: TextStyle(
                                    color: scheme.onSurfaceVariant,
                                    fontSize: 13,
                                  ),
                                ),
                              ],
                              if (selectionNotice != null)
                                Text(
                                  selectionNotice!,
                                  style: TextStyle(color: scheme.error),
                                ),
                            ],
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
                Padding(
                  padding: const EdgeInsets.fromLTRB(20, 0, 20, 8),
                  child: SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    child: SegmentedButton<int>(
                      showSelectedIcon: false,
                      style: SegmentedButton.styleFrom(
                        selectedBackgroundColor: scheme.primaryContainer,
                        selectedForegroundColor: scheme.onPrimaryContainer,
                        side: BorderSide(color: scheme.outlineVariant),
                        visualDensity: VisualDensity.compact,
                        shape: RoundedRectangleBorder(
                          borderRadius: BorderRadius.circular(12),
                        ),
                      ),
                      segments: const [
                        ButtonSegment(
                          value: 0,
                          icon: Icon(Icons.explore_outlined, size: 18),
                          label: Text('概览'),
                        ),
                        ButtonSegment(
                          value: 1,
                          icon: Icon(Icons.build_outlined, size: 18),
                          label: Text('补丁管理'),
                        ),
                        ButtonSegment(
                          value: 2,
                          icon: Icon(Icons.receipt_long_outlined, size: 18),
                          label: Text('操作记录'),
                        ),
                        ButtonSegment(
                          value: 3,
                          icon: Icon(Icons.bookmark_outline, size: 18),
                          label: Text('游戏订阅'),
                        ),
                      ],
                      selected: {tab},
                      onSelectionChanged: (value) =>
                          setState(() => tab = value.single),
                    ),
                  ),
                ),
              ],
            );
          },
        ),

        Expanded(
          child: IndexedStack(
            index: tab,
            children: [
              overview(row, entries),
              if (entry == null)
                const Center(child: Text('当前没有补丁条目，请查看游戏订阅并重新匹配'))
              else
                PatchManagementPage(
                  key: ValueKey(
                    '${row.id}:${entry.key}:${patch?['selection_id'] ?? patch?['release_id']}',
                  ),
                  model: widget.model,
                  artwork: artwork.load(row.game['appid'] as int),
                  row: row,
                  entry: entry,
                  entries: entries,
                  patch: patch,
                  onEntry: selectEntry,
                  onMirror: (value) => setState(() {
                    mirror = value;
                    selectionNotice = null;
                  }),
                  onRecords: () => setState(() => tab = 2),
                ),
              RecordsPage(
                key: ValueKey('${row.id}:records'),
                model: widget.model,
                row: row,
              ),
              GameSourcesPage(
                key: ValueKey('${row.id}:subscriptions'),
                model: widget.model,
                row: row,
              ),
            ],
          ),
        ),
      ],
    );
  }

  Widget overview(GameRow row, List<DisplayEntry> entries) => ListView(
    key: PageStorageKey('${row.id}:overview'),
    padding: const EdgeInsets.all(20),
    children: [
      if (widget.model.transactions.any(
        (record) => record['instance_id'] == row.id && needsRecovery(record),
      ))
        Card(
          child: ListTile(
            title: const Text('此游戏存在未完成操作'),
            trailing: TextButton(
              onPressed: () => setState(() => tab = 2),
              child: const Text('查看异常记录'),
            ),
          ),
        ),
      for (final diagnostic in row.value['diagnostics'] as List)
        SelectableText('${diagnostic['message']}'),
      for (final diagnostic in row.discovery?['diagnostics'] as List? ?? [])
        SelectableText(
          '${diagnostic['code']}${diagnostic['detail'] == null ? '' : '：${diagnostic['detail']}'}',
        ),
      if (entries.where((entry) => entry.installable).isNotEmpty)
        Text('可安装补丁', style: Theme.of(context).textTheme.titleMedium),
      for (final entry in entries.where((entry) => entry.installable))
        entryCard(row, entry),
      if (entries.where((entry) => !entry.installable).isNotEmpty) ...[
        const SizedBox(height: 16),
        Text('待核对的来源', style: Theme.of(context).textTheme.titleMedium),
        for (final entry in entries.where((entry) => !entry.installable))
          entryCard(row, entry),
      ],
      if (entries.isEmpty) ...[
        const Text('已启用来源中未找到补丁；这不代表游戏无需补丁。'),
        Wrap(
          spacing: 12,
          children: [
            OutlinedButton(
              onPressed: () => widget.model.rematch(row),
              child: const Text('重新匹配'),
            ),
            TextButton(
              onPressed: () => setState(() => tab = 3),
              child: const Text('查看游戏订阅'),
            ),
          ],
        ),
      ],
      ExpansionTile(
        title: const Text('游戏信息'),
        children: [
          Padding(
            padding: const EdgeInsets.all(16),
            child: SelectableText(
              'AppID：${row.game['appid']}\nbuild：${row.game['buildid'] ?? '未知'}\n安装路径：${row.game['install_path']}\n库位置：${row.game['library_path']}',
            ),
          ),
        ],
      ),
    ],
  );

  Widget entryCard(GameRow row, DisplayEntry entry) {
    final candidate =
        entry.candidate ??
        ((row.discovery?['candidates'] as List?) ?? [])
            .cast<Map<String, dynamic>>()
            .where(
              (candidate) =>
                  candidate['candidate_id'] == entry.patch?['candidate_id'],
            )
            .firstOrNull;
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(entry.title, style: Theme.of(context).textTheme.titleMedium),
            Text(
              '${sourceName((entry.patch?['source_id'] ?? candidate?['source_id']) as String?, widget.model.sources)} · '
              '${languageLabel(candidate?['languages'] as List?)} · ${formatLabel(candidate, entry.patch)}',
            ),
            Text(candidateConditions(candidate)),
            TextButton(
              onPressed: () => selectEntry(entry.key),
              child: Text(entry.installable ? '查看并安装' : '查看条件与来源'),
            ),
          ],
        ),
      ),
    );
  }
}
