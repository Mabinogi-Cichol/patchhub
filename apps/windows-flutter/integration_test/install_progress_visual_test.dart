import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/steam_artwork.dart';
import 'package:patchhub_windows/pages/install_dialog.dart';
import 'package:patchhub_windows/pages/library_page.dart';
import 'package:window_manager/window_manager.dart';

import '../test/installation_flow_test.dart' show InstallFixtureModel;

// Real Windows rendering with a controlled CLI fixture; no game is installed.
class ProgressVisualModel extends InstallFixtureModel {
  ProgressVisualModel(super.scenario);
  @override
  Future<void> initialize() async {}
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  const outputPath = String.fromEnvironment('PATCHHUB_VISUAL_OUTPUT');

  testWidgets('installation dialog native visual matrix', (tester) async {
    await windowManager.ensureInitialized();
    await windowManager.setTitleBarStyle(TitleBarStyle.hidden);
    final art = SteamArtworkStore(languageForRoot: (_) => 'schinese');
    final artwork = Future.value(await art.load(1011940));
    final output = outputPath.isEmpty ? null : Directory(outputPath);
    await output?.create(recursive: true);
    final captures = <String>[];

    for (final dark in [true, false]) {
      for (final compact in [false, true]) {
        final model = ProgressVisualModel('cancel_wait');
        final row = model.snapshot!.rows.single;
        row.game['localized_name'] = '冲突的意志';
        row.game['appid'] = 1011940;
        final task = model.tasks.startInstall({});
        await task.events.first;
        task.progress = {
          ...task.progress!,
          'stage': 'downloading',
          'substage': null,
          'bytes_done': 84000000,
          'total_bytes': 200000000,
        };
        task.downloadRate.update('downloading', 0, Duration.zero);
        task.downloadRate.update(
          'downloading',
          84000000,
          const Duration(seconds: 10),
        );
        await windowManager.setSize(
          compact ? const Size(850, 560) : const Size(1440, 900),
        );
        final key = GlobalKey();
        await tester.pumpWidget(
          RepaintBoundary(
            key: key,
            child: PatchHubApp(
              home: Builder(
                builder: (context) => MediaQuery(
                  data: MediaQuery.of(
                    context,
                  ).copyWith(textScaler: TextScaler.linear(compact ? 1.5 : 1)),
                  child: LibraryPage(model: model, artwork: art),
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.text(row.name));
        await tester.pumpAndSettle();
        final context = tester.element(find.byType(LibraryPage));
        PatchHubApp.setThemeMode(
          context,
          dark ? ThemeMode.dark : ThemeMode.light,
        );
        await tester.pumpAndSettle();
        final result = showDialog<String>(
          context: context,
          barrierDismissible: false,
          builder: (context) => MediaQuery(
            data: MediaQuery.of(context)
                .copyWith(textScaler: TextScaler.linear(compact ? 1.5 : 1)),
            child: InstallDialog(
              model: model,
              task: task,
              game: compact ? '冲突的意志：长游戏名称与大字体布局检查' : '冲突的意志',
              patch: compact ? '冲突的意志 简体中文补丁与额外内容完整安装包' : '冲突的意志',
              artwork: artwork,
            ),
          ),
        );
        await tester.pump(const Duration(milliseconds: 400));

        Future<void> capture(String state) async {
          // Refresh the route after changing the fixture's displayed progress.
          tester.element(find.byType(InstallDialog)).markNeedsBuild();
          await tester.pump(const Duration(milliseconds: 100));
          for (final image in tester.widgetList<Image>(find.byType(Image))) {
            await precacheImage(image.image, context);
          }
          await tester.pump(const Duration(milliseconds: 100));
          expect(tester.takeException(), isNull);
          final name =
              '${dark ? 'dark' : 'light'}-${compact ? 'compact' : 'normal'}-$state';
          if (output != null) {
            final boundary =
                key.currentContext!.findRenderObject() as RenderRepaintBoundary;
            final image = await boundary.toImage();
            final bytes = await image.toByteData(
              format: ui.ImageByteFormat.png,
            );
            await File('${output.path}/$name.png')
                .writeAsBytes(bytes!.buffer.asUint8List());
            image.dispose();
          }
          captures.add(name);
        }

        await capture('download');
        expect(find.text('42%'), findsOneWidget);
        expect(find.text('84.0 MB / 200.0 MB'), findsOneWidget);
        if (!compact) {
          task.progress = {
            ...task.progress!,
            'total_bytes': null,
            'total_files': 4,
          };
          await capture('unknown');
          expect(
            tester
                .widget<LinearProgressIndicator>(
                  find.byType(LinearProgressIndicator),
                )
                .value,
            isNull,
          );
          task.progress = {
            ...task.progress!,
            'stage': 'applying',
            'substage': 'writing',
            'bytes_done': 0,
            'files_done': 2,
            'total_files': 4,
            'current_file': '游戏目录/补丁文件/patch.dat',
          };
          await capture('writing');
          expect(find.text('2 / 4 个文件'), findsOneWidget);
          expect(find.textContaining('下载速度：'), findsNothing);
        }
        await tester.tap(find.byTooltip('取消安装'));
        await task.done;
        await tester.pumpAndSettle();
        await capture('cancelled');
        expect(find.text('已取消安装'), findsOneWidget);
        await tester.tap(find.byTooltip('关闭结果'));
        await tester.pumpAndSettle();
        await result;
        await tester.pumpWidget(const SizedBox.shrink());
        model.dispose();
      }
    }
    if (output != null) {
      await File('${output.path}/result.json')
          .writeAsString(jsonEncode({'passed': true, 'screenshots': captures}));
    }
  });
}
