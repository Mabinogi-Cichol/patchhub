import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/pages/source_pages.dart';

import 'library_page_test.dart' show FixtureClient;

class SettingsClient extends FixtureClient {
  final calls = <String>[];
  bool projectFails = false;
  @override
  Future<Object?> call(
    String method, [
    Map<String, Object?> args = const {},
  ]) async {
    calls.add(method);
    if (method == 'set_source_enabled') return {'settings_revision': 71};
    if (method == 'list_sources') return <Map<String, dynamic>>[];
    if (method == 'project_instances') {
      if (projectFails) throw StateError('投影读取失败');
      return {'settings_revision': 71, 'instances': []};
    }
    return super.call(method, args);
  }
}

void main() {
  test('scheduler only starts due enabled publisher indexes, without duplicate jobs', () {
    Map<String, dynamic> source(
      String id, {
      bool enabled = true,
      String role = 'publisher',
      int interval = 3,
      int? due = 99,
    }) => {
      'id': id,
      'enabled': enabled,
      'role': role,
      'interval_hours': interval,
      'next_due_at': due,
    };
    expect(
      dueSources(
        [
          source('due'),
          source('disabled', enabled: false),
          source('hgamer', role: 'fallback_indexer'),
          source('manual', interval: 0),
          source('future', due: 101),
          source('running'),
          source('unknown', due: null),
        ],
        100,
        (id) => id == 'running',
      ),
      ['due'],
    );
  });
  test(
    'saved revision survives projection failure and remains visibly distinct',
    () async {
      final client = SettingsClient()..projectFails = true;
      final model = AppModel(client);
      await model.refresh();
      await model.mutate('set_source_enabled', {
        'source_id': 'publisher',
        'enabled': false,
      });
      expect(model.settingsRevision, 71);
      expect(model.stateRefreshError.toString(), contains('设置已保存'));
      await model.loadSources();
      expect(model.stateRefreshError, isNotNull);
      client.projectFails = false;
      await model.project({'one'});
      expect(model.stateRefreshError, isNull);
      model.dispose();
    },
  );
  testWidgets(
    'source cards default folded; opening and folding performs no request',
    (tester) async {
      final client = SettingsClient();
      final model = AppModel(client)
        ..sources = [
          {
            'id': 'publisher',
            'name': '测试发布方',
            'enabled': true,
            'role': 'publisher',
            'status': 'source_refreshed',
            'checked_at': null,
            'interval_hours': 0,
            'entry_count': 8,
            'complete': true,
            'cached_at': null,
            'next_due_at': null,
            'revision': 1,
            'pages': <String>[],
          },
        ];
      await tester.pumpWidget(
        PatchHubApp(
          home: Scaffold(body: GlobalSourcesPage(model: model)),
        ),
      );
      expect(find.text('启用此来源').hitTestable(), findsNothing);
      await tester.tap(find.text('测试发布方'));
      await tester.pumpAndSettle();
      expect(find.text('启用此来源').hitTestable(), findsOneWidget);
      await tester.tap(find.text('测试发布方'));
      await tester.pumpAndSettle();
      expect(find.text('启用此来源').hitTestable(), findsNothing);
      expect(client.calls, isEmpty);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );
}
