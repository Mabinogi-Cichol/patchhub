import 'dart:async';

import 'package:flutter/foundation.dart';

import 'core_client.dart';
import 'core_task.dart';

/// App-owned task handles. Source keys preserve Tauri's distinct concurrency
/// semantics; installation has its own single active slot.
class AppTasks extends ChangeNotifier {
  AppTasks(this.client);
  final CoreClient client;
  final Map<String, CoreTask> sources = {};
  CoreTask? installation;
  bool closing = false;

  Iterable<CoreTask> get active sync* {
    if (installation case final task? when task.running) yield task;
    yield* sources.values.where((task) => task.running);
  }

  CoreTask startSource(String method, Map<String, Object?> arguments) {
    if (closing) throw const CoreError('app_closing');
    final key = switch (method) {
      'update_source' => 'update_source:${arguments['source_id']}',
      'match_games' => 'match_games:match',
      'rematch_game' when arguments['instance_id'] != null =>
        'rematch_game:instance:${arguments['instance_id']}',
      'rematch_game' when arguments['appid'] != null =>
        'rematch_game:appid:${arguments['appid']}',
      _ => throw const CoreError('invalid_request'),
    };
    if (sources[key] case final existing? when existing.running) {
      return existing;
    }
    final task = CoreTask.start(client, method, arguments);
    sources[key] = task;
    _track(task);
    return task;
  }

  CoreTask startInstall(Map<String, Object?> arguments) {
    if (closing) throw const CoreError('app_closing');
    if (installation?.running == true) {
      throw const CoreError('install_in_progress');
    }
    final task = CoreTask.start(client, 'install_patch', arguments);
    installation = task;
    _track(task);
    return task;
  }

  void _track(CoreTask task) {
    task.changes.listen((_) => notifyListeners());
    notifyListeners();
  }

  Future<List<String>> shutdown() async {
    closing = true;
    notifyListeners();
    final tasks = active.toList();
    await Future.wait([
      client.cancelReads(),
      ...tasks.map((task) => task.cancel()),
    ]);
    await Future.wait(tasks.map((task) => task.done));
    await client.waitForIdle();
    return [
      for (final task in tasks) ...[
        if (task.controlError != null) '${task.method}：${task.controlError}',
        if (task.error != null &&
            ![
              'cancelled',
              'task_cancelled',
              'download_cancelled',
            ].contains(task.error!.code))
          '${task.method}：${task.error}',
        if (task.result is Map && task.result['state'] == 'failed')
          '安装未完成：${task.result['error'] ?? task.result['cause']}；请查看操作记录${task.result['direct'] == true && task.result['has_replacements'] == true ? '，直接安装可能留下不完整文件' : ''}',
        if (task.result is Map && task.result['state'] == 'needs_recovery')
          '安装仍需恢复：${task.result['transaction_id']}',
      ],
    ];
  }

  void resume() {
    client.resume();
    closing = false;
    notifyListeners();
  }
}
