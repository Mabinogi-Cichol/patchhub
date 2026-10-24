import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:steam_patch_windows/app.dart';
import 'package:steam_patch_windows/pages/library_page.dart';

import 'modern_layout_test.dart' show ModernFixtureModel;

void main() {
  testWidgets('game and technical details expand without layout errors', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1180, 760);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final model = ModernFixtureModel();
    await tester.pumpWidget(SteamPatchApp(home: LibraryPage(model: model)));
    await tester.pumpAndSettle();
    await tester.tap(find.text(model.snapshot!.rows.single.name));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('游戏信息'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('游戏信息'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.textContaining('安装路径：'), findsOneWidget);
    await tester.tap(find.text('补丁管理'));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('文件与技术详情'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('文件与技术详情'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.textContaining('发行标识：'), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
    model.dispose();
  });
}
