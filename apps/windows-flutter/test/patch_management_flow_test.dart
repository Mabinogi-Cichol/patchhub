import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/core_task.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/pages/library_page.dart';
import 'package:patchhub_windows/pages/source_pages.dart';

import 'installation_flow_test.dart'
    show InstallFixtureModel, FixtureDesktop, mountPatch;
import 'library_page_test.dart' show FixtureClient;
import 'modern_layout_test.dart' show ModernFixtureModel;
import 'support/process_fixture_client.dart';

Map<String, dynamic> fixtureCandidate() {
  final corpus = jsonDecode(
    File('../../packages/contracts/fixtures/v5/cases.json').readAsStringSync(),
  );
  return Map<String, dynamic>.from(
    (corpus['events'] as List).firstWhere(
      (item) => item['name'] == 'match_update',
    )['event']['candidates'][0],
  );
}

class ProjectionClient extends FixtureClient {
  late Map<String, dynamic> projected;
  @override
  Future<Object?> call(
    String method, [
    Map<String, Object?> args = const {},
  ]) async {
    if (method == 'project_instances') {
      return {
        'settings_revision': 1,
        'instances': [projected],
      };
    }
    return super.call(method, args);
  }
}

class QueryModel extends AppModel {
  QueryModel() : super(ProcessFixtureClient('cancel_wait')) {
    snapshot = ModernFixtureModel().snapshot;
  }
  @override
  Future<void> initialize() async {}
  @override
  Future<List<Map<String, dynamic>>> gameSources(String id) async => [];
  @override
  Future<void> loadSources() async {}
}

void main() {
  testWidgets('source-only entry installs user URL through the same task', (
    tester,
  ) async {
    final model = InstallFixtureModel('chunked');
    final row = model.snapshot!.rows.single;
    final candidate = fixtureCandidate();
    row.patches.clear();
    row.value['discovery'] = {
      'status': 'publisher_match',
      'candidates': [candidate],
      'diagnostics': [],
    };
    await mountPatch(tester, model, FixtureDesktop());
    expect(find.textContaining('待核对'), findsNothing);
    expect(find.text('未找到下载链接'), findsWidgets);
    final install = find.widgetWithText(FilledButton, '下载并安装');
    expect(tester.widget<FilledButton>(install).onPressed, isNull);
    await tester.enterText(
      find.byType(TextField),
      'https://mirror.example/download',
    );
    await tester.pump();
    await tester.runAsync(() => tester.tap(install));
    await tester.pump();
    final task = model.tasks.installation!;
    expect(task.arguments, {
      'instance_id': row.id,
      'manual_download': {
        'source_id': candidate['source_id'],
        'candidate_id': candidate['candidate_id'],
        'url': 'https://mirror.example/download',
      },
      'operation': 'overwrite',
      'cleanup_policy': 'disabled',
    });
    await tester.runAsync(() => task.done);
    await tester.pumpAndSettle();
    expect(find.text('安装完成'), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
    model.dispose();
  });

  testWidgets(
    'default management retains game info without overview or results',
    (tester) async {
      final model = ModernFixtureModel();
      final row = model.snapshot!.rows.single;
      row.patches.clear();
      row.value['discovery'] = {
        'status': 'failed',
        'candidates': [],
        'diagnostics': [
          {'code': 'connection_failed', 'detail': 'secret query error'},
        ],
      };
      await tester.pumpWidget(PatchHubApp(home: LibraryPage(model: model)));
      await tester.pumpAndSettle();
      await tester.tap(find.text('查看全部游戏'));
      await tester.pumpAndSettle();
      await tester.tap(find.text(row.name));
      await tester.pumpAndSettle();
      expect(find.text('概览'), findsNothing);
      expect(find.text('暂无补丁结果'), findsWidgets);
      expect(find.text('重新匹配'), findsOneWidget);
      expect(find.text('查看游戏订阅'), findsOneWidget);
      expect(find.textContaining('安装路径：'), findsOneWidget);
      expect(find.textContaining('secret query error'), findsNothing);
      expect(find.textContaining('查询失败'), findsNothing);
      await tester.tap(find.text('查看游戏订阅'));
      await tester.pumpAndSettle();
      expect(find.text('仅控制当前游戏使用的来源；全局停用的来源不会参与匹配。'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );

  test(
    'failed projection preserves active sources but not removed actions',
    () async {
      final client = ProjectionClient();
      final model = AppModel(client);
      final previous = ModernFixtureModel().snapshot!;
      model.snapshot = previous;
      final row = previous.rows.single;
      final candidate = fixtureCandidate();
      row.patches.single['source_id'] = candidate['source_id'];
      row.value['discovery'] = {
        'status': 'publisher_match',
        'candidates': [candidate],
        'diagnostics': [],
      };
      client.projected = {
        ...row.value,
        'patches': [],
        'sources': [
          {'id': candidate['source_id'], 'enabled': true, 'subscribed': true},
        ],
        'discovery': {
          'status': 'incomplete',
          'candidates': [],
          'diagnostics': [],
        },
      };
      await model.project({row.id});
      expect(model.snapshot!.rows.single.patches, isNotEmpty);
      expect(model.snapshot!.rows.single.discovery!['candidates'], isNotEmpty);
      client.projected['sources'] = [
        {'id': candidate['source_id'], 'enabled': true, 'subscribed': false},
      ];
      await model.project({row.id});
      expect(model.snapshot!.rows.single.patches, isEmpty);
      expect(model.snapshot!.rows.single.discovery!['candidates'], isEmpty);
      model.dispose();
    },
  );

  testWidgets(
    'query spinner stops on cancel and error cannot leak to global sources',
    (tester) async {
      final model = QueryModel();
      final row = model.snapshot!.rows.single;
      await tester.pumpWidget(PatchHubApp(home: LibraryPage(model: model)));
      await tester.pumpAndSettle();
      await tester.tap(find.text(row.name));
      await tester.pumpAndSettle();
      final rematch = find.byKey(const ValueKey('game-rematch'));
      final idleBounds = tester.getRect(rematch);
      late CoreTask task;
      await tester.runAsync(() async {
        task = model.startSource('rematch_game', {'instance_id': row.id});
      });
      await tester.pump();
      expect(find.text('正在匹配'), findsOneWidget);
      expect(tester.widget<OutlinedButton>(rematch).onPressed, isNull);
      expect(tester.getRect(rematch), idleBounds);
      expect(model.queryRunning(row), isTrue);
      await tester.runAsync(() async {
        await task.cancel();
        await task.done;
        await Future<void>.delayed(const Duration(milliseconds: 40));
      });
      await tester.pumpAndSettle();
      expect(model.queryRunning(row), isFalse);
      expect(find.text('正在匹配'), findsNothing);
      expect(tester.widget<OutlinedButton>(rematch).onPressed, isNotNull);
      expect(tester.getRect(rematch), idleBounds);
      expect(model.snapshot!.rows.single.patches, isNotEmpty);
      await tester.runAsync(() async {
        final partial = model.startSource('match_games', {
          'instances': ['another-instance'],
        });
        expect(model.queryRunning(row), isFalse);
        await partial.cancel();
        await partial.done;
        await Future<void>.delayed(const Duration(milliseconds: 40));
      });
      await tester.pumpWidget(
        PatchHubApp(
          home: Scaffold(body: GlobalSourcesPage(model: model)),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.textContaining('task_cancelled'), findsNothing);
      expect(find.text('任务诊断'), findsNothing);
      expect(find.textContaining('任务结果：'), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );
}
