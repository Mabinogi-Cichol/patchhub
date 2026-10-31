import 'package:flutter/material.dart';

import '../host/desktop_services.dart';
import '../host/core_task.dart';
import '../models/app_model.dart';
import '../models/library.dart';
import '../presentation/library_presentation.dart';
import 'script_source_editor.dart';

String sourceTime(dynamic seconds) => seconds == null
    ? '尚无记录'
    : DateTime.fromMillisecondsSinceEpoch((seconds as int) * 1000)
          .toLocal()
          .toString()
          .split('.')
          .first;

class TaskSummary extends StatelessWidget {
  const TaskSummary({super.key, required this.task});
  final CoreTask task;
  @override
  Widget build(BuildContext context) {
    final query = ['match_games', 'rematch_game'].contains(task.method);
    if (query && !task.running) return const SizedBox.shrink();
    final progress = task.progress;
    final total = progress?['total'] as int?;
    final completed = progress?['completed'] as int?;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (task.running) ...[
          LinearProgressIndicator(
            value: total != null && total > 0
                ? (completed! / total).clamp(0, 1)
                : null,
          ),
          Text(
            task.cancelRequested
                ? '正在取消，请等待实际结果'
                : '${progress?['phase'] ?? '正在启动'}${total != null && total > 0 ? ' · $completed / $total' : ''}',
          ),
          TextButton(
            onPressed: task.cancelRequested ? null : task.cancel,
            child: const Text('取消任务'),
          ),
        ] else if (task.error != null)
          SelectableText(
            '任务结果：${statusLabel(task.error!.code)}${task.error!.detail == null ? '' : '\n${task.error!.detail}'}',
          )
        else
          const Text('任务已完成'),
        if (!query && task.controlError != null)
          SelectableText('取消通道错误：${task.controlError}'),
        if (!query && task.diagnostics.isNotEmpty)
          ExpansionTile(
            title: const Text('任务诊断'),
            children: [SelectableText(task.diagnostics)],
          ),
      ],
    );
  }
}

class GlobalSourcesPage extends StatefulWidget {
  const GlobalSourcesPage({super.key, required this.model});
  final AppModel model;
  @override
  State<GlobalSourcesPage> createState() => _GlobalSourcesPageState();
}

class _GlobalSourcesPageState extends State<GlobalSourcesPage> {
  final expanded = <String>{};
  final intervals = <String, String>{};
  final names = <String, String>{};
  final errors = <String, Object>{};
  bool saving = false;

  Future<void> save(
    String key,
    String method,
    Map<String, Object?> args,
  ) async {
    setState(() {
      saving = true;
      errors.remove(key);
    });
    try {
      await widget.model.mutate(method, args);
    } catch (failure) {
      if (mounted) setState(() => errors[key] = failure);
    } finally {
      if (mounted) setState(() => saving = false);
    }
  }

  Future<void> removeSource(String sourceId, String name) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: Text('删除来源：$name'),
        content: const Text('删除后该来源不再参与更新和匹配。缓存文件保留在本机，但不会再生效。'),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(dialogContext).pop(false),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(dialogContext).pop(true),
            child: const Text('删除'),
          ),
        ],
      ),
    );
    if (confirmed != true) return;
    await save(sourceId, 'remove_source', {'source_id': sourceId});
  }

  Future<void> editScript([String? sourceId, bool readOnly = false]) async {
    Map<String, dynamic>? document;
    if (sourceId != null) {
      setState(() {
        saving = true;
        errors.remove(sourceId);
      });
      try {
        document = await widget.model.client.call('get_script_source', {
          'source_id': sourceId,
        }) as Map<String, dynamic>;
      } catch (failure) {
        if (mounted) setState(() => errors[sourceId] = failure);
        return;
      } finally {
        if (mounted) setState(() => saving = false);
      }
    }
    if (!mounted) return;
    final saved = await showDialog<bool>(
      context: context,
      barrierDismissible: false,
      builder: (_) => ScriptSourceEditor(
        document: document,
        readOnly: readOnly,
        onSave: (params) => widget.model.mutate('import_script_source', params),
      ),
    );
    if (saved == true && mounted) {
      setState(() {});
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(const SnackBar(content: Text('脚本已保存，尚未执行。请更新来源或重新匹配游戏。')));
    }
  }

  Future<void> open(String url) async {
    try {
      await DesktopServices().openSource(Uri.parse(url));
    } catch (failure) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('$failure')));
      }
    }
  }

  @override
  Widget build(BuildContext context) => ListView(
    key: const PageStorageKey('global-sources'),
    padding: const EdgeInsets.all(24),
    children: [
      Text('全局来源', style: Theme.of(context).textTheme.headlineSmall),
      const Text('管理所有游戏使用的来源；单个游戏的订阅请进入“游戏订阅”。'),
      if (widget.model.sourceError != null)
        SelectableText('来源读取或状态刷新失败：${widget.model.sourceError}'),
      if (widget.model.stateRefreshError != null)
        SelectableText('${widget.model.stateRefreshError}'),
      const SizedBox(height: 12),
      Align(
        alignment: Alignment.centerLeft,
        child: OutlinedButton.icon(
          onPressed: saving || widget.model.tasks.closing
              ? null
              : () => editScript(),
          icon: const Icon(Icons.code),
          label: const Text('导入脚本来源'),
        ),
      ),
      Align(
        alignment: Alignment.centerLeft,
        child: TextButton.icon(
          onPressed: widget.model.tasks.closing
              ? null
              : () => widget.model.startSource('match_games', {}),
          icon: const Icon(Icons.manage_search),
          label: const Text('重新匹配整个游戏库'),
        ),
      ),
      for (final task in widget.model.tasks.sources.values.where(
        (task) => task.method != 'update_source' && task.running,
      ))
        Card(
          child: Padding(
            padding: const EdgeInsets.all(12),
            child: TaskSummary(task: task),
          ),
        ),
      for (final fallback in [false, true]) ...[
        const SizedBox(height: 16),
        Text(
          fallback ? '补查来源' : '发行商来源',
          style: Theme.of(context).textTheme.titleMedium,
        ),
        if (fallback) const Text('补查来源按游戏查询，不参与定时索引更新。'),
        const SizedBox(height: 12),
        for (final source in widget.model.sources.where(
          (source) =>
              (source['role'] == 'fallback_indexer' ||
                  source['id'] == 'hgamer') ==
              fallback,
        ))
          sourceCard(source, fallback),
      ],
    ],
  );

  Widget sourceCard(Map<String, dynamic> source, bool fallback) {
    final id = source['id'] as String;
    final task = widget.model.tasks.sources['update_source:$id'];
    final running = task?.running == true;
    return Card(
      child: ExpansionTile(
        shape: const Border(),
        collapsedShape: const Border(),
        tilePadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
        key: PageStorageKey('source:$id'),
        initiallyExpanded: expanded.contains(id),
        maintainState: true,
        onExpansionChanged: (value) => setState(() {
          if (value) {
            expanded.add(id);
          } else {
            expanded.remove(id);
          }
        }),
        title: Text(source['name'] as String),
        trailing: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Tooltip(
              message: '启用此来源',
              child: Switch(
                value: source['enabled'] as bool,
                onChanged: saving
                    ? null
                    : (value) => save(id, 'set_source_enabled', {
                        'source_id': id,
                        'enabled': value,
                      }),
              ),
            ),
            const SizedBox(width: 12),
            Icon(expanded.contains(id) ? Icons.expand_less : Icons.expand_more),
          ],
        ),
        subtitle: Text(
          '${source['enabled'] == true ? '已启用' : '已停用'} · '
          '${running
              ? (task!.cancelRequested ? '正在取消' : '正在更新')
              : task?.error != null
              ? '${task!.error}'
              : statusLabel(source['status'])}\n'
          '最近检查：${sourceTime(source['checked_at'])}',
        ),
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const Divider(height: 24),
                if (source['script'] is Map) ...[
                  if (source['origin'] == 'builtin') ...[
                    const Text('内置来源随版本更新，脚本只读。'),
                    Align(
                      alignment: Alignment.centerLeft,
                      child: TextButton(
                        key: PageStorageKey('source:$id:view-script'),
                        onPressed: saving ? null : () => editScript(id, true),
                        child: const Text('查看脚本'),
                      ),
                    ),
                  ] else
                    Wrap(
                      spacing: 8,
                      children: [
                        TextButton(
                          onPressed: saving ? null : () => editScript(id),
                          child: const Text('编辑脚本'),
                        ),
                        TextButton(
                          key: PageStorageKey('source:$id:remove'),
                          onPressed: saving || running
                              ? null
                              : () => removeSource(id, source['name'] as String),
                          child: const Text('删除来源'),
                        ),
                      ],
                    ),
                ],
                if (errors[id] != null)
                  SelectableText(
                    '操作失败：${errors[id]}',
                    key: PageStorageKey('source:$id:save-error'),
                  ),
                if (task?.running == true) TaskSummary(task: task!),
                if (!fallback) ...[
                  const Text('自动更新'),
                  const SizedBox(height: 8),
                  Wrap(
                    spacing: 12,
                    runSpacing: 8,
                    crossAxisAlignment: WrapCrossAlignment.center,
                    children: [
                      SizedBox(
                        width: 200,
                        child: Semantics(
                          label: '自动更新间隔（小时）',
                          child: TextFormField(
                            key: PageStorageKey(
                              '$id:interval:${source['interval_hours']}',
                            ),
                            initialValue: '${source['interval_hours']}',
                            keyboardType: TextInputType.number,
                            decoration: const InputDecoration(
                              suffixText: '小时',
                              hintText: '更新间隔',
                            ),
                            enabled: !saving,
                            onChanged: (value) => intervals[id] = value,
                          ),
                        ),
                      ),
                      TextButton(
                        onPressed: saving
                            ? null
                            : () {
                                final value = int.tryParse(
                                  intervals[id] ??
                                      '${source['interval_hours']}',
                                );
                                if (value == null || value < 0) {
                                  setState(() => errors[id] = '请输入0或正整数');
                                  return;
                                }
                                save(id, 'set_source_update_interval', {
                                  'source_id': id,
                                  'interval_hours': value,
                                });
                              },
                        child: const Text('保存间隔'),
                      ),
                      Text(
                        '0 表示关闭自动更新',
                        style: Theme.of(context).textTheme.bodySmall,
                      ),
                    ],
                  ),
                  const SizedBox(height: 16),
                ],
                TextFormField(
                  key: PageStorageKey('$id:name:${source['name']}'),
                  initialValue: source['name'] as String,
                  enabled: !saving,
                  decoration: InputDecoration(
                    labelText: '来源名称',
                    errorText: names[id]?.trim().isEmpty == true
                        ? '请输入来源名称'
                        : null,
                  ),
                  onChanged: (value) => setState(() => names[id] = value),
                ),
                Align(
                  alignment: Alignment.centerLeft,
                  child: TextButton(
                    onPressed:
                        saving ||
                            (names[id] ?? '').trim().isEmpty ||
                            names[id]?.trim() == source['name']
                        ? null
                        : () => save(id, 'set_source_name', {
                            'source_id': id,
                            'name': names[id],
                          }),
                    child: const Text('保存名称'),
                  ),
                ),
                const SizedBox(height: 16),
                Wrap(
                  spacing: 16,
                  runSpacing: 8,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                    Text(
                      '索引：${source['entry_count']} 条 · ${source['complete'] == true ? '覆盖完整' : '覆盖不完整'}',
                    ),
                    if (!fallback)
                      OutlinedButton.icon(
                        onPressed: running || saving
                            ? null
                            : () => widget.model.startSource('update_source', {
                                'source_id': id,
                                'force': false,
                              }),
                        icon: const Icon(Icons.refresh, size: 18),
                        label: const Text('更新来源索引'),
                      ),
                  ],
                ),
                const SizedBox(height: 8),
                ExpansionTile(
                  tilePadding: EdgeInsets.zero,
                  childrenPadding: const EdgeInsets.only(bottom: 8),
                  expandedAlignment: Alignment.centerLeft,
                  expandedCrossAxisAlignment: CrossAxisAlignment.start,
                  shape: const Border(),
                  collapsedShape: const Border(),
                  key: PageStorageKey('source:$id:details'),
                  title: const Text('调度与地址详情'),
                  children: [
                    SelectableText(
                      '最近成功：${sourceTime(source['cached_at'])}\n下次到期：${sourceTime(source['next_due_at'])}\n来源版本：${source['revision']}\n设置版本：${widget.model.settingsRevision ?? '未知'}',
                      key: PageStorageKey('source:$id:details-text'),
                    ),
                    for (final page in source['pages'] as List)
                      TextButton(
                        onPressed: () => open(page as String),
                        child: Text(page),
                      ),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class GameSourcesPage extends StatefulWidget {
  const GameSourcesPage({super.key, required this.model, required this.row});
  final AppModel model;
  final GameRow row;
  @override
  State<GameSourcesPage> createState() => _GameSourcesPageState();
}

class _GameSourcesPageState extends State<GameSourcesPage> {
  List<Map<String, dynamic>>? sources;
  Object? error;
  bool saving = false;
  @override
  void initState() {
    super.initState();
    load();
  }

  Future<void> load() async {
    try {
      final value = await widget.model.gameSources(widget.row.id);
      if (mounted) {
        setState(() {
          sources = value;
          error = null;
        });
      }
    } catch (failure) {
      if (mounted) setState(() => error = failure);
    }
  }

  Future<void> toggle(String id, bool subscribed) async {
    setState(() {
      saving = true;
      error = null;
    });
    try {
      await widget.model.mutate('set_game_source', {
        'instance_id': widget.row.id,
        'source_id': id,
        'subscribed': subscribed,
      });
      await load();
    } catch (failure) {
      if (mounted) setState(() => error = failure);
    } finally {
      if (mounted) setState(() => saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final task =
        widget.model.tasks.sources['rematch_game:instance:${widget.row.id}'];
    return ListView(
      key: PageStorageKey('${widget.row.id}:subscriptions'),
      padding: const EdgeInsets.all(20),
      children: [
        const Text('仅控制当前游戏使用的来源；全局停用的来源不会参与匹配。'),
        if (error != null) SelectableText('订阅读取或保存失败：$error'),
        if (sources == null && error == null) const LinearProgressIndicator(),
        if (error != null)
          TextButton(onPressed: load, child: const Text('重新读取订阅')),
        for (final source in sources ?? <Map<String, dynamic>>[])
          SwitchListTile(
            title: Text(source['name'] as String),
            subtitle: Text(source['enabled'] == true ? '全局已启用' : '全局已停用'),
            value: source['subscribed'] as bool,
            onChanged: saving
                ? null
                : (value) => toggle(source['id'] as String, value),
          ),
        const SizedBox(height: 12),
        const SizedBox(height: 12),
        OutlinedButton(
          onPressed: saving || task?.running == true
              ? null
              : () => widget.model.rematch(widget.row),
          child: const Text('重新匹配当前游戏'),
        ),
        if (task?.running == true) TaskSummary(task: task!),
      ],
    );
  }
}
