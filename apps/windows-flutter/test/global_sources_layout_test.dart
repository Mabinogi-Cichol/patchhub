import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/pages/source_pages.dart';

import 'library_page_test.dart' show FixtureClient;

class NameClient extends FixtureClient {
  String name = 'Shiravune';
  bool rejectName = false;
  final writes = <Map<String, Object?>>[];

  Map<String, dynamic> get source => {
    'id': 'patchmoe',
    'name': name,
    'enabled': true,
    'role': 'publisher',
    'status': 'not_refreshed',
    'interval_hours': 0,
    'entry_count': 0,
    'complete': false,
    'pages': <String>[],
  };

  @override
  Future<Object?> call(
    String method, [
    Map<String, Object?> args = const {},
  ]) async {
    if (method == 'set_source_name') {
      writes.add(Map.of(args));
      if (rejectName) throw StateError('保存被拒绝');
      name = (args['name'] as String).trim();
      return {'settings_revision': 2, 'source_id': 'patchmoe', 'name': name};
    }
    if (method == 'list_sources') return [source];
    return super.call(method, args);
  }
}

void main() {
  for (final brightness in Brightness.values) {
    testWidgets(
      'rule example copies JSON without replacing the draft in $brightness',
      (tester) async {
        tester.view.physicalSize = const Size(580, 560);
        tester.view.devicePixelRatio = 1;
        tester.platformDispatcher.textScaleFactorTestValue = 1.5;
        tester.platformDispatcher.platformBrightnessTestValue = brightness;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
        addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
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
        final model = AppModel(FixtureClient());
        await tester.pumpWidget(
          PatchHubApp(
            home: Scaffold(body: GlobalSourcesPage(model: model)),
          ),
        );
        await tester.ensureVisible(find.text('高级：添加来源规则'));
        await tester.tap(find.text('高级：添加来源规则'));
        await tester.pumpAndSettle();
        final ruleInput = find.byKey(
          const PageStorageKey('global-sources:rule-input'),
        );
        await tester.enterText(ruleInput, '保留未完成规则');
        FocusManager.instance.primaryFocus?.unfocus();
        await tester.ensureVisible(find.text('示例'));
        await tester.tap(find.text('示例'));
        await tester.pumpAndSettle();
        expect(find.byType(AlertDialog), findsOneWidget);
        expect(find.text(sourceRuleExampleHelp), findsOneWidget);
        await tester.tap(find.text('复制示例'));
        await tester.pumpAndSettle();
        final example = jsonDecode(copied!) as Map<String, dynamic>;
        expect(example['adapter'], 'table_html');
        expect(example['columns'], {
          'title': 0,
          'appid': 1,
          'download': 2,
          'date': 3,
          'language': 4,
        });
        expect(tester.widget<TextField>(ruleInput).controller!.text, '保留未完成规则');
        expect(tester.takeException(), isNull);
        await tester.pumpWidget(const SizedBox.shrink());
        model.dispose();
      },
    );
  }

  testWidgets(
    'source name saves, rejects blank input and preserves failed drafts',
    (tester) async {
      final client = NameClient();
      final model = AppModel(client)..sources = [client.source];
      await tester.pumpWidget(
        PatchHubApp(
          home: Scaffold(body: GlobalSourcesPage(model: model)),
        ),
      );
      await tester.tap(find.text('Shiravune'));
      await tester.pumpAndSettle();
      final input = find.widgetWithText(TextFormField, '来源名称');
      final save = find.widgetWithText(TextButton, '保存名称');
      await tester.ensureVisible(input);
      await tester.enterText(input, '  ');
      await tester.pump();
      expect(find.text('请输入来源名称'), findsOneWidget);
      expect(tester.widget<TextButton>(save).onPressed, isNull);
      await tester.enterText(input, '  我的发行商  ');
      await tester.pump();
      await tester.ensureVisible(save);
      await tester.tap(save);
      await tester.pumpAndSettle();
      expect(client.writes.single, {
        'source_id': 'patchmoe',
        'name': '  我的发行商  ',
      });
      expect(find.text('我的发行商'), findsWidgets);
      expect(tester.widget<TextButton>(save).onPressed, isNull);
      client.rejectName = true;
      await tester.enterText(input, '保留草稿');
      await tester.pump();
      await tester.tap(save);
      await tester.pumpAndSettle();
      expect(find.textContaining('保存被拒绝'), findsOneWidget);
      expect(find.text('保留草稿'), findsOneWidget);
      expect(model.sources.single['name'], '我的发行商');
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );

  for (final brightness in Brightness.values) {
    for (final scale in [1.0, 1.5]) {
      testWidgets(
        'source expansions at $scale in $brightness keep separate state after scrolling',
        (tester) async {
          tester.platformDispatcher.textScaleFactorTestValue = scale;
          tester.platformDispatcher.platformBrightnessTestValue = brightness;
          addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
          addTearDown(
            tester.platformDispatcher.clearPlatformBrightnessTestValue,
          );
          tester.view.physicalSize = const Size(580, 560);
          tester.view.devicePixelRatio = 1;
          addTearDown(tester.view.resetPhysicalSize);
          addTearDown(tester.view.resetDevicePixelRatio);
          final model = AppModel(FixtureClient())
            ..sources = [
              for (var i = 0; i < 4; i++)
                {
                  'id': 'source-$i',
                  'name': '来源 $i',
                  'enabled': true,
                  'role': i == 3 ? 'fallback_indexer' : 'publisher',
                  'status': 'source_refreshed',
                  'interval_hours': 24,
                  'entry_count': 8,
                  'complete': true,
                  'pages': <String>[],
                },
            ];
          await tester.pumpWidget(
            PatchHubApp(
              home: Scaffold(body: GlobalSourcesPage(model: model)),
            ),
          );
          await tester.pumpAndSettle();
          final scrollable = find.byType(Scrollable).first;
          await tester.scrollUntilVisible(
            find.text('高级：添加来源规则'),
            200,
            scrollable: scrollable,
          );
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          await tester.tap(find.text('高级：添加来源规则'));
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          tester.state<ScrollableState>(scrollable).position.jumpTo(0);
          await tester.pumpAndSettle();
          await tester.tap(find.text('来源 0').first);
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          expect(find.textContaining('最近成功：').hitTestable(), findsNothing);
          final title = find.text('自动更新');
          await tester.ensureVisible(title);
          await tester.pumpAndSettle();
          final interval = find.byType(TextFormField).first;
          expect(
            tester.getRect(title).bottom,
            lessThan(tester.getRect(interval).top),
          );
          expect(tester.getSize(interval).width, lessThanOrEqualTo(200));
          expect(find.byTooltip('启用此来源'), findsWidgets);
          await tester.enterText(interval, '48');
          FocusManager.instance.primaryFocus?.unfocus();
          await tester.pumpAndSettle();
          await tester.ensureVisible(find.text('调度与地址详情').first);
          await tester.tap(find.text('调度与地址详情').first);
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          expect(find.textContaining('最近成功：'), findsOneWidget);
          tester.state<ScrollableState>(scrollable).position.jumpTo(0);
          await tester.pumpAndSettle();
          await tester.tap(find.text('来源 0').first);
          await tester.pumpAndSettle();
          tester.state<ScrollableState>(scrollable).position.jumpTo(0);
          await tester.pumpAndSettle();
          await tester.tap(find.text('来源 0').first);
          await tester.pumpAndSettle();
          expect(find.textContaining('最近成功：'), findsOneWidget);
          expect(find.widgetWithText(TextFormField, '48'), findsOneWidget);
          expect(tester.takeException(), isNull);
          await tester.pumpWidget(const SizedBox.shrink());
          model.dispose();
        },
      );
    }
  }
}
