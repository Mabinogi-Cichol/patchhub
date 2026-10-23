import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:window_manager/window_manager.dart';

import 'app.dart';
import 'host/app_lifecycle.dart';
import 'host/core_client.dart';
import 'host/core_paths.dart';
import 'host/source_tasks.dart';
import 'pages/library_page.dart';
import 'models/app_model.dart';
import 'presentation/desktop_toolbar.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await windowManager.ensureInitialized();
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
                child: AbsorbPointer(absorbing: overlay != null, child: home),
              ),
              if (overlay != null) Positioned.fill(child: overlay),
            ],
          );
        },
      ),
    ),
  );
}
