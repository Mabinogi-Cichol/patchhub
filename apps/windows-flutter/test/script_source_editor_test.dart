import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/models/library.dart';
import 'package:patchhub_windows/pages/script_source_editor.dart';
import 'package:patchhub_windows/pages/script_source_examples.dart';
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

void main() {
  test('UI examples match the author files', () {
    const root =
        '../../docs/features/script-source-adapters/examples/publisher';
    expect(
      jsonDecode(scriptRuleExample),
      jsonDecode(File('$root/source.json').readAsStringSync()),
    );
    expect(
      scriptCodeExample,
      File('$root/adapter.lua').readAsStringSync().replaceAll('\r\n', '\n'),
    );
  });

  for (final brightness in Brightness.values) {
    testWidgets(
      'failed import retains large draft and example only copies in $brightness',
      (tester) async {
        tester.view.physicalSize = const Size(580, 600);
        tester.view.devicePixelRatio = 1;
        tester.platformDispatcher.textScaleFactorTestValue = 1.5;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
        String? copied;
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          (call) async {
            if (call.method == 'Clipboard.setData') {
              copied = (call.arguments as Map)['text'] as String;
            }
            return null;
          },
        );
        addTearDown(
          () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            SystemChannels.platform,
            null,
          ),
        );
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
        await tester.enterText(
          find.byKey(const ValueKey('script-rule')),
          scriptRuleExample,
        );
        await tester.enterText(find.byKey(const ValueKey('script-code')), code);
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
        FocusManager.instance.primaryFocus?.unfocus();
        await tester.pumpAndSettle();
        await tester.ensureVisible(find.text('查看脚本示例'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('查看脚本示例'));
        await tester.pumpAndSettle();
        await tester.ensureVisible(find.text('复制规则示例'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('复制规则示例'));
        await tester.pumpAndSettle();
        expect(copied, scriptRuleExample);
        expect(
          tester
              .widget<TextField>(find.byKey(const ValueKey('script-rule')))
              .controller!
              .text,
          scriptRuleExample,
        );
        expect(
          tester
              .widget<TextField>(find.byKey(const ValueKey('script-code')))
              .controller!
              .text,
          code,
        );
        expect(tester.takeException(), isNull);
      },
    );
  }

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
      await tester.enterText(
        find.byKey(const ValueKey('script-rule')),
        scriptRuleExample,
      );
      await tester.enterText(
        find.byKey(const ValueKey('script-code')),
        'return {}',
      );
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
      await tester.enterText(
        find.byKey(const ValueKey('script-rule')),
        scriptRuleExample.replaceFirst('my-script-publisher', 'different-id'),
      );
      await tester.tap(find.text('保存并替换 my-script-publisher'));
      await tester.pumpAndSettle();
      expect(client.writes.length, 1);
      expect(find.textContaining('替换时来源 ID 必须为'), findsOneWidget);
      await tester.enterText(
        find.byKey(const ValueKey('script-rule')),
        scriptRuleExample,
      );
      await tester.enterText(
        find.byKey(const ValueKey('script-code')),
        'return { updated = true }',
      );
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
