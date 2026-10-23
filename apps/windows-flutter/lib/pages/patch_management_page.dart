import 'dart:convert';

import 'package:flutter/material.dart';

import '../host/core_client.dart';
import '../host/desktop_services.dart';
import '../host/steam_artwork.dart';
import '../presentation/game_artwork.dart';
import '../models/app_model.dart';
import '../models/library.dart';
import '../presentation/library_presentation.dart';
import 'install_dialog.dart';

Map<String, Object?> selectionArguments(Map<String, dynamic> patch) =>
    patch['selection_id'] != null
    ? {'selection_id': patch['selection_id']}
    : {'release_id': patch['release_id']};

class PatchManagementPage extends StatefulWidget {
  const PatchManagementPage({
    super.key,
    required this.model,
    this.desktop = const DesktopServices(),
    required this.row,
    required this.entry,
    required this.entries,
    required this.patch,
    required this.onEntry,
    required this.onMirror,
    required this.onRecords,
    this.artwork,
  });
  final AppModel model;
  final DesktopServices desktop;
  final GameRow row;
  final DisplayEntry entry;
  final List<DisplayEntry> entries;
  final Map<String, dynamic>? patch;
  final ValueChanged<String> onEntry;
  final ValueChanged<String> onMirror;
  final VoidCallback onRecords;
  final Future<SteamArtwork>? artwork;
  @override
  State<PatchManagementPage> createState() => _PatchManagementPageState();
}

class _PatchManagementPageState extends State<PatchManagementPage> {
  bool confirmed = false;
  bool localRoute = false;
  bool reading = false;
  String? archive;
  Object? error;
  Map<String, dynamic>? preview;
  final installFocus = FocusNode();
  @override
  void dispose() {
    installFocus.dispose();
    super.dispose();
  }

  Future<void> open(String url) async {
    try {
      await widget.desktop.openSource(Uri.parse(url));
    } catch (failure) {
      if (mounted) setState(() => error = failure);
    }
  }

  Future<void> openDirectory() async {
    try {
      await widget.desktop.openGameDirectory(
        widget.row.game['install_path'] as String,
      );
    } catch (failure) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('打开游戏目录失败：$failure')));
      }
    }
  }

  Future<void> chooseArchive() async {
    try {
      final file = await widget.desktop.selectArchive();
      if (mounted && file != null) {
        setState(() {
          archive = file;
          localRoute = true;
        });
      }
    } catch (failure) {
      if (mounted) setState(() => error = failure);
    }
  }

  Future<void> readPreview() async {
    final patch = widget.patch!;
    setState(() {
      reading = true;
      error = null;
    });
    try {
      final value = await widget.model.client.call('preview', {
        'instance_id': widget.row.id,
        ...selectionArguments(patch),
      }) as Map<String, dynamic>;
      if (mounted) setState(() => preview = value);
    } catch (failure) {
      if (mounted) setState(() => error = failure);
    } finally {
      if (mounted) setState(() => reading = false);
    }
  }

  bool available(Map<String, dynamic> patch) {
    final input = localRoute
        ? 'local_archive'
        : patch['actions']['primary']['input'];
    final allowed = input == 'automatic'
        ? patch['actions']['automatic']['available'] == true
        : input == 'local_archive' &&
              patch['actions']['local_archive']['available'] == true &&
              archive != null;
    return allowed &&
        (patch['compatibility'] != 'manual_selection_required' || confirmed);
  }

  Future<void> install() async {
    try {
      final row = widget.model.snapshot?.rows
          .where((row) => row.id == widget.row.id)
          .firstOrNull;
      final identity =
          widget.patch!['selection_id'] ?? widget.patch!['release_id'];
      final latest = row?.patches
          .where(
            (patch) =>
                (patch['selection_id'] ?? patch['release_id']) == identity,
          )
          .firstOrNull;
      if (latest == null) {
        throw const CoreError('selection_stale', '补丁选择已变化，请重新选择');
      }
      if (!available(latest)) return;
      final input = localRoute
          ? 'local_archive'
          : latest['actions']['primary']['input'];
      final task = widget.model.tasks.startInstall({
        'instance_id': widget.row.id,
        ...selectionArguments(latest),
        'operation': latest['actions']['primary']['operation'],
        'cleanup_policy': 'disabled',
        if (input == 'local_archive') 'archive': archive,
      });
      final action = await showInstallDialog(
        context,
        model: widget.model,
        task: task,
        game: widget.row.name,
        patch: widget.entry.title,
      );
      if (mounted) {
        if (action == 'records') {
          widget.onRecords();
        } else {
          installFocus.requestFocus();
        }
      }
    } catch (failure) {
      if (mounted) setState(() => error = failure);
    }
  }

  @override
  Widget build(BuildContext context) {
    final patch = widget.patch;
    final candidates = ((widget.row.discovery?['candidates'] as List?) ?? [])
        .cast<Map<String, dynamic>>();
    final candidate =
        widget.entry.candidate ??
        candidates
            .where((c) => c['candidate_id'] == patch?['candidate_id'])
            .firstOrNull;
    final sourceId =
        (patch?['source_id'] ?? candidate?['source_id']) as String?;
    final url = (patch?['source_page'] ?? candidate?['source_page']) as String?;
    final installables = widget.entries
        .where((entry) => entry.installable)
        .toList();
    final input = patch == null
        ? 'unavailable'
        : localRoute
        ? 'local_archive'
        : patch['actions']['primary']['input'];
    final blockers = patch == null
        ? <dynamic>[]
        : input == 'automatic'
        ? patch['actions']['automatic']['blockers'] as List
        : input == 'local_archive'
        ? patch['actions']['local_archive']['blockers'] as List
        : patch['actions']['overwrite']['blockers'] as List;
    final busy =
        widget.model.tasks.installation?.running == true ||
        widget.model.scanning ||
        widget.model.tasks.closing;
    final details = <Widget>[
      if (patch != null && installables.length > 1)
        DropdownButtonFormField<String>(
          initialValue: widget.entry.key,
          isExpanded: true,
          decoration: const InputDecoration(labelText: '补丁'),
          items: [
            for (final entry in installables)
              DropdownMenuItem(value: entry.key, child: Text(entry.title)),
          ],
          onChanged: (value) {
            if (value != null) widget.onEntry(value);
          },
        ),
      SelectableText(
        widget.entry.title,
        style: Theme.of(context).textTheme.titleLarge
            ?.copyWith(fontWeight: FontWeight.w700),
      ),
      Wrap(
        spacing: 12,
        runSpacing: 4,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          Text(
            '${sourceName(sourceId, widget.model.sources)} · ${languageLabel(candidate?['languages'] as List?)} · ${formatLabel(candidate, patch)}',
          ),
          if (url != null)
            TextButton.icon(
              style: TextButton.styleFrom(visualDensity: VisualDensity.compact),
              onPressed: () => open(url),
              icon: const Icon(Icons.open_in_new, size: 18),
              label: const Text('来源网页'),
            ),
        ],
      ),
      SelectableText('适用条件：${candidateConditions(candidate)}'),
      if (candidate != null)
        for (final warning in candidate['warnings'] as List)
          Text(statusLabel(warning)),
      if (patch != null) ...[
        Text('当前状态：${statusLabel(patch['detection']['status'])}'),
        Text('适用性：${statusLabel(patch['compatibility'])}'),
        if (widget.entry.choices.length == 1)
          InputDecorator(
            decoration: const InputDecoration(labelText: '下载项 / 镜像'),
            child: SelectableText(
              '${patch['download_label'] ?? '来源下载'} · ${patch['download_url'] != null ? Uri.parse(patch['download_url'] as String).pathSegments.lastOrNull ?? '下载文件' : patch['name']}',
            ),
          ),
        if (widget.entry.choices.length > 1)
          DropdownButtonFormField<String>(
            initialValue:
                (patch['selection_id'] ?? patch['release_id']) as String,
            isExpanded: true,
            decoration: const InputDecoration(labelText: '下载项 / 镜像'),
            items: [
              for (final choice in widget.entry.choices)
                DropdownMenuItem(
                  value:
                      (choice['selection_id'] ?? choice['release_id'])
                          as String,
                  child: Text(
                    '${choice['download_label'] ?? '来源下载'} · ${choice['download_url'] != null ? Uri.parse(choice['download_url'] as String).pathSegments.lastOrNull ?? '下载文件' : choice['name']}',
                  ),
                ),
            ],
            onChanged: (value) {
              if (value != null) widget.onMirror(value);
            },
          ),
        if (patch['compatibility'] == 'manual_selection_required')
          CheckboxListTile(
            contentPadding: EdgeInsets.zero,
            controlAffinity: ListTileControlAffinity.leading,
            title: const Text('我已核对来源、语言与 DLC 条件，选择将此候选安装到当前游戏'),
            value: confirmed,
            onChanged: (value) => setState(() => confirmed = value!),
          ),
        for (final blocker in blockers)
          SelectableText(
            '${statusLabel(blocker['code'])}${blocker['detail'] == null ? '' : '\n${blocker['detail']}'}',
          ),
        if (input == 'local_archive') ...[
          const Text('主要安装路线需要本地 ZIP，请先选择文件。'),
          OutlinedButton(
            onPressed: busy ? null : chooseArchive,
            child: const Text('选择本地 ZIP'),
          ),
          if (archive != null) SelectableText('已选择：$archive'),
        ] else if (patch['actions']['local_archive']['available'] == true)
          ExpansionTile(
            title: const Text('使用本地 ZIP'),
            children: [
              OutlinedButton(
                onPressed: busy ? null : chooseArchive,
                child: const Text('选择本地 ZIP'),
              ),
              if (archive != null) SelectableText('已选择：$archive'),
            ],
          ),
        if (localRoute && patch['actions']['primary']['input'] == 'automatic')
          TextButton(
            onPressed: () => setState(() {
              localRoute = false;
              archive = null;
            }),
            child: const Text('改用自动下载'),
          ),
        ExpansionTile(
          title: const Text('文件与技术详情'),
          children: [
            SelectableText(
              '发行标识：${patch['release_id']}\n选择标识：${patch['selection_id'] ?? '无'}\n下载地址：${patch['download_url'] ?? '无'}',
            ),
            if (candidate != null)
              SelectableText(
                '身份依据：${candidate['match_basis']}\n版本依据：${(candidate['version_evidence'] as List).join('、')}',
              ),
            OutlinedButton(
              onPressed: reading || busy ? null : readPreview,
              child: Text(reading ? '正在读取变更' : '查看文件变更（可选，只读）'),
            ),
            if (preview != null) ...[
              for (final blocker in preview!['blockers'] as List)
                SelectableText(statusLabel(blocker)),
              for (final change in preview!['changes'] as List)
                ListTile(
                  title: SelectableText(change['path'] as String),
                  subtitle: Text('${change['action']}'),
                ),
              for (final diagnostic in preview!['diagnostics'] as List)
                SelectableText('${diagnostic['message']}'),
            ],
          ],
        ),
      ] else ...[
        const SizedBox(height: 16),
        const Text('此条目是待核对的来源线索，不能据此启动安装。请核对来源网页的语言、版本和 DLC 条件。'),
        if (candidate != null)
          ExpansionTile(
            title: const Text('来源技术详情'),
            children: [SelectableText(jsonEncode(candidate))],
          ),
      ],
      if (error != null) SelectableText('操作失败：$error'),
    ];
    final scheme = Theme.of(context).colorScheme;
    return LayoutBuilder(
      builder: (context, constraints) {
        final wide = constraints.maxWidth >= 1020;
        final showCover = constraints.maxWidth >= 660;
        final inset = constraints.maxWidth < 660 ? 16.0 : 24.0;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(
              child: ListView(
                key: PageStorageKey(
                  '${widget.row.id}:${widget.entry.key}:management',
                ),
                padding: EdgeInsets.fromLTRB(inset, 12, inset, 0),
                children: [
                  Card(
                    child: Padding(
                      padding: EdgeInsets.all(inset),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            '补丁信息',
                            style: Theme.of(context).textTheme.titleMedium
                                ?.copyWith(fontWeight: FontWeight.w600),
                          ),
                          const SizedBox(height: 20),
                          Row(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              if (showCover && widget.artwork != null) ...[
                                ClipRRect(
                                  borderRadius: BorderRadius.circular(12),
                                  child: SizedBox(
                                    width: wide ? 144 : 112,
                                    height: wide ? 216 : 168,
                                    child: GameArtwork(
                                      artwork: widget.artwork!,
                                      kind: ArtworkKind.cover,
                                    ),
                                  ),
                                ),
                                const SizedBox(width: 24),
                              ],
                              Expanded(
                                child: Column(
                                  crossAxisAlignment:
                                      CrossAxisAlignment.stretch,
                                  children: [
                                    for (final detail in details)
                                      Padding(
                                        padding: const EdgeInsets.only(
                                          bottom: 8,
                                        ),
                                        child: detail,
                                      ),
                                  ],
                                ),
                              ),
                              if (wide) ...[
                                const SizedBox(width: 24),
                                SizedBox(
                                  width: 200,
                                  child: DecoratedBox(
                                    decoration: BoxDecoration(
                                      color: scheme.surfaceContainer,
                                      borderRadius: BorderRadius.circular(12),
                                    ),
                                    child: Padding(
                                      padding: const EdgeInsets.all(20),
                                      child: Column(
                                        crossAxisAlignment:
                                            CrossAxisAlignment.start,
                                        children: [
                                          Text(
                                            '游戏信息',
                                            style: Theme.of(context)
                                                .textTheme
                                                .titleMedium,
                                          ),
                                          const SizedBox(height: 20),
                                          Text(
                                            'AppID',
                                            style: TextStyle(
                                              color: scheme.onSurfaceVariant,
                                              fontSize: 12,
                                            ),
                                          ),
                                          SelectableText(
                                            '${widget.row.game['appid']}',
                                          ),
                                          const Divider(height: 28),
                                          Text(
                                            '游戏 Build',
                                            style: TextStyle(
                                              color: scheme.onSurfaceVariant,
                                              fontSize: 12,
                                            ),
                                          ),
                                          SelectableText(
                                            '${widget.row.game['buildid'] ?? '未知'}',
                                          ),
                                        ],
                                      ),
                                    ),
                                  ),
                                ),
                              ],
                            ],
                          ),
                        ],
                      ),
                    ),
                  ),
                ],
              ),
            ),
            if (patch != null)
              Container(
                padding: EdgeInsets.symmetric(horizontal: inset, vertical: 14),
                decoration: BoxDecoration(
                  color: scheme.surface,
                  border: Border(top: BorderSide(color: scheme.outlineVariant)),
                ),
                child: LayoutBuilder(
                  builder: (context, footer) {
                    final notice = Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon(
                          Icons.info_outline,
                          color: scheme.secondary,
                          size: 20,
                        ),
                        const SizedBox(width: 10),
                        const Flexible(
                          child: Text(
                            '安装时请关闭游戏并暂停 Steam 更新',
                            style: TextStyle(fontSize: 13),
                          ),
                        ),
                      ],
                    );
                    final buttons = Wrap(
                      spacing: 12,
                      runSpacing: 8,
                      crossAxisAlignment: WrapCrossAlignment.center,
                      children: [
                        OutlinedButton.icon(
                          onPressed: openDirectory,
                          icon: const Icon(
                            Icons.folder_open_outlined,
                            size: 18,
                          ),
                          label: const Text('打开游戏目录'),
                        ),
                        FilledButton.icon(
                          focusNode: installFocus,
                          onPressed: !busy && available(patch) ? install : null,
                          icon: const Icon(Icons.download_outlined, size: 20),
                          label: Text(
                            input == 'local_archive' && archive == null
                                ? '需要选择本地 ZIP'
                                : localRoute
                                ? '使用本地 ZIP 安装'
                                : patch['actions']['primary']['label'] ==
                                      'overwrite'
                                ? '覆盖安装补丁'
                                : '安装补丁',
                          ),
                        ),
                      ],
                    );
                    final textScale =
                        MediaQuery.textScalerOf(context).scale(14) / 14;
                    if (footer.maxWidth < 920 * textScale) {
                      return Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [notice, const SizedBox(height: 12), buttons],
                      );
                    }
                    return Row(
                      children: [
                        Expanded(child: notice),
                        const SizedBox(width: 16),
                        buttons,
                      ],
                    );
                  },
                ),
              ),
          ],
        );
      },
    );
  }
}
