import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/source_tasks.dart';

import 'support/process_fixture_client.dart';

void main() {
  test('scan returns the normal DTO through the cancellable channel', () async {
    final client = ProcessFixtureClient('chunked');
    final result = await client.call('scan') as Map;
    expect(result['installs'], isNotEmpty);
    await client.waitForIdle();
  });

  for (final method in ['scan', 'project_instances', 'preview']) {
    test(
      'shutdown cancels $method even while its preflight is pending',
      () async {
        final gate = Completer<void>();
        final client = ProcessFixtureClient('cancel_wait', gate: gate);
        final host = AppTasks(client);
        final read = client.call(method);
        final rejected = expectLater(
          read,
          throwsA(
            isA<CoreError>().having(
              (error) => error.code,
              'code',
              'task_cancelled',
            ),
          ),
        );
        final closing = host.shutdown();
        await expectLater(
          client.call('list_sources'),
          throwsA(
            isA<CoreError>().having(
              (error) => error.code,
              'code',
              'app_closing',
            ),
          ),
        );
        gate.complete();
        await rejected;
        expect(await closing, isEmpty);
        host.resume();
        expect(await client.call('get_capabilities'), isA<Map>());
        host.dispose();
      },
    );
  }

  test('shutdown still waits for an existing recovery write', () async {
    final gate = Completer<void>();
    final client = ProcessFixtureClient('chunked', gate: gate);
    final host = AppTasks(client);
    final recovery = client.call('recover');
    var closed = false;
    final closing = host.shutdown().then((_) => closed = true);
    await Future<void>.delayed(Duration.zero);
    expect(closed, isFalse);
    gate.complete();
    expect(await recovery, isA<List>());
    await closing;
    expect(closed, isTrue);
    host.dispose();
  });
}
