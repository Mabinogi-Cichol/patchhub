import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/app.dart';
import 'package:patchhub_windows/host/app_lifecycle.dart';
import 'package:patchhub_windows/host/source_tasks.dart';
import 'package:patchhub_windows/presentation/desktop_toolbar.dart';

import 'support/process_fixture_client.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const channel = MethodChannel('window_manager');
  testWidgets(
    'caption close enters the app lifecycle without a native close round trip',
    (tester) async {
      var requested = 0;
      final nativeCalls = <String>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, (call) async {
            nativeCalls.add(call.method);
            return null;
          });
      await tester.pumpWidget(
        PatchHubApp(
          onClose: () => requested++,
          home: const Scaffold(body: DesktopToolbar(actions: [])),
        ),
      );
      await tester.tap(find.byTooltip('关闭'));
      await tester.pump();
      expect(requested, 1);
      expect(nativeCalls, isNot(contains('close')));
    },
  );
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
          if (call.method == 'hide' || call.method == 'close') {
            expect(task.running, isFalse);
          }
          if (call.method == 'close') {
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
        var hidden = false;
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(channel, (call) async {
              if (call.method == 'close') destroyed = true;
              if (call.method == 'hide') hidden = true;
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
        expect(hidden, isFalse);
        expect(lifecycle.readyToExit, isTrue);
        await lifecycle.finishExit();
        expect(destroyed, isTrue);
        expect(hidden, isTrue);
        lifecycle.dispose();
        host.dispose();
      },
    );
  }

  test('idle close hides the window before normal native teardown', () async {
    final calls = <String>[];
    final closed = Completer<void>();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
          calls.add(call.method);
          if (call.method == 'close') closed.complete();
          return null;
        });
    final host = AppTasks(ProcessFixtureClient('chunked'));
    final lifecycle = AppLifecycle(tasks: host);
    await lifecycle.initialize();
    calls.clear();
    lifecycle.onWindowClose();
    await closed.future;
    expect(calls, ['setPreventClose', 'hide', 'close']);
    expect(host.active, isEmpty);
    expect(lifecycle.error, isNull);
    lifecycle.dispose();
    host.dispose();
  });

  test(
    'a native close failure restores the hidden window and reports the error',
    () async {
      final calls = <String>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, (call) async {
            calls.add(call.method);
            if (call.method == 'close') {
              throw PlatformException(code: 'native_close_failed');
            }
            return call.method == 'isMinimized' ? false : null;
          });
      final lifecycle = AppLifecycle();
      await lifecycle.finishExit();
      expect(calls, [
        'setPreventClose',
        'hide',
        'close',
        'isMinimized',
        'show',
      ]);
      expect(lifecycle.error, isA<PlatformException>());
      expect(lifecycle.closing, isFalse);
      lifecycle.dispose();
    },
  );
}
