import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:steam_patch_windows/app.dart';
import 'package:steam_patch_windows/host/core_client.dart';
import 'package:steam_patch_windows/models/app_model.dart';
import 'package:steam_patch_windows/pages/library_page.dart';

import 'library_page_test.dart' show FixtureClient;

class LongFailureClient extends FixtureClient {
  final detail =
      '完整错误开头${List.filled(80, '游戏库路径无法读取：F:/SteamLibrary/steamapps/common/长中文路径。').join()}完整错误末尾';
  @override
  Future<Object?> call(
    String method, [
    Map<String, Object?> args = const {},
  ]) async {
    if (this.fail) throw CoreError('scan_failed', detail);
    return super.call(method, args);
  }
}

void main() {
  testWidgets(
    'long read errors keep the library usable and full detail scrollable',
    (tester) async {
      tester.view.physicalSize = const Size(850, 560);
      tester.view.devicePixelRatio = 1;
      tester.platformDispatcher.textScaleFactorTestValue = 1.5;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
      final client = LongFailureClient();
      final model = AppModel(client);
      await tester.pumpWidget(SteamPatchApp(home: LibraryPage(model: model)));
      await tester.pumpAndSettle();
      await tester.tap(find.text('查看全部游戏'));
      await tester.pumpAndSettle();
      client.fail = true;
      await tester.tap(find.text('重新扫描游戏库'));
      await tester.pumpAndSettle();
      expect(find.text('游戏库读取失败'), findsOneWidget);
      expect(find.text('未匹配游戏'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.tap(find.text('查看原因'));
      await tester.pumpAndSettle();
      final detail = tester
          .widget<SelectableText>(find.byType(SelectableText).last)
          .data!;
      expect(detail, 'scan_failed: ${client.detail}');
      expect(find.byType(SingleChildScrollView), findsWidgets);
      expect(tester.takeException(), isNull);
      await tester.tap(find.text('关闭'));
      await tester.pumpAndSettle();
      expect(find.text('未匹配游戏'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );
}
