import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_paths.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/models/library.dart';
import 'package:patchhub_windows/pages/script_source_editor.dart';
import 'package:patchhub_windows/pages/source_pages.dart';

import 'library_page_test.dart' show FixtureClient;

class ScriptClient extends FixtureClient {
  Map<String, dynamic>? document;
  bool reject = false;
  bool rejectRead = false;
  bool rejectRefresh = false;
  final calls = <String>[];
  final writes = <Map<String, Object?>>[];
  Map<String, dynamic> get source => {
    ...document!['rule'] as Map<String, dynamic>,
    'status': 'not_refreshed',
    'interval_hours': 72,
    'entry_count': 0,
    'complete': false,
    'script': {'api_version': 1, 'sha256': document!['sha256']},
  };
  @override
  Future<Object?> call(
    String method, [
    Map<String, Object?> args = const {},
  ]) async {
    calls.add(method);
    if (method == 'get_script_source') {
      if (rejectRead) throw StateError('读取失败');
      return document;
    }
    if (method == 'import_script_source') {
      writes.add(Map.of(args));
      if (reject) {
        throw StateError('script_source_syntax_error: source/custom.lua:4: 错误');
      }
      document = {
        'rule': args['rule'],
        'api_version': args['api_version'],
        'code': args['code'],
        'sha256': 'a' * 64,
      };
      return {'rule': args['rule'], 'settings_revision': 3};
    }
    if (method == 'list_sources') {
      if (rejectRefresh) throw StateError('列表读取失败');
      return document == null ? [] : [source];
    }
    if (method == 'project_instances') {
      return {'settings_revision': 3, 'instances': []};
    }
    return super.call(method, args);
  }
}

class SourceAdminClient extends FixtureClient {
  SourceAdminClient(this.sources);
  List<Map<String, dynamic>> sources;
  final calls = <String>[];
  final writes = <Map<String, Object?>>[];
  @override
  Future<Object?> call(
    String method, [
    Map<String, Object?> args = const {},
  ]) async {
    calls.add(method);
    if (method == 'list_sources') return sources;
    if (method == 'get_script_source') {
      final source = sources.firstWhere(
        (item) => item['id'] == args['source_id'],
      );
      return {
        'rule': {
          for (final entry in source.entries)
            if (![
              'status',
              'interval_hours',
              'entry_count',
              'complete',
              'script',
              'origin',
            ].contains(entry.key))
              entry.key: entry.value,
        },
        'api_version': 1,
        'code': 'return {}',
        'sha256': 'a' * 64,
      };
    }
    if (method == 'remove_source') {
      writes.add(Map.of(args));
      sources = sources
          .where((item) => item['id'] != args['source_id'])
          .toList();
      return {'settings_revision': 4, 'source_id': args['source_id']};
    }
    if (method == 'project_instances') {
      return {'settings_revision': 4, 'instances': <Object?>[]};
    }
    return super.call(method, args);
  }
}

Map<String, dynamic> adminSource(String id, String name, String origin) => {
  'id': id,
  'name': name,
  'adapter': 'lua',
  'enabled': true,
  'role': 'publisher',
  'origin': origin,
  'status': 'not_refreshed',
  'interval_hours': 0,
  'entry_count': 0,
  'complete': false,
  'pages': <String>[],
  'page_hosts': <String>[],
  'download_hosts': <String>[],
  'root_layout': 'game_root',
  'script': {'api_version': 1, 'sha256': 'a' * 64},
};

Future<void> enterField(WidgetTester tester, String key, String value) async {
  final field = find.byKey(ValueKey('script-$key'));
  await tester.ensureVisible(field);
  await tester.enterText(field, value);
}

Future<void> fillPublisher(WidgetTester tester) async {
  final fixture = jsonDecode(
    File(
      '../../docs/features/script-source-adapters/examples/publisher/source.json',
    ).readAsStringSync(),
  ) as Map<String, dynamic>;
  await enterField(tester, 'id', fixture['id'] as String);
  await enterField(tester, 'name', fixture['name'] as String);
  for (final key in ['pages', 'page_hosts', 'download_hosts']) {
    await enterField(tester, key, (fixture[key] as List).join('\n'));
  }
}

void main() {
  final emptyCore = Platform.environment['PATCHHUB_EMPTY_WINDOWS_CORE'];
  testWidgets('five bundled sources roundtrip through the empty Core form', (
    tester,
  ) async {
    final sandbox = Directory.systemTemp.createTempSync(
      'patchhub-empty-editor-',
    );
    addTearDown(() => sandbox.deleteSync(recursive: true));
    final client = CoreClient(
      CorePaths(binary: emptyCore!, stateDirectory: sandbox.path),
      WireProtocol(
        jsonDecode(File('assets/protocol-v5.schema.json').readAsStringSync())
            as Map<String, dynamic>,
      ),
    );
    const resource = '../../crates/core/resources';
    final config =
        jsonDecode(File('$resource/sources.json').readAsStringSync()) as Map;
    final files = config['bundled_scripts'] as Map;
    expect(await tester.runAsync(() => client.call('list_sources')), isEmpty);

    for (final rule in (config['sources'] as List).cast<Map>()) {
      final code = File('$resource/source-scripts/${files[rule['adapter']]}')
          .readAsStringSync();
      Future<void>? saved;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () => showDialog<void>(
                  context: context,
                  builder: (_) => ScriptSourceEditor(
                    onSave: (params) {
                      return saved = client
                          .call('import_script_source', params)
                          .then((_) {});
                    },
                  ),
                ),
                child: const Text('打开'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('打开'));
      await tester.pumpAndSettle();
      for (final key in ['id', 'name', 'priority']) {
        if (rule[key] != null) {
          await enterField(tester, key, rule[key].toString());
        }
      }
      if (rule['role'] == 'fallback_indexer') {
        final roles = find.byType(DropdownButtonFormField<String>);
        await tester.ensureVisible(roles);
        await tester.tap(roles);
        await tester.pumpAndSettle();
        await tester.tap(find.text('补查来源').last);
        await tester.pumpAndSettle();
      } else if (rule['discovery_only'] == true) {
        final discovery = find.widgetWithText(SwitchListTile, '仅发现');
        await tester.ensureVisible(discovery);
        await tester.tap(discovery);
        await tester.pumpAndSettle();
      }
      for (final key in ['pages', 'page_hosts', 'download_hosts']) {
        await enterField(tester, key, (rule[key] as List).join('\n'));
      }
      await enterField(tester, 'code', code);
      await tester.runAsync(() async {
        await tester.tap(find.text('保存并导入'));
        await saved;
      });
      await tester.pumpAndSettle();
      expect(find.byType(ScriptSourceEditor), findsNothing);
      final record = await tester.runAsync(
        () => client.call('get_script_source', {'source_id': rule['id']}),
      ) as Map;
      for (final entry in rule.entries.where(
        (entry) => entry.key != 'adapter',
      )) {
        final actual = record['rule'][entry.key];
        expect(
          entry.key == 'download_hosts' ? actual ?? <String>[] : actual,
          entry.value,
          reason: '${rule['id']}/${entry.key}',
        );
      }
      expect(record['rule']['adapter'], 'lua');
      expect(record['code'], code);
    }
    expect(
      await tester.runAsync(() => client.call('list_sources')),
      hasLength(5),
    );
  }, skip: emptyCore == null);

  for (final brightness in Brightness.values) {
    testWidgets(
      'failed import retains named fields and large Lua draft in $brightness',
      (tester) async {
        tester.view.physicalSize = const Size(580, 600);
        tester.view.devicePixelRatio = 1;
        tester.platformDispatcher.textScaleFactorTestValue = 1.5;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
        Map<String, Object?>? received;
        await tester.pumpWidget(
          MaterialApp(
            theme: ThemeData(brightness: brightness),
            home: Builder(
              builder: (context) => Scaffold(
                body: TextButton(
                  onPressed: () => showDialog<void>(
                    context: context,
                    builder: (_) => ScriptSourceEditor(
                      onSave: (params) async {
                        received = params;
                        throw StateError(
                          'script_source_syntax_error: source/custom.lua:4: 完整错误',
                        );
                      },
                    ),
                  ),
                  child: const Text('打开'),
                ),
              ),
            ),
          ),
        );
        await tester.tap(find.text('打开'));
        await tester.pumpAndSettle();
        final code = '-- ${'中文' * 18000}\nreturn {}';
        await fillPublisher(tester);
        await enterField(tester, 'code', code);
        await tester.tap(find.text('保存并导入'));
        await tester.pumpAndSettle();
        expect(received!['code'], code);
        expect(received!['replace'], false);
        expect(find.byType(ScriptSourceEditor), findsOneWidget);
        expect(
          tester
              .widget<TextField>(find.byKey(const ValueKey('script-code')))
              .controller!
              .text,
          code,
        );
        expect(find.textContaining('完整错误'), findsOneWidget);
        expect((received!['rule'] as Map)['id'], 'my-script-publisher');
        expect(
          tester
              .widget<TextField>(find.byKey(const ValueKey('script-id')))
              .controller!
              .text,
          'my-script-publisher',
        );
        expect(tester.takeException(), isNull);
      },
    );
  }

  testWidgets(
    'builtin sources are read-only and custom sources can be deleted',
    (tester) async {
      tester.view.physicalSize = const Size(900, 760);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final client = SourceAdminClient([
        adminSource('patchmoe', 'Shiravune', 'builtin'),
        adminSource('mine', '我的脚本', 'added'),
      ]);
      final model = AppModel(client)..sources = client.sources;
      addTearDown(model.dispose);
      await tester.pumpWidget(
        PatchHubApp(
          home: Scaffold(body: GlobalSourcesPage(model: model)),
        ),
      );
      await tester.tap(find.text('Shiravune'));
      await tester.pumpAndSettle();
      expect(find.textContaining('内置来源随版本更新'), findsOneWidget);
      expect(find.text('编辑脚本'), findsNothing);
      expect(find.text('删除来源'), findsNothing);
      await tester.ensureVisible(find.text('查看脚本'));
      await tester.tap(find.text('查看脚本'));
      await tester.pumpAndSettle();
      expect(find.text('查看脚本：patchmoe'), findsOneWidget);
      expect(find.byType(FilledButton), findsNothing);
      final code = tester.widget<TextField>(
        find.byKey(const ValueKey('script-code')),
      );
      expect(code.enabled, isFalse);
      String? copied;
      final messenger = tester.binding.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
        if (call.method == 'Clipboard.setData') {
          copied = (call.arguments as Map)['text'] as String;
        }
        return null;
      });
      addTearDown(
        () => messenger.setMockMethodCallHandler(SystemChannels.platform, null),
      );
      await tester.tap(find.text('复制 Lua 源码'));
      await tester.pumpAndSettle();
      expect(copied, code.controller!.text);

      expect(find.text('关闭'), findsOneWidget);
      await tester.tap(find.text('关闭'));
      await tester.pumpAndSettle();
      expect(find.byType(ScriptSourceEditor), findsNothing);

      await tester.ensureVisible(find.text('我的脚本'));
      await tester.tap(find.text('我的脚本'));
      await tester.pumpAndSettle();
      expect(find.text('编辑脚本'), findsOneWidget);
      await tester.ensureVisible(find.text('删除来源'));
      await tester.tap(find.text('删除来源'));
      await tester.pumpAndSettle();
      expect(find.text('删除来源：我的脚本'), findsOneWidget);
      await tester.tap(find.text('取消'));
      await tester.pumpAndSettle();
      expect(client.writes, isEmpty);
      await tester.tap(find.text('删除来源'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('删除'));
      await tester.pumpAndSettle();
      expect(client.writes.single['source_id'], 'mine');
      expect(client.calls, contains('remove_source'));
      expect(find.text('我的脚本'), findsNothing);
      expect(client.sources.map((source) => source['id']), ['patchmoe']);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets(
    'page imports, refreshes projections, reads and explicitly replaces the same ID',
    (tester) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final client = ScriptClient();
      final model = AppModel(client);
      model.snapshot = ScanSnapshot(
        await client.call('scan') as Map<String, dynamic>,
      );
      final previous = model.snapshot!.rows.single;
      model.snapshot!.rows[0] = GameRow({
        ...previous.value,
        'patches': [
          {'source_id': 'old-source', 'release_id': 'old-dynamic'},
          {'release_id': 'signed'},
        ],
        'discovery': {
          'status': 'incomplete',
          'candidates': [
            {'source_id': 'old-source', 'candidate_id': 'old'},
          ],
          'diagnostics': [],
        },
      });
      addTearDown(model.dispose);
      await tester.pumpWidget(
        PatchHubApp(
          home: Scaffold(body: GlobalSourcesPage(model: model)),
        ),
      );
      await tester.tap(find.text('导入脚本来源'));
      await tester.pumpAndSettle();
      await fillPublisher(tester);
      await enterField(tester, 'code', 'return {}');
      await tester.tap(find.text('保存并导入'));
      await tester.pumpAndSettle();
      expect(
        client.calls,
        containsAllInOrder([
          'import_script_source',
          'list_sources',
          'project_instances',
        ]),
      );
      expect(
        model.snapshot!.rows.single.patches.map((patch) => patch['release_id']),
        ['signed'],
      );
      expect(model.snapshot!.rows.single.discovery!['candidates'], isEmpty);
      expect(find.textContaining('脚本已保存，尚未执行'), findsOneWidget);
      await tester.tap(find.text('脚本发行商示例'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('编辑脚本'));
      await tester.tap(find.text('编辑脚本'));
      await tester.pumpAndSettle();
      expect(client.calls.last, 'get_script_source');
      expect(
        tester
            .widget<TextField>(find.byKey(const ValueKey('script-code')))
            .controller!
            .text,
        'return {}',
      );
      expect(
        tester
            .widget<TextField>(find.byKey(const ValueKey('script-id')))
            .enabled,
        isFalse,
      );
      await enterField(tester, 'code', 'return { updated = true }');
      client.reject = true;
      await tester.tap(find.text('保存并替换 my-script-publisher'));
      await tester.pumpAndSettle();
      expect(client.document!['code'], 'return {}');
      expect(find.byType(ScriptSourceEditor), findsOneWidget);
      client.reject = false;
      await tester.tap(find.text('保存并替换 my-script-publisher'));
      await tester.pumpAndSettle();
      expect(client.writes.last['replace'], true);
      await tester.ensureVisible(find.text('编辑脚本'));
      await tester.tap(find.text('编辑脚本'));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<TextField>(find.byKey(const ValueKey('script-code')))
            .controller!
            .text,
        'return { updated = true }',
      );
      await tester.tap(find.text('取消'));
      await tester.pumpAndSettle();
      client.rejectRead = true;
      await tester.ensureVisible(find.text('编辑脚本'));
      await tester.tap(find.text('编辑脚本'));
      await tester.pumpAndSettle();
      expect(find.byType(ScriptSourceEditor), findsNothing);
      expect(find.textContaining('读取失败'), findsOneWidget);
      client.rejectRead = false;
      client.rejectRefresh = true;
      await tester.ensureVisible(find.text('编辑脚本'));
      await tester.tap(find.text('编辑脚本'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const ValueKey('script-code')),
        'return { saved = true }',
      );
      await tester.tap(find.text('保存并替换 my-script-publisher'));
      await tester.pumpAndSettle();
      expect(find.byType(ScriptSourceEditor), findsNothing);
      expect(client.document!['code'], 'return { saved = true }');
      expect(model.stateRefreshError.toString(), contains('设置已保存'));
      expect(tester.takeException(), isNull);
    },
  );
}
