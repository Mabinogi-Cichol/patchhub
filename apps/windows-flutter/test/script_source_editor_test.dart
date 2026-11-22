import 'dart:async';
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

const sampleScript = '''
return {
  meta = {
    name = "脚本发行商示例",
    pages = { "https://patches.vendor.example/" },
    page_hosts = { "patches.vendor.example" },
    download_hosts = { "downloads.vendor.example" },
  },
  collect_index = function()
    return { entries = {}, pending = {}, complete = true }
  end,
}
''';

class ScriptClient extends FixtureClient {
  Map<String, dynamic>? document;
  String role = 'publisher';
  bool reject = false;
  bool rejectRead = false;
  bool rejectRefresh = false;
  final calls = <String>[];
  final writes = <Map<String, Object?>>[];
  final startedTasks = <Map<String, Object?>>[];

  @override
  Future<Process> startProcess(String channel) async =>
      _FakeUpdateProcess(startedTasks);

  Map<String, dynamic> get source => {
    ...document!['rule'] as Map<String, dynamic>,
    'name': document!['name'],
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
    if (method == 'get_script_source_prompt') {
      return {
        'prompt':
            '请先阅读 https://raw.githubusercontent.com/Mabinogi-Cichol/patchhub/main/docs/custom-subscription-sources.md',
      };
    }
    if (method == 'get_script_source') {
      if (rejectRead) throw StateError('读取失败');
      return document;
    }
    if (method == 'import_script_source') {
      writes.add(Map.of(args));
      if (reject) {
        throw StateError('script_source_syntax_error: source/custom.lua:4: 错误');
      }
      final sourceId = args['source_id'] as String? ?? 'patches-vendor-example';
      final rule = {
        'id': sourceId,
        'adapter': 'lua',
        'enabled': true,
        'role': role,
        'official_title_match': false,
        'priority': 100,
        'pages': ['https://patches.vendor.example/'],
        'page_hosts': ['patches.vendor.example'],
        'download_hosts': ['downloads.vendor.example'],
      };
      document = {
        'source_id': sourceId,
        'name': '脚本发行商示例',
        'rule': rule,
        'api_version': args['api_version'] ?? 1,
        'code': args['code'],
        'sha256': 'a' * 64,
      };
      return {
        'rule': rule,
        'name': '脚本发行商示例',
        'settings_revision': 3,
      };
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

class _FakeUpdateProcess implements Process {
  _FakeUpdateProcess(this.startedTasks) {
    _stdinController.stream.transform(utf8.decoder).listen((chunk) {
      for (final line in chunk.split('\n')) {
        final trimmed = line.trim();
        if (!trimmed.startsWith('{')) continue;
        final decoded = jsonDecode(trimmed);
        if (decoded is! Map<String, dynamic>) continue;
        final requestId = decoded['request_id'];
        final method = decoded['method'];
        final params = decoded['params'];
        if (requestId is String &&
            method is String &&
            params is Map<String, dynamic> &&
            !_completed) {
          _completed = true;
          startedTasks.add({'method': method, 'params': params});
          _stdout.add(
            utf8.encode(
              '${jsonEncode({
                'protocol_version': 5,
                'request_id': requestId,
                'ok': true,
                'result': {
                  'source_id': params['source_id'],
                  'settings_revision': 3,
                  'status': {
                    'checked_at': 1,
                    'cached_at': 1,
                    'entry_count': 0,
                    'code': 'ok',
                  },
                  'revision': 1,
                  'complete': true,
                  'affected_appids': <int>[],
                },
              })}\n',
            ),
          );
          _stdout.close();
          _stderr.close();
          _exitCode.complete(0);
        }
      }
    });
  }

  final List<Map<String, Object?>> startedTasks;
  final _stdout = StreamController<List<int>>();
  final _stderr = StreamController<List<int>>();
  final _stdinController = StreamController<List<int>>();
  final _exitCode = Completer<int>();
  bool _completed = false;

  @override
  Future<int> get exitCode => _exitCode.future;

  @override
  Stream<List<int>> get stdout => _stdout.stream;

  @override
  Stream<List<int>> get stderr => _stderr.stream;

  @override
  late final IOSink stdin = IOSink(_stdinController.sink);

  @override
  int get pid => 1;

  @override
  bool kill([ProcessSignal signal = ProcessSignal.sigterm]) => true;
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
        'source_id': source['id'],
        'name': source['name'],
        'rule': {
          for (final entry in source.entries)
            if (![
              'name',
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
  'official_title_match': false,
  'priority': 100,
  'origin': origin,
  'status': 'not_refreshed',
  'interval_hours': 0,
  'entry_count': 0,
  'complete': false,
  'pages': <String>['https://example.com/'],
  'page_hosts': <String>['example.com'],
  'download_hosts': <String>['dl.example.com'],
  'script': {'api_version': 1, 'sha256': 'a' * 64},
};

Future<void> enterCode(WidgetTester tester, String code) async {
  final field = find.byKey(const ValueKey('script-code'));
  await tester.ensureVisible(field);
  await tester.enterText(field, code);
}

void main() {
  final emptyCore = Platform.environment['PATCHHUB_EMPTY_WINDOWS_CORE'];
  testWidgets('five bundled sources import by pasting only Lua through empty Core', (
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

    for (final entry in (config['sources'] as List).cast<Map>()) {
      final code = File('$resource/source-scripts/${files[entry['adapter']]}')
          .readAsStringSync();
      late Future<Map<String, dynamic>> saved;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () => showDialog<void>(
                  context: context,
                  builder: (_) => ScriptSourceEditor(
                    onSave: (params) async {
                      saved = client
                          .call('import_script_source', params)
                          .then((v) => v as Map<String, dynamic>);
                      await saved;
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
      await enterCode(tester, code);
      late Map<String, dynamic> imported;
      await tester.runAsync(() async {
        await tester.tap(find.text('保存并导入'));
        imported = await saved;
      });
      await tester.pumpAndSettle();
      expect(find.byType(ScriptSourceEditor), findsNothing);
      final assignedId = (imported['rule'] as Map)['id'] as String;
      final record = await tester.runAsync(
        () => client.call('get_script_source', {'source_id': assignedId}),
      ) as Map;
      expect(record['rule']['adapter'], 'lua');
      expect(record['code'], code);
    }
    expect(
      await tester.runAsync(() => client.call('list_sources')),
      hasLength(5),
    );
  }, skip: emptyCore == null);

  testWidgets('copy AI prompt button loads prompt and copies it to clipboard', (
    tester,
  ) async {
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

    const promptText =
        '请查阅 https://raw.githubusercontent.com/Mabinogi-Cichol/patchhub/main/docs/custom-subscription-sources.md';
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: ScriptSourceEditor(
            onLoadPrompt: () async => promptText,
            onSave: (_) async {},
          ),
        ),
      ),
    );
    await tester.tap(find.byKey(const ValueKey('script-copy-prompt')));
    await tester.pumpAndSettle();
    expect(copied, promptText);
    expect(find.textContaining('AI 提示词已复制'), findsOneWidget);
  });

  for (final brightness in Brightness.values) {
    testWidgets(
      'failed import retains large Lua draft without extra form fields in $brightness',
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
        final code = '-- ${'中文' * 18000}\n$sampleScript';
        await enterCode(tester, code);
        await tester.tap(find.text('保存并导入'));
        await tester.pumpAndSettle();
        expect(received!['code'], code);
        expect(received!.containsKey('source_id'), isFalse);
        expect(received!.containsKey('rule'), isFalse);
        expect(find.byType(ScriptSourceEditor), findsOneWidget);
        expect(
          tester
              .widget<TextField>(find.byKey(const ValueKey('script-code')))
              .controller!
              .text,
          code,
        );
        expect(find.textContaining('完整错误'), findsOneWidget);
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
      expect(find.byKey(const ValueKey('script-summary')), findsOneWidget);
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
    'page imports, displays host domains, refreshes projections, reads and locks edited ID',
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
      expect(find.byKey(const ValueKey('script-copy-prompt')), findsOneWidget);
      await enterCode(tester, sampleScript);
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
      expect(client.writes.last.containsKey('source_id'), isFalse);
      expect(
        model.snapshot!.rows.single.patches.map((patch) => patch['release_id']),
        ['signed'],
      );
      expect(model.snapshot!.rows.single.discovery!['candidates'], isEmpty);
      expect(client.startedTasks, hasLength(1));
      expect(client.startedTasks.single['method'], 'update_source');
      expect(
        (client.startedTasks.single['params'] as Map)['source_id'],
        'patches-vendor-example',
      );
      expect(
        (client.startedTasks.single['params'] as Map)['force'],
        isTrue,
      );
      expect(find.textContaining('脚本已导入，正在进行初次订阅更新'), findsOneWidget);
      await tester.pump(const Duration(seconds: 5));
      await tester.pumpAndSettle();
      await tester.tap(find.text('脚本发行商示例'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('调度与地址详情'));
      await tester.tap(find.text('调度与地址详情'));
      await tester.pumpAndSettle();
      expect(
        find.textContaining('允许访问域名：patches.vendor.example'),
        findsOneWidget,
      );
      expect(
        find.textContaining('允许下载域名：downloads.vendor.example'),
        findsOneWidget,
      );
      await tester.ensureVisible(find.text('编辑脚本'));
      await tester.tap(find.text('编辑脚本'));
      await tester.pumpAndSettle();
      expect(client.calls.last, 'get_script_source');
      expect(find.byKey(const ValueKey('script-copy-prompt')), findsNothing);
      expect(find.byKey(const ValueKey('script-summary')), findsOneWidget);
      expect(
        tester
            .widget<TextField>(find.byKey(const ValueKey('script-code')))
            .controller!
            .text,
        sampleScript,
      );
      await enterCode(tester, '$sampleScript\n-- updated');
      client.reject = true;
      await tester.tap(find.text('保存并替换 patches-vendor-example'));
      await tester.pumpAndSettle();
      expect(client.document!['code'], sampleScript);
      expect(find.byType(ScriptSourceEditor), findsOneWidget);
      client.reject = false;
      await tester.tap(find.text('保存并替换 patches-vendor-example'));
      await tester.pumpAndSettle();
      expect(client.writes.last['source_id'], 'patches-vendor-example');
      expect(client.writes.last.containsKey('replace'), isFalse);
      expect(client.startedTasks, hasLength(1));
      expect(find.textContaining('脚本已保存，尚未执行'), findsOneWidget);
      await tester.pump(const Duration(seconds: 5));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('编辑脚本'));
      await tester.tap(find.text('编辑脚本'));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<TextField>(find.byKey(const ValueKey('script-code')))
            .controller!
            .text,
        '$sampleScript\n-- updated',
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
      await enterCode(tester, '$sampleScript\n-- saved');
      await tester.tap(find.text('保存并替换 patches-vendor-example'));
      await tester.pumpAndSettle();
      expect(find.byType(ScriptSourceEditor), findsNothing);
      expect(client.document!['code'], '$sampleScript\n-- saved');
      expect(model.stateRefreshError.toString(), contains('设置已保存'));
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets('fallback script import does not start initial update', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final client = ScriptClient()..role = 'fallback_indexer';
    final model = AppModel(client);
    addTearDown(model.dispose);
    await tester.pumpWidget(
      PatchHubApp(
        home: Scaffold(body: GlobalSourcesPage(model: model)),
      ),
    );
    await tester.tap(find.text('导入脚本来源'));
    await tester.pumpAndSettle();
    await enterCode(tester, sampleScript);
    await tester.tap(find.text('保存并导入'));
    await tester.pumpAndSettle();
    expect(client.startedTasks, isEmpty);
    expect(find.text('脚本已保存，尚未执行。请更新来源或重新匹配游戏。'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
