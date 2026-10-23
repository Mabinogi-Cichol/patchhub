import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:steam_patch_windows/app.dart';
import 'package:steam_patch_windows/models/app_model.dart';
import 'package:steam_patch_windows/models/library.dart';
import 'package:steam_patch_windows/pages/library_page.dart';

import 'library_page_test.dart' show FixtureClient;

class ModernFixtureModel extends AppModel {
  ModernFixtureModel() : super(FixtureClient()) {
    final cases = jsonDecode(
      File('../../packages/contracts/fixtures/v5/cases.json')
          .readAsStringSync(),
    );
    final data = (cases['responses'] as List).firstWhere(
      (c) => c['name'] == 'scan-success',
    )['response']['result'];
    data['installs'][0]['game']['name'] = '用于检查长中文标题换行与补丁安装操作的测试游戏';
    snapshot = ScanSnapshot(data);
  }
  @override
  Future<void> initialize() async {}
}

void main() {
  for (final size in [
    const Size(1440, 900),
    const Size(1180, 760),
    const Size(850, 560),
  ]) {
    for (final scale in [1.0, 1.25, 1.5]) {
      for (final brightness in Brightness.values) {
        testWidgets(
          '$size, scale $scale, $brightness: patch actions and tabs survive resizing',
          (tester) async {
            tester.view.physicalSize = size;
            tester.view.devicePixelRatio = 1;
            tester.platformDispatcher.textScaleFactorTestValue = scale;
            tester.platformDispatcher.platformBrightnessTestValue = brightness;
            addTearDown(tester.view.resetPhysicalSize);
            addTearDown(tester.view.resetDevicePixelRatio);
            addTearDown(
              tester.platformDispatcher.clearTextScaleFactorTestValue,
            );
            addTearDown(
              tester.platformDispatcher.clearPlatformBrightnessTestValue,
            );
            final model = ModernFixtureModel();
            await tester.pumpWidget(
              SteamPatchApp(home: LibraryPage(model: model)),
            );
            await tester.pumpAndSettle();
            await tester.tap(find.text(model.snapshot!.rows.single.name));
            await tester.pumpAndSettle();
            await tester.tap(find.text('补丁管理'));
            await tester.pumpAndSettle();
            expect(find.text('打开游戏目录').hitTestable(), findsOneWidget);
            expect(find.text('安装补丁').hitTestable(), findsOneWidget);
            final footer = tester.getRect(
              find.byKey(const ValueKey('patch-floating-actions')),
            );
            final page = tester.getRect(find.byType(LibraryPage));
            expect(page.right - footer.right, greaterThanOrEqualTo(16));
            expect(page.bottom - footer.bottom, greaterThanOrEqualTo(16));
            final status = find.byKey(const ValueKey('patch-status-panel'));
            await tester.ensureVisible(find.text('补丁状态'));
            await tester.pumpAndSettle();
            expect(
              find.descendant(of: status, matching: find.text('可安装补丁')),
              findsOneWidget,
            );
            expect(
              find.descendant(of: status, matching: find.text('游戏信息')),
              findsOneWidget,
            );
            expect(tester.takeException(), isNull);
            // Scroll the information without moving the persistent action bar.
            final actionPosition = tester.getTopLeft(find.text('安装补丁'));
            final content = find.byType(ListView).last;
            await tester.drag(content, const Offset(0, -300));
            await tester.pumpAndSettle();
            expect(tester.getTopLeft(find.text('安装补丁')), actionPosition);
            // Shift+Tab reaches the preceding action and Tab returns to install.
            final installButton = tester.widget<FilledButton>(
              find.widgetWithText(FilledButton, '安装补丁'),
            );
            installButton.focusNode!.requestFocus();
            await tester.pump();
            await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
            await tester.sendKeyEvent(LogicalKeyboardKey.tab);
            await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
            await tester.pump();
            expect(installButton.focusNode!.hasFocus, isFalse);
            await tester.sendKeyEvent(LogicalKeyboardKey.tab);
            await tester.pump();
            expect(installButton.focusNode!.hasFocus, isTrue);
            expect(tester.takeException(), isNull);
            await tester.pumpWidget(const SizedBox.shrink());
            model.dispose();
          },
        );
      }
    }
  }
}
