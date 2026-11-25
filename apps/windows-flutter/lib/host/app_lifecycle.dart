import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

import 'source_tasks.dart';

class AppLifecycle extends ChangeNotifier with WindowListener {
  AppLifecycle({this.tasks, this.onReturnToApp, this.beforeClose});
  final AppTasks? tasks;
  final VoidCallback? onReturnToApp;
  final Future<void> Function()? beforeClose;
  bool closing = false;
  bool readyToExit = false;
  Object? error;

  Future<void> initialize() async {
    windowManager.addListener(this);
    await windowManager.setPreventClose(true);
  }

  @override
  void onWindowClose() async {
    if (closing || readyToExit) return;
    closing = true;
    error = null;
    notifyListeners();
    try {
      final notices = await shutdown();
      readyToExit = true;
      if (notices.isNotEmpty) {
        error = notices.join('\n');
        closing = false;
        notifyListeners();
        return;
      }
      await finishExit();
    } catch (failure) {
      error = failure;
      closing = false;
      notifyListeners();
    }
  }

  Future<List<String>> shutdown() async {
    await beforeClose?.call();
    return await tasks?.shutdown() ?? <String>[];
  }

  Future<void> finishExit() async {
    try {
      // Use the normal WM_CLOSE -> WM_DESTROY path after cleanup. On Windows,
      // plugin destroy() posts WM_QUIT and skips the runner's window teardown.
      await windowManager.setPreventClose(false);
      // Business cleanup is complete. Stop displaying the window while the
      // native Flutter engine finishes its synchronous teardown.
      await windowManager.hide();
      await windowManager.close();
    } catch (failure) {
      error = failure;
      closing = false;
      notifyListeners();
      await windowManager.show();
    }
  }

  void returnToApp() {
    tasks?.resume();
    closing = false;
    readyToExit = false;
    error = null;
    notifyListeners();
    onReturnToApp?.call();
  }

  @override
  void dispose() {
    windowManager.removeListener(this);
    super.dispose();
  }
}
