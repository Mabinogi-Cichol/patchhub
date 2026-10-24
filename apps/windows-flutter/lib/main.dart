import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:window_manager/window_manager.dart';

import 'app.dart';
import 'host/app_lifecycle.dart';
import 'host/app_update.dart';
import 'pages/app_update_panel.dart';
import 'host/core_client.dart';
import 'host/core_paths.dart';
import 'host/source_tasks.dart';
import 'pages/library_page.dart';
import 'models/app_model.dart';
import 'presentation/desktop_toolbar.dart';

Future<void> main(List<String> arguments) async {
  WidgetsFlutterBinding.ensureInitialized();
  await windowManager.ensureInitialized();
  final directory = File(Platform.resolvedExecutable).parent;
  if (!arguments.contains('--update-probe')) {
    final maintenance = File('${directory.path}/.steam-patch-maintenance.lock');
    if (await maintenance.exists()) {
      try {
        final handle = await maintenance.open(mode: FileMode.append);
        await handle.close();
      } on FileSystemException {
        stderr.writeln('应用正在更新或卸载，请稍后重试。');
        exit(1);
      }
    }
  }
  final marker = File('${directory.path}/.steam-patch-update.json');
  if (!arguments.contains('--update-probe') && await marker.exists()) {
    final transaction = jsonDecode(await marker.readAsString()) as Map;
    await Process.start('powershell.exe', [
      '-NoProfile',
      '-NonInteractive',
      '-ExecutionPolicy',
      'Bypass',
      '-File',
      '${transaction['work']}/windows-update.ps1',
      '-Mode',
      'Recover',
      '-Target',
      directory.path,
      '-ParentId',
      '$pid',
    ], mode: ProcessStartMode.detached);
    exit(0);
  }
  if (arguments.length == 2 && arguments[0] == '--update-probe') {
    try {
      final request =
          jsonDecode(await File(arguments[1]).readAsString()) as Map;
      final metadata = jsonDecode(
        await File('${directory.path}/app-version.json').readAsString(),
      ) as Map;
      if (request['target'] != directory.path ||
          request['version'] != metadata['version']) {
        throw StateError('更新目标不匹配');
      }
      final protocol = WireProtocol(
        jsonDecode(
          await rootBundle.loadString('assets/protocol-v5.schema.json'),
        ) as Map<String, dynamic>,
      );
      await CoreClient(CorePaths.resolve(), protocol).capabilities();
      runApp(
        const SteamPatchApp(
          home: Scaffold(body: Center(child: Text('正在确认更新…'))),
        ),
      );
      await WidgetsBinding.instance.endOfFrame;
      final work = request['work'] as String;
      await File('$work/ready.json').writeAsString(
        jsonEncode({'pid': pid, 'version': metadata['version']}),
        flush: true,
      );
      while (!await File('$work/commit.json').exists()) {
        if (await File('$work/abort.json').exists()) exit(1);
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      await File('$work/ack.json').writeAsString('{}', flush: true);
    } catch (e) {
      stderr.writeln(e);
      exit(1);
    }
  }
  AppTasks? tasks;
  AppModel? model;
  Widget home;
  try {
    final protocol = WireProtocol(
      jsonDecode(await rootBundle.loadString('assets/protocol-v5.schema.json'))
          as Map<String, dynamic>,
    );
    final client = CoreClient(CorePaths.resolve(), protocol);
    model = AppModel(client);
    tasks = model.tasks;
    home = LibraryPage(model: model);
  } catch (error) {
    home = Scaffold(
      appBar: const PreferredSize(
        preferredSize: Size.fromHeight(52),
        child: DesktopToolbar(actions: []),
      ),
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: SelectableText('$error'),
        ),
      ),
    );
  }
  final lifecycle = AppLifecycle(tasks: tasks, onReturnToApp: model?.refresh);
  await lifecycle.initialize();
  final metadataFile = File('${directory.path}/app-version.json');
  final version = await metadataFile.exists()
      ? (jsonDecode(await metadataFile.readAsString()) as Map)['version']
            as String
      : 'development';
  final appData = Platform.environment['APPDATA'];
  final update = AppUpdate(
    directory: directory,
    current: version,
    stateFile: File('$appData/io.github.steampatch.manager/updates/state.json'),
  );
  final updateHome = AppUpdatePanel(
    update: update,
    lifecycle: lifecycle,
    child: home,
  );
  const options = WindowOptions(
    size: Size(1180, 760),
    minimumSize: Size(850, 560),
    center: true,
    title: 'Steam 黄油补丁助手',
    titleBarStyle: TitleBarStyle.hidden,
    windowButtonVisibility: false,
  );
  windowManager.waitUntilReadyToShow(options, () async {
    await windowManager.show();
    await windowManager.focus();
  });
  runApp(
    SteamPatchApp(
      home: ListenableBuilder(
        listenable: Listenable.merge([lifecycle, ?tasks]),
        builder: (context, _) {
          Widget? overlay;
          if (lifecycle.error != null) {
            overlay = Scaffold(
              body: Center(
                child: Padding(
                  padding: const EdgeInsets.all(24),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      const Text('任务已结束，退出前请查看以下信息'),
                      const SizedBox(height: 12),
                      SelectableText('${lifecycle.error}'),
                      const SizedBox(height: 16),
                      Wrap(
                        spacing: 12,
                        children: [
                          OutlinedButton(
                            onPressed: lifecycle.returnToApp,
                            child: const Text('返回应用'),
                          ),
                          if (lifecycle.readyToExit)
                            FilledButton(
                              onPressed: lifecycle.finishExit,
                              child: const Text('关闭应用'),
                            ),
                        ],
                      ),
                    ],
                  ),
                ),
              ),
            );
          } else if (lifecycle.closing) {
            overlay = Scaffold(
              body: Center(
                child: Padding(
                  padding: const EdgeInsets.all(24),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      const CircularProgressIndicator(),
                      const SizedBox(height: 20),
                      const Text('正在取消任务并处理已修改文件，请等待实际结果…'),
                      if (tasks != null)
                        for (final task in tasks.active)
                          if (task.controlError != null)
                            SelectableText('${task.controlError}'),
                    ],
                  ),
                ),
              ),
            );
          }
          return Stack(
            children: [
              ExcludeFocus(
                excluding: overlay != null,
                child: AbsorbPointer(
                  absorbing: overlay != null,
                  child: updateHome,
                ),
              ),
              if (overlay != null) Positioned.fill(child: overlay),
            ],
          );
        },
      ),
    ),
  );
}
