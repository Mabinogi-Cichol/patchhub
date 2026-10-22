import 'package:flutter/material.dart';

import '../models/app_model.dart';
import '../models/library.dart';
import '../presentation/library_presentation.dart';
import 'source_pages.dart' show sourceTime;

bool needsRecovery(Map<String, dynamic> transaction) => [
  'backing_up',
  'applying',
  'uninstalling',
  'rolling_back',
  'needs_recovery',
].contains(transaction['state']);

class RecordsPage extends StatefulWidget {
  const RecordsPage({super.key, required this.model, required this.row});
  final AppModel model;
  final GameRow row;
  @override
  State<RecordsPage> createState() => _RecordsPageState();
}

class _RecordsPageState extends State<RecordsPage> {
  bool busy = false;
  Object? error;
  List<Map<String, dynamic>>? recoveryResult;
  bool refreshFailed = false;

  Future<void> recover() async {
    setState(() {
      busy = true;
      error = null;
      refreshFailed = false;
    });
    try {
      await widget.model.loadTransactions();
      if (!mounted) return;
      final all = widget.model.transactions;
      final pending = all.where(needsRecovery).toList();
      final confirmed = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('恢复全部未完成操作'),
          content: SizedBox(
            width: 600,
            child: SingleChildScrollView(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text(
                    '这是全局操作，可能处理其它游戏的未完成记录。核心会检查所有记录；本入口不提供卸载或恢复成功安装前版本的功能。',
                  ),
                  for (final record in pending)
                    ListTile(
                      title: Text(recordName(record)),
                      subtitle: SelectableText(
                        '${statusLabel(record['state'])}\n${record['transaction_id']}',
                      ),
                    ),
                  if (pending.isEmpty) const Text('当前未发现可见的未完成操作。'),
                  ExpansionTile(
                    title: const Text('全部全局记录（核心检查范围）'),
                    children: [
                      for (final record in all)
                        ListTile(
                          title: Text(recordName(record)),
                          subtitle: SelectableText(
                            '${statusLabel(record['state'])}\n${record['transaction_id']}',
                          ),
                        ),
                    ],
                  ),
                ],
              ),
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context, false),
              child: const Text('取消'),
            ),
            FilledButton(
              onPressed: () => Navigator.pop(context, true),
              child: const Text('恢复全部未完成操作'),
            ),
          ],
        ),
      );
      if (confirmed != true) return;
      final result = (await widget.model.client.call('recover') as List)
          .cast<Map<String, dynamic>>();
      if (mounted) setState(() => recoveryResult = result);
      final refreshed = await widget.model.refresh();
      if (!refreshed && mounted) setState(() => refreshFailed = true);
    } catch (failure) {
      if (mounted) setState(() => error = failure);
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  String recordName(Map<String, dynamic> record) {
    final row = widget.model.snapshot?.rows
        .where((row) => row.id == record['instance_id'])
        .firstOrNull;
    final patch = row?.patches
        .where(
          (patch) => patch['selection_id'] != null
              ? patch['selection_id'] == record['selection_id']
              : patch['release_id'] == record['release_id'],
        )
        .firstOrNull;
    return '${row?.name ?? record['instance_id']} · ${patch?['name'] ?? record['release_id']}';
  }

  @override
  Widget build(BuildContext context) {
    final records = widget.model.transactions
        .where((record) => record['instance_id'] == widget.row.id)
        .toList();
    final pending = widget.model.transactions.where(needsRecovery).toList();
    return ListView(
      key: PageStorageKey('${widget.row.id}:records'),
      padding: const EdgeInsets.all(20),
      children: [
        if (widget.model.transactionError != null)
          SelectableText('操作记录读取失败：${widget.model.transactionError}'),
        if (pending.isNotEmpty)
          Card(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text('全局有 ${pending.length} 项未完成操作；恢复会检查所有游戏。'),
                  OutlinedButton(
                    onPressed:
                        busy || widget.model.tasks.installation?.running == true
                        ? null
                        : recover,
                    child: Text(busy ? '正在处理全局恢复' : '恢复全部未完成操作'),
                  ),
                ],
              ),
            ),
          ),
        if (busy) const LinearProgressIndicator(),
        if (error != null) SelectableText('恢复失败：$error'),
        if (recoveryResult != null) ...[
          Text(
            recoveryResult!.isEmpty
                ? '核心未发现需要处理的操作'
                : '全局恢复已返回 ${recoveryResult!.length} 项实际结果',
          ),
          for (final result in recoveryResult!)
            ListTile(
              title: Text(recordName(result)),
              subtitle: SelectableText(
                '${statusLabel(result['state'])}${result['error'] == null ? '' : '\n${result['error']}'}',
              ),
            ),
          if (refreshFailed) const Text('恢复结果已返回，状态刷新失败；请重新扫描确认。'),
        ],
        if (records.isEmpty) const Text('当前游戏暂无操作记录'),
        for (final record in records)
          Card(
            child: ExpansionTile(
              title: Text(recordName(record)),
              subtitle: Text(
                '${statusLabel(record['state'])} · ${sourceTime(record['updated_at'])}',
              ),
              children: [
                Padding(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      if (record['error'] != null)
                        SelectableText('具体原因：${record['error']}'),
                      SelectableText(
                        '事务：${record['transaction_id']}\n计划：${record['plan_id']}\n'
                        '发行：${record['release_id']}\n选择：${record['selection_id'] ?? '无'}\n创建：${sourceTime(record['created_at'])}',
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }
}
