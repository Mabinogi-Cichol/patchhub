import 'package:flutter/material.dart';

import '../host/core_client.dart';
import '../models/cache_model.dart';

String cacheSize(num bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return '${value.toStringAsFixed(unit == 0 ? 0 : 2)} ${units[unit]}';
}

String cacheError(Object error) =>
    error is CoreError && error.code == 'installer_busy'
    ? '有安装任务正在使用缓存，请任务结束后重试。'
    : '$error';

class CachePage extends StatefulWidget {
  const CachePage({super.key, required this.model});
  final CacheModel model;
  @override
  State<CachePage> createState() => _CachePageState();
}

class _CachePageState extends State<CachePage> {
  @override
  void initState() {
    super.initState();
    if (widget.model.inventory == null && !widget.model.cleaning) {
      widget.model.refresh();
    }
  }

  Future<void> confirmCleanup() async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('清理下载与临时缓存？'),
        content: const SingleChildScrollView(
          child: Text(
            '将删除下载包、处理缓存和临时报告。再次安装可能需要重新下载补丁。\n\n游戏、已安装补丁、来源与订阅配置、安装恢复材料会保留。',
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('清理缓存'),
          ),
        ],
      ),
    );
    if (confirmed == true) await widget.model.cleanup();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.model,
    builder: (context, _) {
      final m = widget.model;
      final inventory = m.inventory;
      final diagnostics = inventory?['diagnostics'] as List? ?? [];
      final result = m.result;
      final failures = result?['failures'] as List? ?? [];
      final resultText = result == null
          ? null
          : switch (result['status']) {
              'completed' =>
                '清理完成，已释放 ${cacheSize(result['removed_bytes'] as num)}',
              'partial' =>
                '部分清理完成，已释放 ${cacheSize(result['removed_bytes'] as num)}；部分项目未清理，请查看详情后重试。',
              'busy' => '有安装任务正在使用缓存，请任务结束后重试。',
              _ =>
                '缓存清理失败，已释放 ${cacheSize(result['removed_bytes'] as num)}，请查看详情。',
            };
      return ListView(
        key: const PageStorageKey('settings-cache'),
        padding: const EdgeInsets.all(28),
        children: [
          Text('缓存管理', style: Theme.of(context).textTheme.headlineSmall),
          const SizedBox(height: 8),
          const Text('释放本机空间，保留游戏、订阅和来源数据。'),
          const SizedBox(height: 32),
          Text('下载与临时缓存', style: Theme.of(context).textTheme.titleMedium),
          const SizedBox(height: 16),
          Card(
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text('缓存占用'),
                  const SizedBox(height: 12),
                  if (m.loading)
                    const Text('正在统计…')
                  else if (m.inventoryError != null) ...[
                    const Text('占用读取失败'),
                    SelectableText(cacheError(m.inventoryError!)),
                  ] else if (inventory != null) ...[
                    Text(
                      cacheSize(inventory['total_bytes'] as num),
                      style: Theme.of(context).textTheme.displaySmall,
                    ),
                    Text(
                      diagnostics.isNotEmpty
                          ? '部分文件无法统计 · 已统计 ${inventory['entries']} 项'
                          : inventory['entries'] == 0
                          ? '暂无需要清理的缓存'
                          : '共 ${inventory['entries']} 项',
                    ),
                    if (diagnostics.isNotEmpty) _details('统计详情', diagnostics),
                  ],
                  const SizedBox(height: 12),
                  OutlinedButton.icon(
                    onPressed: m.loading || m.cleaning ? null : m.refresh,
                    icon: const Icon(Icons.refresh, size: 18),
                    label: const Text('刷新占用'),
                  ),
                  const Divider(height: 32),
                  const Text('清理下载包、处理缓存和临时报告。再次安装时可能需要重新下载补丁。'),
                  const SizedBox(height: 20),
                  Wrap(
                    spacing: 16,
                    runSpacing: 12,
                    crossAxisAlignment: WrapCrossAlignment.center,
                    children: [
                      FilledButton(
                        onPressed: m.cleaning || m.loading
                            ? null
                            : confirmCleanup,
                        child: Text(m.cleaning ? '正在清理…' : '清理缓存'),
                      ),
                      const Text('实际释放大小以清理结果为准'),
                    ],
                  ),
                  if (resultText != null || m.cleanupError != null) ...[
                    const SizedBox(height: 20),
                    Semantics(
                      liveRegion: true,
                      child: SelectableText(
                        resultText ?? '清理失败：${cacheError(m.cleanupError!)}',
                      ),
                    ),
                    if (failures.isNotEmpty) _details('未清理项详情', failures),
                    if (m.cleanupError != null ||
                        result?['status'] != 'completed')
                      TextButton(
                        onPressed: m.loading || m.cleaning
                            ? null
                            : confirmCleanup,
                        child: const Text('重试清理'),
                      ),
                  ],
                ],
              ),
            ),
          ),
          const SizedBox(height: 28),
          Text('清理时会保留', style: Theme.of(context).textTheme.titleMedium),
          const SizedBox(height: 12),
          const Text(
            '已安装的补丁与游戏文件\n来源索引、全局来源与游戏订阅配置\n未完成安装所需的恢复材料',
            style: TextStyle(height: 1.8),
          ),
          const SizedBox(height: 20),
          const Text('安装任务正在使用缓存时，完成任务后再清理。'),
        ],
      );
    },
  );

  Widget _details(String title, List failures) => ExpansionTile(
    key: PageStorageKey(title),
    tilePadding: EdgeInsets.zero,
    title: Text(title),
    children: [
      for (final (index, failure) in failures.indexed)
        Align(
          alignment: Alignment.centerLeft,
          child: SelectableText(
            '${failure['code']}：${failure['detail'] ?? ''}',
            key: PageStorageKey('$title:$index'),
          ),
        ),
    ],
  );
}
