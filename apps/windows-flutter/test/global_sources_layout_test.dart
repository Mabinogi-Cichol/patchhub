import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/pages/source_pages.dart';

import 'library_page_test.dart' show FixtureClient;

void main() {
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
          await tester.tap(find.text('来源 0'));
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
          await tester.tap(find.text('来源 0'));
          await tester.pumpAndSettle();
          tester.state<ScrollableState>(scrollable).position.jumpTo(0);
          await tester.pumpAndSettle();
          await tester.tap(find.text('来源 0'));
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
