import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/app_lifecycle.dart';
import 'package:patchhub_windows/host/source_tasks.dart';

import 'support/process_fixture_client.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const channel = MethodChannel('window_manager');
  tearDown(
    () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, null),
  );

  test('minimize leaves task running, close waits for cancellation before native close', () async {
    final gate = Completer<void>();
    final host = AppTasks(ProcessFixtureClient('cancel_wait', gate: gate));
    final task = host.startInstall({});
    final destroyed = Completer<void>();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
          if (call.method == 'close') {
            expect(task.running, isFalse);
            destroyed.complete();
          }
          return null;
        });
    final lifecycle = AppLifecycle(tasks: host);
    await lifecycle.initialize();
    lifecycle.onWindowMinimize();
    expect(task.cancelRequested, isFalse);
    lifecycle.onWindowClose();
    expect(lifecycle.closing, isTrue);
    expect(destroyed.isCompleted, isFalse);
    gate.complete();
    await destroyed.future;
    expect(lifecycle.error, isNull);
    lifecycle.dispose();
    host.dispose();
  });

  for (final (scenario, message) in [
    ('recover_wait', '仍需恢复'),
    ('failed_wait', '未完成'),
  ]) {
    test(
      '$scenario stays visible until explicit exit after task completion',
      () async {
        final gate = Completer<void>();
        final host = AppTasks(ProcessFixtureClient(scenario, gate: gate));
        host.startInstall({});
        var destroyed = false;
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(channel, (call) async {
              if (call.method == 'close') destroyed = true;
              return null;
            });
        final lifecycle = AppLifecycle(tasks: host);
        final notice = Completer<void>();
        lifecycle.addListener(() {
          if (lifecycle.error != null && !notice.isCompleted) notice.complete();
        });
        await lifecycle.initialize();
        lifecycle.onWindowClose();
        gate.complete();
        await notice.future;
        expect(host.active, isEmpty);
        expect(lifecycle.error.toString(), contains(message));
        expect(destroyed, isFalse);
        expect(lifecycle.readyToExit, isTrue);
        await lifecycle.finishExit();
        expect(destroyed, isTrue);
        lifecycle.dispose();
        host.dispose();
      },
    );
  }
}
