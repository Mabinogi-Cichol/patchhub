import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:steam_patch_windows/app.dart';
import 'package:steam_patch_windows/host/desktop_services.dart';
import 'package:steam_patch_windows/models/app_model.dart';
import 'package:steam_patch_windows/models/library.dart';
import 'package:steam_patch_windows/pages/patch_management_page.dart';
import 'package:steam_patch_windows/pages/install_dialog.dart';
import 'package:steam_patch_windows/presentation/library_presentation.dart';

import 'support/process_fixture_client.dart';

class InstallFixtureModel extends AppModel {
  InstallFixtureModel(String scenario, {this.failRefresh = false})
    : super(ProcessFixtureClient(scenario)) {
    final cases = jsonDecode(
      File('../../packages/contracts/fixtures/v5/cases.json')
          .readAsStringSync(),
    );
    snapshot = ScanSnapshot(
      (cases['responses'] as List).firstWhere(
            (c) => c['name'] == 'scan-success',
          )['response']['result']
          as Map<String, dynamic>,
    );
  }
  final bool failRefresh;
  int refreshCalls = 0;
  @override
  Future<bool> refresh() async {
    refreshCalls++;
    if (failRefresh) error = '模拟刷新失败';
    return !failRefresh;
  }
}

class FixtureDesktop extends DesktopServices {
  String? file;
  @override
  Future<String?> selectArchive() async => file;
}

Future<void> mountPatch(
  WidgetTester tester,
  InstallFixtureModel model,
  FixtureDesktop desktop,
) async {
  final row = model.snapshot!.rows.single;
  final entries = displayEntries(row);
  await tester.pumpWidget(
    SteamPatchApp(
      home: Scaffold(
        body: PatchManagementPage(
          model: model,
          desktop: desktop,
          row: row,
          entry: entries.single,
          entries: entries,
          patch: entries.single.patch,
          onEntry: (_) {},
          onMirror: (_) {},
          onRecords: () {},
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets(
    'local file cancel does not start installation; selected ZIP uses only release identity',
    (tester) async {
      final model = InstallFixtureModel('chunked');
      final patch = model.snapshot!.rows.single.patches.single;
      patch['actions']['primary']['input'] = 'local_archive';
      final desktop = FixtureDesktop();
      await mountPatch(tester, model, desktop);
      await tester.tap(find.text('选择本地 ZIP'));
      await tester.pumpAndSettle();
      expect(model.tasks.installation, isNull);
      expect(
        tester.widget<FilledButton>(find.byType(FilledButton)).onPressed,
        isNull,
      );
      desktop.file = 'fixture/patch.zip';
      await tester.tap(find.text('选择本地 ZIP'));
      await tester.pumpAndSettle();
      await tester.runAsync(() => tester.tap(find.text('使用本地 ZIP 安装')));
      await tester.pump();
      final task = model.tasks.installation!;
      expect(task.arguments, {
        'instance_id': 'instance',
        'release_id': 'release',
        'operation': 'overwrite',
        'cleanup_policy': 'disabled',
        'archive': 'fixture/patch.zip',
      });
      await tester.runAsync(() => task.done);
      await tester.pumpAndSettle();
      expect(find.text('安装完成'), findsOneWidget);
      await tester.tap(find.text('关闭结果'));
      await tester.pumpAndSettle();
      expect(task.cancelRequested, isFalse);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );

  testWidgets('completed receipt survives a failed state refresh', (
    tester,
  ) async {
    final model = InstallFixtureModel('chunked', failRefresh: true);
    await mountPatch(tester, model, FixtureDesktop());
    await tester.runAsync(() => tester.tap(find.text('安装补丁')));
    await tester.pump();
    final task = model.tasks.installation!;
    await tester.runAsync(() => task.done);
    await tester.pumpAndSettle();
    expect(find.text('安装完成'), findsOneWidget);
    expect(find.textContaining('安装已完成，状态刷新失败'), findsOneWidget);
    expect(task.result['disposition'], 'completed');
    await tester.tap(find.text('关闭结果'));
    await tester.pumpAndSettle();
    await tester.pumpWidget(const SizedBox.shrink());
    model.dispose();
  });

  testWidgets(
    'Escape requests cancellation and result close never cancels again',
    (tester) async {
      final model = InstallFixtureModel('cancel_wait');
      await mountPatch(tester, model, FixtureDesktop());
      await tester.runAsync(() => tester.tap(find.text('安装补丁')));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      final task = model.tasks.installation!;
      await tester.runAsync(
        () => tester.sendKeyEvent(LogicalKeyboardKey.escape),
      );
      await tester.pump();
      expect(task.cancelRequested, isTrue);
      await tester.runAsync(() => task.done);
      await tester.pumpAndSettle();
      expect(find.text('已取消安装'), findsOneWidget);
      await tester.tap(find.text('关闭结果'));
      await tester.pumpAndSettle();
      expect(task.diagnostics, contains('cancel_count=1'));
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );
  testWidgets('closing skips refresh without claiming a refresh failure', (
    tester,
  ) async {
    final model = InstallFixtureModel('cancel_wait', failRefresh: true);
    await mountPatch(tester, model, FixtureDesktop());
    await tester.runAsync(() => tester.tap(find.text('安装补丁')));
    await tester.pump();
    await tester.runAsync(() => model.tasks.shutdown());
    await tester.pumpAndSettle();
    expect(model.refreshCalls, 0);
    expect(find.textContaining('应用正在关闭'), findsOneWidget);
    expect(find.textContaining('状态刷新失败'), findsNothing);
    await tester.tap(find.text('关闭结果'));
    await tester.pumpAndSettle();
    await tester.pumpWidget(const SizedBox.shrink());
    model.dispose();
  });
  testWidgets('download dialog shows throughput alongside byte progress', (
    tester,
  ) async {
    final model = InstallFixtureModel('cancel_wait');
    final task = (await tester.runAsync(() async {
      final task = model.tasks.startInstall({});
      await task.events.first;
      return task;
    }))!;
    task.progress = {
      ...task.progress!,
      'stage': 'downloading',
      'bytes_done': 4000000,
      'total_bytes': 8000000,
    };
    await tester.pumpWidget(
      SteamPatchApp(
        home: Scaffold(
          body: InstallDialog(
            model: model,
            task: task,
            game: '测试游戏',
            patch: '测试补丁',
          ),
        ),
      ),
    );
    expect(find.text('下载速度：计算中…'), findsOneWidget);
    task.downloadRate.update(
      'downloading',
      1000000,
      const Duration(seconds: 1),
    );
    task.downloadRate.update(
      'downloading',
      4000000,
      const Duration(seconds: 3),
    );
    // Rebuild with the next accepted progress sample, without a real download.
    await tester.pumpWidget(
      SteamPatchApp(
        home: Scaffold(
          body: InstallDialog(
            model: model,
            task: task,
            game: '测试游戏',
            patch: '测试补丁',
          ),
        ),
      ),
    );
    expect(find.text('下载速度：1.5 MB/s'), findsOneWidget);
    expect(find.text('4.0 MB / 8.0 MB'), findsOneWidget);
    await tester.runAsync(() async {
      await task.cancel();
      await task.done;
    });
    await tester.pumpAndSettle();
    expect(find.textContaining('下载速度：'), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
    model.dispose();
  });
}
