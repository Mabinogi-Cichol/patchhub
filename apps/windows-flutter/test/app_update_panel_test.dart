import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/app_lifecycle.dart';
import 'package:patchhub_windows/host/app_update.dart';
import 'package:patchhub_windows/pages/app_update_panel.dart';
import 'package:patchhub_windows/pages/settings_page.dart';

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
    result = {'state': 'checking'};
    pending = Completer<void>();
    notifyListeners();
    await pending!.future;
  }

  void complete(String state, {String error = '无法连接更新服务'}) {
    result = {
      'state': state,
      if (state == 'available' || state == 'unavailable') 'tag': 'v0.4.0',
      'notes': '版本说明\n' * 50,
      'error': error,
    };
    pending?.complete();
    pending = null;
    notifyListeners();
  }

  void progress(int downloaded, int length) {
    bytes = downloaded;
    total = length;
    result = {...result, 'state': 'downloading'};
    notifyListeners();
  }

  @override
  Future<void> skip() async {
    data['skipped_tag'] = result['tag'];
    notifyListeners();
  }

  @override
  void cancelDownload() {
    result = {...result, 'state': 'failed', 'error': '已取消下载'};
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
            child: const SizedBox.expand(
              key: ValueKey('main-content'),
              child: AboutPage(),
            ),
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
      expect(find.text('正在检查更新…'), findsOneWidget);
      expect(
        tester.getRect(find.byKey(const ValueKey('main-content'))),
        before,
      );
      update.complete(state);
      await tester.pumpAndSettle();
      expect(find.byType(AlertDialog), findsNothing);
      expect(tester.takeException(), isNull);
      expect(
        find.textContaining('发现新版本'),
        state == 'available' || state == 'unavailable'
            ? findsOneWidget
            : findsNothing,
      );
      expect(
        tester.getRect(find.byKey(const ValueKey('main-content'))),
        state == 'available' || state == 'unavailable' ? isNot(before) : before,
      );
      await tester.pumpWidget(const SizedBox.shrink());
      update.dispose();
    });
  }

  testWidgets(
    'cancel, failed download, ready and explicit skip preserve reachable actions',
    (tester) async {
      final update = PanelUpdate()
        ..result = {'state': 'available', 'tag': 'v0.4.0', 'notes': 'notes'};
      await mount(tester, update);
      await tester.tap(find.text('查看更新').last);
      await tester.pumpAndSettle();
      update.progress(2048, 4096);
      await tester.pumpAndSettle();
      await tester.tap(find.text('取消下载'));
      await tester.pumpAndSettle();
      expect(
        find.descendant(
          of: find.byType(AlertDialog),
          matching: find.textContaining('已取消下载'),
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('关闭'));
      await tester.pumpAndSettle();
      expect(find.text('查看详情'), findsOneWidget);
      await tester.tap(find.text('查看详情'));
      await tester.pumpAndSettle();
      expect(find.text('重新检查'), findsOneWidget);
      update.complete('available');
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('跳过此版本'));
      await tester.tap(find.text('跳过此版本'));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('app-update-banner')), findsNothing);
      expect(find.textContaining('（已跳过）'), findsOneWidget);
      update.result = {...update.result, 'tag': 'v0.5.0'};
      update.progress(1, 2);
      await tester.pumpAndSettle();
      expect(find.text('查看进度'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
      update.dispose();
    },
  );

  testWidgets('download progress tracks bytes and supports unknown length', (
    tester,
  ) async {
    final update = PanelUpdate();
    await mount(tester, update);
    await tester.tap(find.text('检查更新'));
    await tester.pumpAndSettle();
    update.complete('available');
    await tester.pumpAndSettle();
    await tester.tap(find.text('查看更新').last);
    await tester.pumpAndSettle();
    update.progress(5 * 1024 * 1024, 20 * 1024 * 1024);
    await tester.pump();
    expect(
      tester
          .widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator))
          .value,
      0.25,
    );
    expect(find.text('25% · 5.0 MB / 20.0 MB'), findsOneWidget);
    update.progress(10 * 1024 * 1024, 20 * 1024 * 1024);
    await tester.pump();
    expect(
      tester
          .widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator))
          .value,
      0.5,
    );
    update.progress(1024, -1);
    await tester.pump();
    expect(
      tester
          .widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator))
          .value,
      isNull,
    );
    expect(find.text('已下载 1.0 KB'), findsOneWidget);
    expect(find.text('取消下载'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.tap(find.text('关闭'));
    await tester.pumpAndSettle();
    await tester.pumpWidget(const SizedBox.shrink());
    update.dispose();
  });

  testWidgets(
    'previous installation failure is separate from current TLS failure and can retry',
    (tester) async {
      final update = PanelUpdate();
      update.outcome = {
        'state': 'failed',
        'error': 'Application still running in target directory',
      };
      await mount(tester, update);
      await tester.tap(find.text('检查更新'));
      await tester.pumpAndSettle();
      update.complete(
        'failed',
        error: 'HandshakeException: Connection terminated during handshake',
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('查看更新').first);
      await tester.pumpAndSettle();
      expect(find.textContaining('上次安装结果：更新目录中还有 PatchHub 进程'), findsOneWidget);
      expect(find.textContaining('本次更新操作失败：连接更新服务时 TLS 握手中断'), findsOneWidget);
      await tester.tap(find.text('重新检查'));
      await tester.pumpAndSettle();
      expect(update.checks, 2);
      expect(find.text('正在检查 GitHub Release…'), findsOneWidget);
      update.complete('up_to_date');
      await tester.pumpAndSettle();
      await tester.tap(find.text('关闭'));
      await tester.pumpAndSettle();
      await tester.pumpWidget(const SizedBox.shrink());
      update.dispose();
    },
  );

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
      await tester.tap(find.text('查看更新').last);
      await tester.pumpAndSettle();
      expect(update.checks, 1);
      await tester.ensureVisible(find.text('下载更新'));
      await tester.tap(find.text('下载更新'));
      await tester.pumpAndSettle();
      expect(find.text('重启并更新'), findsOneWidget);
      await tester.tap(find.text('关闭'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('重启更新'));
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
