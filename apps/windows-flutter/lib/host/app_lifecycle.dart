import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

import 'source_tasks.dart';

class AppLifecycle extends ChangeNotifier with WindowListener {
  AppLifecycle({this.tasks, this.onReturnToApp});
  final AppTasks? tasks;
  final VoidCallback? onReturnToApp;
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
      final notices = await tasks?.shutdown() ?? <String>[];
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

  Future<void> finishExit() async {
    try {
      await windowManager.destroy();
    } catch (failure) {
      error = failure;
      closing = false;
      notifyListeners();
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
