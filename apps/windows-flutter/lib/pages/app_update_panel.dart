import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:url_launcher/url_launcher.dart';

import '../host/app_lifecycle.dart';
import '../host/app_update.dart';

class AppUpdatePanel extends StatefulWidget {
  const AppUpdatePanel({
    super.key,
    required this.update,
    required this.lifecycle,
    required this.child,
  });
  final AppUpdate update;
  final AppLifecycle lifecycle;
  final Widget child;
  @override
  State<AppUpdatePanel> createState() => _AppUpdatePanelState();
}

class _AppUpdatePanelState extends State<AppUpdatePanel> {
  String? dismissed;
  bool applying = false;
  Object? error;
  @override
  void initState() {
    super.initState();
    unawaited(widget.update.initialize());
  }

  Future<void> apply() async {
    final update = widget.update;
    if (widget.lifecycle.tasks?.active.isNotEmpty == true) {
      setState(() => error = '请先完成或取消当前任务，再重启更新。');
      return;
    }
    setState(() {
      applying = true;
      error = null;
    });
    Directory? work;
    try {
      // Gate tasks before awaiting helper preparation. Core calls are drained by shutdown.
      widget.lifecycle.tasks?.closing = true;
      work = await update.prepare();
      final notices = await widget.lifecycle.tasks?.shutdown() ?? <String>[];
      if (notices.isNotEmpty) throw StateError(notices.join('\n'));
      await File('${work.path}/apply.json').writeAsString('{}', flush: true);
      await widget.lifecycle.finishExit();
      if (widget.lifecycle.error != null) {
        throw StateError('${widget.lifecycle.error}');
      }
    } catch (e) {
      if (work != null) {
        await File('${work.path}/abort.json').writeAsString('{}', flush: true);
      }
      widget.lifecycle.returnToApp();
      update.result = {...update.result, 'state': 'failed', 'error': '$e'};
      if (mounted) {
        setState(() {
          error = e;
          applying = false;
        });
      }
    }
  }

  Future<void> showUpdates({bool check = true}) async {
    setState(() => error = null);
    if (check &&
        widget.update.result['state'] != 'ready' &&
        widget.update.result['state'] != 'downloading') {
      unawaited(widget.update.check(manual: true));
    }
    await showDialog<void>(
      context: context,
      builder: (dialogContext) => StatefulBuilder(
        builder: (context, refresh) => ListenableBuilder(
          listenable: widget.update,
          builder: (context, _) {
            final u = widget.update, state = u.result['state'];
            return AlertDialog(
              title: const Text('应用更新'),
              content: SizedBox(
                width: 560,
                child: SingleChildScrollView(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text('当前版本 ${u.current}'),
                      if (u.outcome != null) ...[
                        const SizedBox(height: 12),
                        SelectableText(
                          u.outcome!['state'] == 'completed'
                              ? '上次更新已完成：${u.outcome!['version']}'
                              : '上次安装结果：${appUpdateErrorText(u.outcome!['error'] ?? u.outcome!['message'])}',
                        ),
                        const Divider(height: 24),
                      ],
                      if (state == 'checking')
                        const Text('正在检查 GitHub Release…'),
                      if (state == 'up_to_date') const Text('当前已是最新正式版。'),
                      if (state == 'failed' || state == 'unavailable') ...[
                        SelectableText(
                          state == 'failed'
                              ? '本次更新操作失败：${appUpdateErrorText(u.result['error'])}'
                              : '${u.result['error']}',
                        ),
                        if (state == 'failed')
                          TextButton(
                            onPressed: () {
                              refresh(() => error = null);
                              unawaited(u.check(manual: true));
                            },
                            child: const Text('重新检查'),
                          ),
                      ],
                      if (error != null && state != 'failed')
                        SelectableText(appUpdateErrorText(error)),
                      if (state == 'available' || state == 'ready') ...[
                        Text(
                          '${u.current} → ${u.result['tag']} · ${u.result['published_at'] ?? ''}',
                        ),
                        SelectableText('${u.result['notes']}'),
                        Wrap(
                          spacing: 12,
                          runSpacing: 8,
                          children: [
                            FilledButton(
                              onPressed: applying
                                  ? null
                                  : () async {
                                      if (state == 'ready') {
                                        final pending = apply();
                                        refresh(() {});
                                        await pending;
                                        if (context.mounted) refresh(() {});
                                      } else {
                                        await u.download();
                                      }
                                    },
                              child: Text(
                                applying
                                    ? '正在准备重启…'
                                    : state == 'ready'
                                    ? '重启并更新'
                                    : '下载更新',
                              ),
                            ),
                            TextButton(
                              onPressed: applying
                                  ? null
                                  : () => launchUrl(
                                      Uri.parse(u.result['page'] as String),
                                    ),
                              child: const Text('查看 Release'),
                            ),
                            TextButton(
                              onPressed: applying
                                  ? null
                                  : () async {
                                      await u.skip();
                                      if (dialogContext.mounted) {
                                        Navigator.pop(dialogContext);
                                      }
                                    },
                              child: const Text('跳过此版本'),
                            ),
                          ],
                        ),
                      ],
                      if (state == 'downloading') ...[
                        const SizedBox(height: 16),
                        Text(u.downloadLabel),
                        const SizedBox(height: 8),
                        LinearProgressIndicator(
                          value: u.total > 0
                              ? (u.bytes / u.total).clamp(0.0, 1.0)
                              : null,
                          semanticsLabel: u.downloadLabel,
                        ),
                        const SizedBox(height: 8),
                        Text(
                          u.total > 0
                              ? '${(u.bytes / u.total * 100).clamp(0, 100).toStringAsFixed(0)}% · ${_downloadSize(u.bytes)} / ${_downloadSize(u.total)}'
                              : '已下载 ${_downloadSize(u.bytes)}',
                        ),
                        TextButton(
                          onPressed: u.cancelDownload,
                          child: const Text('取消下载'),
                        ),
                      ],
                    ],
                  ),
                ),
              ),
              actions: [
                TextButton(
                  onPressed: applying
                      ? null
                      : () => Navigator.pop(dialogContext),
                  child: const Text('关闭'),
                ),
              ],
            );
          },
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.update,
    builder: (context, _) {
      final u = widget.update;
      final visible =
          !u.lastCheckWasManual &&
          u.result['state'] == 'available' &&
          !u.skipped &&
          dismissed != u.result['tag'];
      return Column(
        children: [
          Expanded(child: widget.child),
          Material(
            color: Theme.of(context).colorScheme.surfaceContainer,
            child: SafeArea(
              top: false,
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 16,
                  vertical: 4,
                ),
                child: Column(
                  children: [
                    if (visible)
                      Row(
                        children: [
                          Expanded(child: Text('发现新版本 ${u.result['tag']}')),
                          TextButton(
                            onPressed: () => showUpdates(check: false),
                            child: const Text('查看更新'),
                          ),
                          TextButton(
                            onPressed: () => setState(
                              () => dismissed = u.result['tag'] as String?,
                            ),
                            child: const Text('稍后'),
                          ),
                        ],
                      ),
                    Row(
                      children: [
                        Expanded(child: Text('版本 ${u.current}')),
                        TextButton(
                          onPressed: applying ? null : showUpdates,
                          child: const Text('检查更新'),
                        ),
                      ],
                    ),
                  ],
                ),
              ),
            ),
          ),
        ],
      );
    },
  );
}

String _downloadSize(int bytes) {
  if (bytes < 1024) return '$bytes B';
  if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(1)} KB';
  return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
}
