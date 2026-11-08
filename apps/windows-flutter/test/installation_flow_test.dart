import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/desktop_services.dart';
import 'package:patchhub_windows/models/app_model.dart';
import 'package:patchhub_windows/models/library.dart';
import 'package:patchhub_windows/pages/patch_management_page.dart';
import 'package:patchhub_windows/pages/install_dialog.dart';
import 'package:patchhub_windows/presentation/library_presentation.dart';
import 'package:window_manager/window_manager.dart';

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

class FixtureDesktop extends DesktopServices {}

Future<void> mountPatch(
  WidgetTester tester,
  InstallFixtureModel model,
  FixtureDesktop desktop,
) async {
  final row = model.snapshot!.rows.single;
  final entries = displayEntries(row);
  await tester.pumpWidget(
    PatchHubApp(
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
  testWidgets('modal install keeps window dragging and controls reachable', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(850, 560);
    tester.view.devicePixelRatio = 1;
    tester.platformDispatcher.textScaleFactorTestValue = 1.5;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    const channel = MethodChannel('window_manager');
    final calls = <String>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
      call,
    ) async {
      calls.add(call.method);
      if (call.method == 'isMaximized' || call.method == 'isFullScreen') {
        return false;
      }
      return null;
    });
    addTearDown(() {
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      );
    });
    final model = InstallFixtureModel('cancel_wait');
    var closeRequests = 0;
    await tester.pumpWidget(
      PatchHubApp(
        onClose: () => closeRequests++,
        home: Builder(
          builder: (context) => Scaffold(
            body: TextButton(
              onPressed: () => showInstallDialog(
                context,
                model: model,
                task: model.tasks.startInstall({}),
                game: '测试游戏',
                patch: '测试补丁',
              ),
              child: const Text('打开安装'),
            ),
          ),
        ),
      ),
    );
    await tester.runAsync(() => tester.tap(find.text('打开安装')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    final task = model.tasks.installation!;
    final titleBar = find.byType(DragToMoveArea);
    expect(tester.getCenter(titleBar).dy, 26);
    await tester.dragFrom(const Offset(200, 26), const Offset(80, 30));
    await tester.pump();
    expect(calls, contains('startDragging'));
    await tester.tap(find.byTooltip('最小化'));
    await tester.tap(find.byTooltip('最大化或还原'));
    await tester.pump();
    expect(calls, containsAll(['minimize', 'isMaximized', 'maximize']));
    await tester.tap(find.byTooltip('关闭'));
    expect(closeRequests, 1);
    // Clicking outside the panel must not dismiss or cancel the installation.
    await tester.tapAt(const Offset(5, 80));
    await tester.pump();
    expect(find.byType(InstallDialog), findsOneWidget);
    expect(task.cancelRequested, isFalse);
    await tester.runAsync(() async {
      await task.cancel();
      await task.done;
    });
    await tester.pumpAndSettle();
    await tester.dragFrom(const Offset(200, 26), const Offset(80, 30));
    await tester.pump();
    expect(calls.where((call) => call == 'startDragging'), hasLength(2));
    await tester.tap(find.byTooltip('关闭结果'));
    await tester.pumpAndSettle();
    expect(find.byType(DragToMoveArea), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
    model.dispose();
  });

  testWidgets(
    'selecting another candidate installs without extra confirmation',
    (tester) async {
      final model = InstallFixtureModel('chunked');
      final row = model.snapshot!.rows.single;
      final original = {...row.patches.single};
      row.patches
        ..clear()
        ..addAll([
          {...original, 'name': '推荐补丁', 'candidate_id': 'preferred'},
          {
            ...original,
            'name': '其他补丁',
            'candidate_id': 'alternative',
            'selection_id': 'selected-alternative',
            'compatibility': 'manual_selection_required',
          },
        ]);
      final entries = displayEntries(row);
      var selected = entries.first;
      await tester.pumpWidget(
        PatchHubApp(
          home: StatefulBuilder(
            builder: (context, update) => Scaffold(
              body: PatchManagementPage(
                key: ValueKey(selected.selectionKey),
                model: model,
                desktop: FixtureDesktop(),
                row: row,
                entry: selected,
                entries: entries,
                patch: selected.patch,
                onEntry: (key) => update(() {
                  selected = entries.singleWhere((e) => e.selectionKey == key);
                }),
                onMirror: (_) {},
                onRecords: () {},
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('desktop-select-field')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('其他补丁').last);
      await tester.pumpAndSettle();
      expect(find.byType(CheckboxListTile), findsNothing);
      expect(find.text('可安装补丁'), findsOneWidget);
      await tester.runAsync(() => tester.tap(find.text('安装补丁')));
      await tester.pump();
      final task = model.tasks.installation!;
      expect(task.arguments['selection_id'], 'selected-alternative');
      expect(task.arguments['cleanup_policy'], 'after_commit');
      expect(task.arguments.containsKey('release_id'), isFalse);
      await tester.runAsync(() => task.done);
      await tester.pumpAndSettle();
      expect(find.text('安装完成'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );

  testWidgets(
    'cleanup stays busy until successful receipt with cleanup failure',
    (tester) async {
      final model = InstallFixtureModel('cleanup_wait');
      await mountPatch(tester, model, FixtureDesktop());
      await tester.runAsync(() => tester.tap(find.text('安装补丁')));
      final task = model.tasks.installation!;
      await tester.runAsync(() async {
        while (task.running && task.progress?['stage'] != 'cleanup') {
          await Future<void>.delayed(const Duration(milliseconds: 10));
        }
      });
      await tester.pump();
      expect(task.error, isNull, reason: task.diagnostics);
      expect(task.running, isTrue);
      expect(find.text('正在清理'), findsWidgets);
      expect(find.text('安装完成'), findsNothing);
      // A late cancel releases the fixture; committed success keeps precedence.
      await tester.runAsync(() async {
        await task.cancel();
        await task.done;
      });
      await tester.pumpAndSettle();
      expect(task.running, isFalse);
      expect(find.text('安装完成'), findsOneWidget);
      expect(find.textContaining('cleanup_fixture_failure'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );

  testWidgets('unavailable dynamic selection has no retired local ZIP input', (
    tester,
  ) async {
    final model = InstallFixtureModel('chunked');
    final patch = model.snapshot!.rows.single.patches.single;
    patch['actions']['primary']['input'] = 'unavailable';
    patch['actions']['automatic']['available'] = false;
    final desktop = FixtureDesktop();
    await mountPatch(tester, model, desktop);
    expect(find.text('需要选择本地 ZIP'), findsNothing);
    expect(find.text('使用本地 ZIP（可选）'), findsNothing);
    expect(find.text('选择本地 ZIP'), findsNothing);
    expect(model.tasks.installation, isNull);
    expect(
      tester.widget<FilledButton>(find.byType(FilledButton)).onPressed,
      isNull,
    );
    await tester.pumpWidget(const SizedBox.shrink());
    model.dispose();
  });

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
      'substage': null,
      'bytes_done': 4000000,
      'total_bytes': 8000000,
    };
    await tester.pumpWidget(
      PatchHubApp(
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
      PatchHubApp(
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
    expect(find.text('50%'), findsOneWidget);
    expect(find.text('补丁：测试补丁'), findsOneWidget);
    expect(find.text('下载完成后将继续校验与安装'), findsOneWidget);
    expect(find.textContaining('个文件'), findsNothing);

    Future<void> rebuild() => tester.pumpWidget(
      PatchHubApp(
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
    // A download with an unknown byte total must not use unrelated file totals.
    task.progress = {...task.progress!, 'total_bytes': null, 'total_files': 10};
    await rebuild();
    expect(
      tester
          .widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator))
          .value,
      isNull,
    );
    expect(find.text('50%'), findsNothing);
    expect(find.text('已处理 4.0 MB'), findsOneWidget);

    task.progress = {
      ...task.progress!,
      'stage': 'applying',
      'substage': 'writing',
      'bytes_done': 0,
      'files_done': 3,
      'total_files': 10,
      'current_file': '游戏目录/补丁文件.dat',
    };
    await rebuild();
    expect(find.text('30%'), findsOneWidget);
    expect(find.text('3 / 10 个文件'), findsOneWidget);
    expect(find.text('游戏目录/补丁文件.dat'), findsOneWidget);
    expect(find.textContaining('下载速度：'), findsNothing);
    await tester.runAsync(() async {
      await task.cancel();
      await task.done;
    });
    await tester.pumpAndSettle();
    expect(find.textContaining('下载速度：'), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
    model.dispose();
  });

  testWidgets(
    'header close waits for cancellation receipt and then closes result',
    (tester) async {
      final model = InstallFixtureModel('cancel_wait');
      await mountPatch(tester, model, FixtureDesktop());
      await tester.runAsync(() => tester.tap(find.text('安装补丁')));
      await tester.pump();
      final task = model.tasks.installation!;
      await tester.runAsync(() => tester.tap(find.byTooltip('取消安装')));
      expect(task.cancelRequested, isTrue);
      expect(find.byType(InstallDialog), findsOneWidget);
      await tester.runAsync(() => task.done);
      await tester.pumpAndSettle();
      expect(find.text('已取消安装'), findsOneWidget);
      expect(task.diagnostics, contains('cancel_count=1'));
      await tester.tap(find.byTooltip('关闭结果'));
      await tester.pumpAndSettle();
      expect(find.byType(InstallDialog), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
      model.dispose();
    },
  );
}
