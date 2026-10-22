import 'package:flutter/material.dart';

import '../app.dart';
import '../models/app_model.dart';
import '../models/library.dart';
import '../presentation/library_presentation.dart';
import 'patch_management_page.dart';
import 'records_page.dart';
import 'source_pages.dart';

class LibraryPage extends StatefulWidget {
  const LibraryPage({super.key, required this.model});
  final AppModel model;
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
    return Scaffold(
      appBar: AppBar(
        title: const Text('Steam 黄油补丁助手'),
        actions: [
          TextButton.icon(
            onPressed: () => setState(() => globalSources = !globalSources),
            icon: Icon(
              globalSources ? Icons.arrow_back : Icons.source_outlined,
            ),
            label: Text(globalSources ? '返回游戏' : '全局来源'),
          ),
          PopupMenuButton<ThemeMode>(
            tooltip: '外观',
            icon: const Icon(Icons.brightness_6_outlined),
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
          const SizedBox(width: 16),
        ],
      ),
      body: Column(
        children: [
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
                  width: 260,
                  child: Column(
                    children: [
                      Padding(
                        padding: const EdgeInsets.all(12),
                        child: TextField(
                          decoration: const InputDecoration(
                            labelText: '搜索名称或 AppID',
                            prefixIcon: Icon(Icons.search),
                            border: OutlineInputBorder(),
                          ),
                          onChanged: (value) => setState(() => query = value),
                        ),
                      ),
                      Padding(
                        padding: const EdgeInsets.symmetric(horizontal: 12),
                        child: DropdownButtonFormField<GameFilter>(
                          initialValue: filter,
                          key: ValueKey(filter),
                          isExpanded: true,
                          decoration: const InputDecoration(labelText: '筛选游戏'),
                          items: [
                            for (final value in GameFilter.values)
                              DropdownMenuItem(
                                value: value,
                                child: Text(value.label),
                              ),
                          ],
                          onChanged: (value) => setState(() => filter = value!),
                        ),
                      ),
                      if (model.scanning) const LinearProgressIndicator(),
                      Expanded(
                        child: visible.isEmpty
                            ? Center(
                                child: Padding(
                                  padding: const EdgeInsets.all(16),
                                  child: Column(
                                    mainAxisSize: MainAxisSize.min,
                                    children: [
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
                                itemCount: visible.length,
                                itemBuilder: (context, index) {
                                  final item = visible[index];
                                  final multiple =
                                      rows
                                          .where(
                                            (row) =>
                                                row.game['appid'] ==
                                                item.game['appid'],
                                          )
                                          .length >
                                      1;
                                  return ListTile(
                                    selected: selectedId == item.id,
                                    title: Text(item.name),
                                    subtitle: Text(
                                      '${rowSummary(item)}${multiple ? '\n${item.game['library_path']}' : ''}',
                                    ),
                                    onTap: () => selectGame(item),
                                  );
                                },
                              ),
                      ),
                      Padding(
                        padding: const EdgeInsets.all(12),
                        child: OutlinedButton.icon(
                          onPressed: model.scanning ? null : model.refresh,
                          icon: const Icon(Icons.refresh),
                          label: Text(model.scanning ? '正在扫描' : '重新扫描游戏库'),
                        ),
                      ),
                    ],
                  ),
                ),
                const VerticalDivider(width: 1),
                Expanded(
                  child: IndexedStack(
                    index: globalSources ? 1 : 0,
                    children: [
                      row == null
                          ? const Center(child: Text('从游戏库选择游戏'))
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
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(20, 16, 20, 8),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(row.name, style: Theme.of(context).textTheme.headlineSmall),
              Text(rowSummary(row)),
              if (selectionNotice != null) Text(selectionNotice!),
              const SizedBox(height: 12),
              SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: SegmentedButton<int>(
                  segments: const [
                    ButtonSegment(value: 0, label: Text('概览')),
                    ButtonSegment(value: 1, label: Text('补丁管理')),
                    ButtonSegment(value: 2, label: Text('操作记录')),
                    ButtonSegment(value: 3, label: Text('游戏订阅')),
                  ],
                  selected: {tab},
                  onSelectionChanged: (value) =>
                      setState(() => tab = value.single),
                ),
              ),
            ],
          ),
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
