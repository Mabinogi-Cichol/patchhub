import 'dart:convert';

import 'package:flutter/material.dart';

import '../presentation/desktop_select.dart';

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
    this.contentBuilder,
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
  final Widget Function(Widget management)? contentBuilder;
  @override
  State<PatchManagementPage> createState() => _PatchManagementPageState();
}

class _PatchManagementPageState extends State<PatchManagementPage> {
  bool localRoute = false;
  bool reading = false;
  String? archive;
  final manualUrl = TextEditingController();
  Object? error;
  Map<String, dynamic>? preview;
  final installFocus = FocusNode();
  @override
  void dispose() {
    manualUrl.dispose();
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
    final input = localRoute ? 'local_archive' : 'automatic';
    final allowed = input == 'automatic'
        ? patch['actions']['automatic']['available'] == true
        : input == 'local_archive' &&
              patch['actions']['local_archive']['available'] == true &&
              archive != null;
    return allowed;
  }

  bool get manualReady {
    final uri = Uri.tryParse(manualUrl.text.trim());
    return uri != null &&
        ['https', 'http'].contains(uri.scheme) &&
        uri.host.isNotEmpty;
  }

  Future<void> install() async {
    try {
      final row = widget.model.snapshot?.rows
          .where((row) => row.id == widget.row.id)
          .firstOrNull;
      final Map<String, Object?> arguments;
      if (widget.patch == null) {
        if (!manualReady) return;
        final candidate = widget.entry.candidate!;
        arguments = {
          'instance_id': widget.row.id,
          'manual_download': {
            'source_id': candidate['source_id'],
            'candidate_id': candidate['candidate_id'],
            'url': manualUrl.text.trim(),
          },
          'operation': 'overwrite',
          'cleanup_policy': 'disabled',
        };
      } else {
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
        arguments = {
          'instance_id': widget.row.id,
          ...selectionArguments(latest),
          'operation': latest['actions']['primary']['operation'],
          'cleanup_policy': 'disabled',
          if (localRoute) 'archive': archive,
        };
      }
      final task = widget.model.tasks.startInstall(arguments);
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
    final input = patch == null
        ? 'unavailable'
        : localRoute
        ? 'local_archive'
        : 'automatic';
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
    final technicalDetails = patch == null
        ? null
        : ExpansionTile(
            key: PageStorageKey(
              '${widget.row.id}:${widget.entry.selectionKey}:technical',
            ),
            title: const Text('文件与技术详情'),
            expandedCrossAxisAlignment: CrossAxisAlignment.stretch,
            childrenPadding: const EdgeInsets.all(16),
            leading: const Icon(Icons.description_outlined),
            subtitle: const Text('补丁文件、版本依据与变更预览'),
            children: [
              SelectableText(
                '发行标识：${patch['release_id']}\n选择标识：${patch['selection_id'] ?? '无'}\n下载地址：${patch['download_url'] ?? '无'}',
                key: const PageStorageKey('release-details'),
              ),
              if (candidate != null)
                SelectableText(
                  '身份依据：${candidate['match_basis']}\n版本依据：${(candidate['version_evidence'] as List).join('、')}',
                  key: const PageStorageKey('candidate-details'),
                ),
              OutlinedButton(
                onPressed: reading || busy ? null : readPreview,
                child: Text(reading ? '正在读取变更' : '查看文件变更（可选，只读）'),
              ),
              if (preview != null) ...[
                for (final (index, blocker)
                    in (preview!['blockers'] as List).indexed)
                  SelectableText(
                    statusLabel(blocker),
                    key: PageStorageKey('blocker:$index'),
                  ),
                for (final (index, change)
                    in (preview!['changes'] as List).indexed)
                  ListTile(
                    title: SelectableText(
                      change['path'] as String,
                      key: PageStorageKey('change:$index'),
                    ),
                    subtitle: Text('${change['action']}'),
                  ),
                for (final (index, diagnostic)
                    in (preview!['diagnostics'] as List).indexed)
                  SelectableText(
                    '${diagnostic['message']}',
                    key: PageStorageKey('diagnostic:$index'),
                  ),
              ],
            ],
          );
    final fieldTitleStyle = Theme.of(context).textTheme.labelLarge?.copyWith(
      color: Theme.of(context).colorScheme.onSurfaceVariant,
      fontWeight: FontWeight.w600,
    );
    final details = <Widget>[
      if (widget.entries.length > 1) ...[
        Text('补丁选择', style: fieldTitleStyle),
        DesktopSelect(
          label: '补丁选择',
          value: widget.entry.selectionKey,
          options: [
            for (final entry in widget.entries)
              (value: entry.selectionKey, label: entry.title),
          ],
          onSelected: widget.onEntry,
        ),
        const SizedBox(height: 4),
      ],
      if (widget.entries.length <= 1)
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
        if (widget.entry.choices.isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text('下载项 / 镜像', style: fieldTitleStyle),
          ),
        if (widget.entry.choices.length == 1)
          InputDecorator(
            decoration: const InputDecoration(),
            child: SelectableText(
              '${patch['download_label'] ?? '来源下载'} · ${patch['download_url'] != null ? Uri.parse(patch['download_url'] as String).pathSegments.lastOrNull ?? '下载文件' : patch['name']}',
            ),
          ),
        if (widget.entry.choices.length > 1)
          DesktopSelect(
            label: '下载项 / 镜像',
            value: (patch['selection_id'] ?? patch['release_id']) as String,
            options: [
              for (final choice in widget.entry.choices)
                (
                  value:
                      (choice['selection_id'] ?? choice['release_id'])
                          as String,
                  label:
                      '${choice['download_label'] ?? '来源下载'} · ${choice['download_url'] != null ? Uri.parse(choice['download_url'] as String).pathSegments.lastOrNull ?? '下载文件' : choice['name']}',
                ),
            ],
            onSelected: widget.onMirror,
          ),
        for (final blocker in blockers)
          SelectableText(
            '${statusLabel(blocker['code'])}${blocker['detail'] == null ? '' : '\n${blocker['detail']}'}',
          ),
        if (patch['actions']['local_archive']['available'] == true)
          ExpansionTile(
            key: PageStorageKey(
              '${widget.row.id}:${widget.entry.selectionKey}:local-zip',
            ),
            title: const Text('使用本地 ZIP（可选）'),
            children: [
              OutlinedButton(
                onPressed: busy ? null : chooseArchive,
                child: const Text('选择本地 ZIP'),
              ),
              if (archive != null)
                SelectableText(
                  '已选择：$archive',
                  key: const PageStorageKey('local-archive'),
                ),
            ],
          ),
        if (localRoute)
          TextButton(
            onPressed: () => setState(() {
              localRoute = false;
              archive = null;
            }),
            child: const Text('取消本地文件选择'),
          ),
      ] else ...[
        const SizedBox(height: 16),
        const Text('未找到下载链接'),
        const Text('可打开来源网页，复制 ZIP 或 EXE 下载直链后在下方安装。'),
        TextField(
          controller: manualUrl,
          enabled: !busy,
          onChanged: (_) => setState(() => error = null),
          decoration: InputDecoration(
            labelText: '手动输入下载链接',
            hintText: 'ZIP 或 EXE 的 HTTP(S) 下载直链',
            errorText: manualUrl.text.trim().isNotEmpty && !manualReady
                ? '请输入完整的 HTTP(S) 下载链接'
                : null,
          ),
        ),
        if (candidate != null)
          ExpansionTile(
            key: PageStorageKey(
              '${widget.row.id}:${widget.entry.selectionKey}:source-details',
            ),
            title: const Text('来源技术详情'),
            children: [
              SelectableText(
                jsonEncode(candidate),
                key: const PageStorageKey('source-json'),
              ),
            ],
          ),
      ],
      if (error != null) SelectableText('操作失败：$error'),
    ];
    final scheme = Theme.of(context).colorScheme;
    final ready = patch == null ? manualReady : available(patch);
    final installed =
        patch != null && installedStatus(patch['detection']['status']);
    final statusColor = !busy && ready
        ? (scheme.brightness == Brightness.dark
              ? const Color(0xff75ddb0)
              : const Color(0xff176b48))
        : scheme.onSurfaceVariant;
    final statusTitle = busy
        ? '操作进行中'
        : patch == null
        ? (manualReady ? '已补充下载链接' : '未找到下载链接')
        : ready
        ? (installed ? '已安装补丁' : '可安装补丁')
        : '暂不可安装';
    final statusDescription = busy
        ? '请等待当前任务完成'
        : patch == null
        ? (manualReady ? '安装时读取下载文件' : '可从来源网页补充下载直链')
        : ready
        ? (installed ? '可按当前安装路线覆盖安装' : '可按当前安装路线安装此补丁')
        : '当前没有可用的自动下载项，可查看其他输入方式';
    final summary = Container(
      key: const ValueKey('patch-status-panel'),
      padding: const EdgeInsets.all(20),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: scheme.outlineVariant),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text('补丁状态', style: Theme.of(context).textTheme.titleMedium),
          const SizedBox(height: 12),
          Container(
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              color: statusColor.withValues(alpha: .08),
              border: Border.all(color: statusColor.withValues(alpha: .35)),
              borderRadius: BorderRadius.circular(10),
            ),
            child: Row(
              children: [
                Icon(
                  busy
                      ? Icons.hourglass_top
                      : installed && ready
                      ? Icons.check_circle_outline
                      : Icons.download_outlined,
                  color: statusColor,
                  size: 26,
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        statusTitle,
                        style: TextStyle(
                          color: statusColor,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      const SizedBox(height: 4),
                      Text(
                        statusDescription,
                        style: TextStyle(
                          color: scheme.onSurfaceVariant,
                          fontSize: 12,
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 24),
          Text('游戏信息', style: Theme.of(context).textTheme.titleMedium),
          const SizedBox(height: 12),
          Text(
            'AppID',
            style: TextStyle(color: scheme.onSurfaceVariant, fontSize: 12),
          ),
          SelectableText('${widget.row.game['appid']}'),
          const Divider(height: 24),
          Text(
            '游戏 Build',
            style: TextStyle(color: scheme.onSurfaceVariant, fontSize: 12),
          ),
          SelectableText('${widget.row.game['buildid'] ?? '未知'}'),
          const SizedBox(height: 12),
          ExpansionTile(
            key: PageStorageKey('${widget.row.id}:game-paths'),
            tilePadding: EdgeInsets.zero,
            title: const Text('更多游戏信息'),
            children: [
              SelectableText(
                '安装路径：${widget.row.game['install_path']}\n库位置：${widget.row.game['library_path']}',
                key: const PageStorageKey('game-paths-text'),
              ),
            ],
          ),
        ],
      ),
    );
    return LayoutBuilder(
      builder: (context, constraints) {
        final textScale = MediaQuery.textScalerOf(context).scale(14) / 14;
        final wide = constraints.maxWidth >= 880 * textScale;
        final showCover = constraints.maxWidth >= 660;
        final inset = constraints.maxWidth < 660 ? 16.0 : 24.0;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(
              child: buildContent(
                ListView(
                  key: PageStorageKey(
                    '${widget.row.id}:${widget.entry.selectionKey}:management',
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
                                  SizedBox(width: 240, child: summary),
                                ],
                              ],
                            ),
                            if (!wide) ...[const SizedBox(height: 20), summary],
                            if (technicalDetails != null) ...[
                              const SizedBox(height: 16),
                              Material(
                                color: scheme.surfaceContainer,
                                clipBehavior: Clip.antiAlias,
                                shape: RoundedRectangleBorder(
                                  borderRadius: BorderRadius.circular(12),
                                  side: BorderSide(
                                    color: scheme.outlineVariant,
                                  ),
                                ),
                                child: technicalDetails,
                              ),
                            ],
                          ],
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
            Padding(
              padding: EdgeInsets.fromLTRB(inset, 8, inset, 20),
              child: Container(
                key: const ValueKey('patch-floating-actions'),
                padding: EdgeInsets.symmetric(horizontal: inset, vertical: 20),
                decoration: BoxDecoration(
                  color: scheme.surface,
                  borderRadius: BorderRadius.circular(16),
                  border: Border.all(color: scheme.outlineVariant),
                  boxShadow: [
                    BoxShadow(
                      color: Colors.black.withValues(alpha: .16),
                      blurRadius: 24,
                      offset: const Offset(0, 6),
                    ),
                  ],
                ),
                child: LayoutBuilder(
                  builder: (context, footer) {
                    final notice = Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon(
                          Icons.info_outline,
                          color: scheme.brightness == Brightness.dark
                              ? const Color(0xffffc75b)
                              : const Color(0xff875d00),
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
                      alignment: WrapAlignment.end,
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
                          onPressed: !busy && ready ? install : null,
                          icon: const Icon(Icons.download_outlined, size: 20),
                          label: Text(
                            patch == null
                                ? '下载并安装'
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
                    if (footer.maxWidth < 720 * textScale) {
                      return Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
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
            ),
          ],
        );
      },
    );
  }

  Widget buildContent(Widget management) =>
      widget.contentBuilder?.call(management) ?? management;
}
