import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/app_lifecycle.dart';
import 'package:patchhub_windows/host/app_update.dart';
import 'package:patchhub_windows/pages/app_update_panel.dart';

class PanelUpdate extends AppUpdate {
  PanelUpdate()
    : super(
        directory: Directory('unused'),
        current: '0.3.0',
        stateFile: File('unused'),
      );
  Completer<void>? pending;
  int checks = 0;
  @override
  Future<void> initialize() async {}
  @override
  Future<void> check({bool manual = false}) async {
    checks++;
    lastCheckWasManual = manual;
    result = {'state': 'checking'};
    pending = Completer<void>();
    notifyListeners();
    await pending!.future;
  }

  void complete(String state) {
    result = {
      'state': state,
      'tag': 'v0.4.0',
      'notes': '版本说明\n' * 50,
      'error': '无法连接更新服务',
    };
    pending?.complete();
    pending = null;
    notifyListeners();
  }

  @override
  Future<void> download() async {
    result = {...result, 'state': 'ready'};
    notifyListeners();
  }
}

void main() {
  Future<void> mount(WidgetTester tester, PanelUpdate update) async {
    await tester.pumpWidget(
      PatchHubApp(
        home: Scaffold(
          body: AppUpdatePanel(
            update: update,
            lifecycle: AppLifecycle(),
            child: const SizedBox.expand(key: ValueKey('main-content')),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  for (final state in ['up_to_date', 'failed', 'unavailable', 'available']) {
    testWidgets('manual $state result leaves main layout unchanged', (
      tester,
    ) async {
      final update = PanelUpdate();
      await mount(tester, update);
      final before = tester.getRect(find.byKey(const ValueKey('main-content')));
      await tester.tap(find.text('检查更新'));
      await tester.pumpAndSettle();
      expect(find.text('正在检查 GitHub Release…'), findsOneWidget);
      expect(
        tester.getRect(find.byKey(const ValueKey('main-content'))),
        before,
      );
      update.complete(state);
      await tester.pumpAndSettle();
      expect(find.byType(AlertDialog), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.tap(find.text('关闭'));
      await tester.pumpAndSettle();
      expect(find.textContaining('发现新版本'), findsNothing);
      expect(
        tester.getRect(find.byKey(const ValueKey('main-content'))),
        before,
      );
      await tester.pumpWidget(const SizedBox.shrink());
      update.dispose();
    });
  }

  testWidgets(
    'automatic notification opens details and preserves ready download',
    (tester) async {
      final update = PanelUpdate();
      await mount(tester, update);
      final check = update.check();
      update.complete('available');
      await check;
      await tester.pumpAndSettle();
      expect(find.text('发现新版本 v0.4.0'), findsOneWidget);
      await tester.tap(find.text('查看更新'));
      await tester.pumpAndSettle();
      expect(update.checks, 1);
      await tester.ensureVisible(find.text('下载更新'));
      await tester.tap(find.text('下载更新'));
      await tester.pumpAndSettle();
      expect(find.text('重启并更新'), findsOneWidget);
      await tester.tap(find.text('关闭'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('检查更新'));
      await tester.pumpAndSettle();
      expect(find.text('重启并更新'), findsOneWidget);
      expect(update.checks, 1);
      expect(tester.takeException(), isNull);
      await tester.tap(find.text('关闭'));
      await tester.pumpAndSettle();
      await tester.pumpWidget(const SizedBox.shrink());
      update.dispose();
    },
  );
}
