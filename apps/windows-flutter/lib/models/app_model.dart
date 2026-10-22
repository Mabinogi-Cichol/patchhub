import 'dart:async';

import 'package:flutter/foundation.dart';

import '../host/core_client.dart';
import '../host/core_task.dart';
import '../host/source_tasks.dart';
import 'library.dart';

List<String> dueSources(
  List<Map<String, dynamic>> sources,
  int now,
  bool Function(String) running,
) => sources
    .where(
      (source) =>
          source['enabled'] == true &&
          source['role'] == 'publisher' &&
          (source['interval_hours'] as int) > 0 &&
          source['next_due_at'] is int &&
          (source['next_due_at'] as int) <= now &&
          !running(source['id'] as String),
    )
    .map((source) => source['id'] as String)
    .toList();

class AppModel extends ChangeNotifier {
  AppModel(this.client) : tasks = AppTasks(client) {
    tasks.addListener(_taskChanged);
  }
  final CoreClient client;
  final AppTasks tasks;
  ScanSnapshot? snapshot;
  List<Map<String, dynamic>> sources = [];
  List<Map<String, dynamic>> transactions = [];
  Object? error;
  Object? sourceError;
  Object? transactionError;
  Object? stateRefreshError;
  int? settingsRevision;
  String? preferredLanguage;
  bool scanning = false;
  bool _initialized = false;
  bool _ticking = false;
  Timer? _scheduler;
  final _watched = <CoreTask>{};

  Future<void> initialize() async {
    if (_initialized) return;
    _initialized = true;
    await refresh();
    _startScheduler();
  }

  void _taskChanged() {
    if (tasks.closing) {
      _scheduler?.cancel();
      _scheduler = null;
    } else if (_initialized && _scheduler == null) {
      _startScheduler();
    }
    notifyListeners();
  }

  void _startScheduler() {
    if (tasks.closing || _scheduler != null) return;
    _scheduler = Timer.periodic(
      const Duration(seconds: 60),
      (_) => schedulerTick(),
    );
    schedulerTick();
  }

  Future<void> schedulerTick() async {
    if (_ticking || tasks.closing) return;
    _ticking = true;
    try {
      await loadSources();
      if (tasks.closing) return;
      for (final sourceId in dueSources(
        sources,
        DateTime.now().millisecondsSinceEpoch ~/ 1000,
        (id) => tasks.sources['update_source:$id']?.running == true,
      )) {
        startSource('update_source', {'source_id': sourceId, 'force': false});
      }
    } catch (failure) {
      sourceError = failure;
      notifyListeners();
    } finally {
      _ticking = false;
    }
  }

  Future<bool> refresh() async {
    if (scanning || tasks.closing) return false;
    scanning = true;
    error = null;
    notifyListeners();
    try {
      final value = await client.call('scan') as Map<String, dynamic>;
      snapshot = ScanSnapshot(value);
      stateRefreshError = null;
      settingsRevision = value['settings_revision'] as int?;
      notifyListeners();
      // The journal read follows scan; do not parallelize these operations.
      try {
        await loadTransactions();
      } catch (failure) {
        transactionError = failure;
      }
      try {
        await loadSources();
      } catch (failure) {
        sourceError = failure;
      }
      final pending = snapshot!.rows
          .where(
            (row) =>
                row.game['availability'] == 'installed' &&
                (row.discovery == null ||
                    row.discovery!['status'] == 'pending'),
          )
          .map((row) => row.id)
          .toList();
      if (!tasks.closing &&
          sources.any(
            (source) =>
                source['enabled'] == true && source['role'] == 'publisher',
          ) &&
          pending.isNotEmpty &&
          tasks.sources['match_games:match']?.running != true) {
        startSource('match_games', {'instances': pending});
      }
      return true;
    } catch (failure) {
      error = failure;
      return false;
    } finally {
      scanning = false;
      notifyListeners();
    }
  }

  Future<void> loadTransactions() async {
    final value = (await client.call('list_transactions') as List)
        .cast<Map<String, dynamic>>();
    transactions = value;
    transactionError = null;
    notifyListeners();
  }

  Future<void> loadSources() async {
    final value = (await client.call('list_sources') as List)
        .cast<Map<String, dynamic>>();
    sources = value;
    sourceError = null;
    notifyListeners();
  }

  Future<List<Map<String, dynamic>>> gameSources(String instanceId) async =>
      (await client.call('list_game_sources', {
        'instance_id': instanceId,
      }) as List).cast<Map<String, dynamic>>();

  Future<void> mutate(String method, Map<String, Object?> args) async {
    final value = await client.call(method, args) as Map<String, dynamic>;
    settingsRevision = value['settings_revision'] as int;
    try {
      await loadSources();
      if (snapshot != null) {
        final ids = args['instance_id'] is String
            ? {args['instance_id'] as String}
            : snapshot!.rows.map((row) => row.id).toSet();
        await project(ids);
      }
    } catch (failure) {
      stateRefreshError = CoreError(
        'state_refresh_failed',
        '设置已保存（版本 $settingsRevision），状态刷新失败：$failure',
      );
      notifyListeners();
    }
  }

  Future<void> project(Set<String> instanceIds) async {
    if (instanceIds.isEmpty) return;
    final value = await client.call('project_instances', {
      'instance_ids': instanceIds.toList(),
    }) as Map<String, dynamic>;
    settingsRevision = value['settings_revision'] as int?;
    final rows = (value['instances'] as List).map(
      (row) => GameRow(row as Map<String, dynamic>),
    );
    for (final row in rows) {
      final index =
          snapshot?.rows.indexWhere((item) => item.id == row.id) ?? -1;
      if (index >= 0) snapshot!.rows[index] = row;
    }
    stateRefreshError = null;
    notifyListeners();
  }

  CoreTask startSource(String method, Map<String, Object?> arguments) {
    final task = tasks.startSource(method, arguments);
    if (!_watched.add(task)) return task;
    final touched = <String>{};
    task.events.listen((event) {
      settingsRevision = event['settings_revision'] as int? ?? settingsRevision;
      if (event['event'] == 'match_update') {
        for (final row in snapshot?.rows ?? <GameRow>[]) {
          if (row.id == event['instance_id'] ||
              row.game['appid'] == event['appid']) {
            touched.add(row.id);
            final index = snapshot!.rows.indexOf(row);
            snapshot!.rows[index] = GameRow({
              ...row.value,
              'discovery': {
                'status': event['status'],
                'candidates': event['candidates'],
                'diagnostics': event['diagnostics'],
              },
            });
          }
        }
      }
      notifyListeners();
    });
    task.done.then((_) async {
      try {
        if (task.result case final Map result) {
          settingsRevision =
              result['settings_revision'] as int? ?? settingsRevision;
          final appids = method == 'update_source'
              ? result['affected_appids'] as List
              : (result['results'] as List)
                    .map((match) => match['appid'])
                    .toList();
          touched.addAll(
            (snapshot?.rows ?? <GameRow>[])
                .where((row) => appids.contains(row.game['appid']))
                .map((row) => row.id),
          );
        }
        await project(touched);
        await loadSources();
      } catch (failure) {
        stateRefreshError = CoreError(
          'state_refresh_failed',
          '来源任务已结束，安装能力刷新失败：$failure',
        );
      } finally {
        _watched.remove(task);
        notifyListeners();
      }
    });
    return task;
  }

  void setPreferredLanguage(String? value) {
    preferredLanguage = value;
    notifyListeners();
  }

  void rematch(GameRow row) => startSource('rematch_game', {
    'instance_id': row.id,
    if (preferredLanguage != null) 'preferred_language': preferredLanguage,
  });

  @override
  void dispose() {
    _scheduler?.cancel();
    tasks.removeListener(_taskChanged);
    tasks.dispose();
    super.dispose();
  }
}
