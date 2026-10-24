import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/pages/library_page.dart';

import 'library_page_test.dart' show FixtureClient;

void main() {
  for (final scale in [1.25, 1.5]) {
    for (final brightness in Brightness.values) {
      testWidgets('850x560 at text scale $scale in $brightness stays usable', (
        tester,
      ) async {
        tester.view.physicalSize = const Size(850, 560);
        tester.view.devicePixelRatio = 1;
        tester.platformDispatcher.textScaleFactorTestValue = scale;
        tester.platformDispatcher.platformBrightnessTestValue = brightness;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
        addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
        final model = AppModel(FixtureClient());
        await tester.pumpWidget(PatchHubApp(home: LibraryPage(model: model)));
        await tester.pumpAndSettle();
        await tester.tap(find.text('查看全部游戏'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('未匹配游戏'));
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
        await tester.tap(find.text('全局来源'));
        await tester.pumpAndSettle();
        expect(find.text('返回游戏'), findsOneWidget);
        expect(tester.takeException(), isNull);
        await tester.pumpWidget(const SizedBox.shrink());
        model.dispose();
      });
    }
  }
}
