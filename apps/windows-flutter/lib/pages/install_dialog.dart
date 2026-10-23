import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../host/core_task.dart';
import '../models/app_model.dart';
import '../presentation/library_presentation.dart';

String byteCount(int bytes) => bytes >= 1000000000
    ? '${(bytes / 1000000000).toStringAsFixed(2)} GB'
    : bytes >= 1000000
    ? '${(bytes / 1000000).toStringAsFixed(1)} MB'
    : bytes >= 1000
    ? '${(bytes / 1000).toStringAsFixed(1)} KB'
    : '$bytes B';

String installStage(dynamic stage, dynamic substage) =>
    const {
      'connecting': '正在连接',
      'downloading': '正在下载',
      'validating': '正在校验',
      'analyzing_installer': '正在分析安装器',
      'extracting': '正在提取',
      'planning': '正在生成计划',
      'applying': '正在写入',
      'rolling_back': '正在恢复已修改文件',
      'cleanup': '正在清理',
      'backing_up': '正在备份',
      'writing': '正在写入',
      'committing': '正在提交',
    }[substage ?? stage] ??
    '正在启动安装';

Future<String?> showInstallDialog(
  BuildContext context, {
  required AppModel model,
  required CoreTask task,
  required String game,
  required String patch,
}) => showDialog<String>(
  context: context,
  barrierDismissible: false,
  builder: (_) =>
      InstallDialog(model: model, task: task, game: game, patch: patch),
);

class InstallDialog extends StatefulWidget {
  const InstallDialog({
    super.key,
    required this.model,
    required this.task,
    required this.game,
    required this.patch,
  });
  final AppModel model;
  final CoreTask task;
  final String game;
  final String patch;
  @override
  State<InstallDialog> createState() => _InstallDialogState();
}

class _InstallDialogState extends State<InstallDialog> {
  StreamSubscription<void>? subscription;
  Object? refreshError;
  bool refreshing = false;
  bool refreshSkipped = false;
  @override
  void initState() {
    super.initState();
    subscription = widget.task.changes.listen((_) {
      if (mounted) setState(() {});
    });
    afterTask();
  }

  Future<void> afterTask() async {
    await widget.task.done;
    if (!mounted) return;
    if (widget.model.tasks.closing) {
      setState(() => refreshSkipped = true);
      return;
    }
    setState(() => refreshing = true);
    try {
      if (!await widget.model.refresh()) {
        refreshError = widget.model.error ?? '状态读取未完成';
      }
    } catch (failure) {
      refreshError = failure;
    }
    if (mounted) setState(() => refreshing = false);
  }

  @override
  void dispose() {
    subscription?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final task = widget.task;
    final outcome = task.result as Map?;
    final needsRecovery =
        outcome?['state'] == 'needs_recovery' ||
        outcome?['recovery']?['required'] == true;
    final reasonCode =
        outcome?['cause']?['code'] ??
        outcome?['error'] ??
        outcome?['cause']?['kind'];
    final reason = switch (reasonCode) {
      'cancelled_incomplete' => '已取消安装',
      'install_incomplete' => '安装未完成',
      _ => statusLabel(reasonCode),
    };
    final title = task.running
        ? '安装进度'
        : needsRecovery
        ? '仍需恢复'
        : task.error != null
        ? ([
                'cancelled',
                'task_cancelled',
                'download_cancelled',
              ].contains(task.error!.code)
              ? '已取消安装'
              : '安装未完成')
        : outcome?['error'] == 'cancelled_incomplete'
        ? '已取消，安装未完成'
        : switch (outcome?['disposition']) {
            'completed' => '安装完成',
            'cancelled' => '已取消安装',
            _ => '安装失败',
          };
    final progress = task.progress;
    final totalBytes = progress?['total_bytes'] as int?;
    final totalFiles = progress?['total_files'] as int?;
    final doneBytes = progress?['bytes_done'] as int? ?? 0;
    final doneFiles = progress?['files_done'] as int? ?? 0;
    final fraction = totalBytes != null && totalBytes > 0
        ? (doneBytes / totalBytes).clamp(0.0, 1.0)
        : totalFiles != null && totalFiles > 0
        ? (doneFiles / totalFiles).clamp(0.0, 1.0)
        : null;
    void escape() {
      if (task.running) {
        task.cancel();
      } else {
        Navigator.of(context).pop();
      }
    }

    return CallbackShortcuts(
      bindings: {const SingleActivator(LogicalKeyboardKey.escape): escape},
      child: Focus(
        autofocus: true,
        child: PopScope(
          canPop: !task.running,
          onPopInvokedWithResult: (didPop, result) {
            if (!didPop && task.running) task.cancel();
          },
          child: AlertDialog(
            title: Text(title),
            content: SizedBox(
              width: 600,
              child: SingleChildScrollView(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      widget.game,
                      style: Theme.of(context).textTheme.titleMedium,
                    ),
                    Text(widget.patch),
                    const SizedBox(height: 20),
                    if (task.running) ...[
                      Text(
                        task.cancelRequested
                            ? '正在取消并处理已修改文件，请等待实际结果'
                            : installStage(
                                progress?['stage'],
                                progress?['substage'],
                              ),
                      ),
                      const SizedBox(height: 12),
                      LinearProgressIndicator(value: fraction),
                      const SizedBox(height: 12),
                      Text(
                        totalBytes != null
                            ? '${byteCount(doneBytes)} / ${byteCount(totalBytes)}'
                            : '已处理 ${byteCount(doneBytes)}',
                      ),
                      if (progress?['stage'] == 'downloading')
                        Text(
                          task.downloadRate.bytesPerSecond != null
                              ? '下载速度：${byteCount(task.downloadRate.bytesPerSecond!.round())}/s'
                              : '下载速度：计算中…',
                        ),
                      Text(
                        totalFiles != null
                            ? '$doneFiles / $totalFiles 个文件'
                            : '已处理 $doneFiles 个文件',
                      ),
                      if (progress?['current_file'] != null)
                        SelectableText(progress!['current_file'] as String),
                    ] else ...[
                      if (task.error != null)
                        SelectableText('具体原因：${task.error}'),
                      if (outcome != null) ...[
                        Text('文件状态：${statusLabel(outcome['state'])}'),
                        if (reasonCode != null) SelectableText('原因：$reason'),
                        if (outcome['error'] != null &&
                            outcome['error'] != reasonCode)
                          SelectableText(statusLabel(outcome['error'])),
                        if (outcome['cause']?['detail'] != null)
                          SelectableText('${outcome['cause']['detail']}'),
                        if (outcome['direct'] == true &&
                            outcome['state'] == 'failed')
                          const Text(
                            '直接提取未完成；如已写入文件，这些文件不会自动恢复。可重新安装补丁或使用 Steam 验证游戏文件。',
                          ),
                        if (outcome['recovery']?['detail'] != null)
                          SelectableText('${outcome['recovery']['detail']}'),
                        for (final failure
                            in outcome['cleanup']['failures'] as List)
                          SelectableText(
                            '清理未完成：${failure['code']}\n${failure['detail'] ?? ''}',
                          ),
                        if (needsRecovery)
                          const Text('请进入操作记录查看未完成操作，并使用全局恢复入口处理。'),
                        ExpansionTile(
                          title: const Text('操作与清理详情'),
                          children: [SelectableText(jsonEncode(outcome))],
                        ),
                      ],
                      if (refreshing)
                        const Padding(
                          padding: EdgeInsets.only(top: 12),
                          child: Text('正在刷新游戏与记录状态…'),
                        ),
                      if (refreshSkipped)
                        const Text('应用正在关闭；返回应用后会重新读取游戏与记录状态。'),
                      if (refreshError != null)
                        SelectableText(
                          '${outcome?['disposition'] == 'completed' ? '安装已完成' : '任务已结束'}，状态刷新失败：$refreshError',
                        ),
                    ],
                    if (task.controlError != null)
                      SelectableText('取消请求传输失败：${task.controlError}'),
                    if (task.diagnostics.isNotEmpty)
                      ExpansionTile(
                        title: const Text('核心诊断'),
                        children: [SelectableText(task.diagnostics)],
                      ),
                  ],
                ),
              ),
            ),
            actions: task.running
                ? [
                    TextButton(
                      onPressed: task.cancelRequested ? null : task.cancel,
                      child: Text(task.cancelRequested ? '正在取消…' : '取消安装'),
                    ),
                  ]
                : [
                    TextButton(
                      onPressed: () => Navigator.of(context).pop('records'),
                      child: const Text('查看操作记录'),
                    ),
                    FilledButton(
                      onPressed: () => Navigator.of(context).pop(),
                      child: const Text('关闭结果'),
                    ),
                  ],
          ),
        ),
      ),
    );
  }
}
