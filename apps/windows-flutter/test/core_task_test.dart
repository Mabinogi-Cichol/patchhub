import 'support/process_fixture_client.dart';

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_task.dart';
import 'package:patchhub_windows/host/source_tasks.dart';

class BrowserFixtureClient extends ProcessFixtureClient {
  BrowserFixtureClient({this.blocked = false, this.waitForCancel = false})
    : super('auto_source');
  final bool blocked, waitForCancel;
  final entered = Completer<void>();
  int browserCalls = 0;
  @override
  Future<List<Map<String, dynamic>>> collectSourcePages(
    String id,
    Future<void> cancelled,
    void Function(String) progress,
  ) {
    browserCalls++;
    entered.complete();
    progress('等待站点验证');
    if (blocked) {
      return Future.error(const CoreError('source_age_verification_required'));
    }
    if (waitForCancel) {
      return cancelled.then((_) => throw const CoreError('task_cancelled'));
    }
    return Future.value([]);
  }
}

void main() {
  test('a source update automatically changes transport with a fresh wire id in one task', () async {
    final client = BrowserFixtureClient();
    final task = CoreTask.start(client, 'update_source', {
      'source_id': 'patchmoe',
    });
    await task.done;
    expect(task.error, isNull);
    expect(task.result['complete'], true);
    expect(client.browserCalls, 1);
    final wires = task.diagnostics
        .split('\n')
        .where((line) => line.startsWith('wire:'))
        .toList();
    expect(wires, hasLength(2));
    expect(wires[0].split(':')[1], isNot(wires[1].split(':')[1]));
    expect(task.running, false);
  });
  test('cancelling background verification ends the original task without a second core call', () async {
    final client = BrowserFixtureClient(waitForCancel: true);
    final task = CoreTask.start(client, 'update_source', {
      'source_id': 'patchmoe',
    });
    await client.entered.future;
    await task.cancel();
    await task.done;
    expect(task.error?.code, 'task_cancelled');
    expect(task.controlError, isNull);
    expect(
      task.diagnostics.split('\n').where((line) => line.startsWith('wire:')),
      hasLength(1),
    );
  });
  test(
    'a required personal declaration stays a failed automatic update',
    () async {
      final client = BrowserFixtureClient(blocked: true);
      final task = CoreTask.start(client, 'update_source', {
        'source_id': 'patchmoe',
      });
      await task.done;
      expect(task.error?.code, 'source_age_verification_required');
      expect(task.result['complete'], false);
    },
  );
  final protocol = ProcessFixtureClient('chunked').protocol;
  final corpus = jsonDecode(
    File('../../packages/contracts/fixtures/v5/cases.json').readAsStringSync(),
  );
  for (final item in corpus['sequences'] as List) {
    test('sequence ${item['name']}', () {
      final sequence = EventSequence(protocol, 'fixture');
      for (var i = 0; i < (item['events'] as List).length; i++) {
        if (item['terminal_after'] == i) sequence.terminal = true;
        final value = item['events'][i] as Map<String, dynamic>;
        if (item['expected'][i] == 'error') {
          expect(() => sequence.accept(value), throwsA(isA<CoreError>()));
        } else {
          expect(sequence.accept(value), item['expected'][i] == 'accept');
        }
      }
    });
  }
  for (final scenario in [
    'chunked',
    'duplicate',
    'stderr',
    'late_noise',
    'terminal_nonzero',
  ]) {
    test('real child stream: $scenario', () async {
      final task = CoreTask.start(
        ProcessFixtureClient(scenario),
        'install_patch',
        {},
      );
      await task.done;
      expect(task.error, isNull);
      expect(task.result['state'], 'committed');
      expect(task.running, isFalse);
      if (scenario == 'chunked') {
        expect(task.lastEvent!['current_file'], '中文/补丁.dat');
      }
      if (scenario == 'duplicate') expect(task.lastEvent!['sequence'], 1);
      if (scenario == 'stderr') expect(task.diagnostics.length, 1024 * 1024);
      if (scenario == 'late_noise') {
        expect(task.diagnostics, contains('after_terminal'));
      }
      if (scenario == 'terminal_nonzero') {
        expect(task.diagnostics, contains('exit=7'));
      }
    });
  }
  test(
    'cancel before process start is retained, repeated cancel is one line',
    () async {
      final gate = Completer<void>();
      final task = CoreTask.start(
        ProcessFixtureClient('cancel_wait', gate: gate),
        'install_patch',
        {},
      );
      await task.cancel();
      await task.cancel();
      gate.complete();
      await task.done;
      expect(task.result['disposition'], 'cancelled');
      expect(task.error, isNull);
      expect(task.diagnostics, contains('cancel_count=1'));
    },
  );
  for (final scenario in [
    'gap',
    'wrong_event',
    'missing',
    'no_terminal',
    'unterminated',
  ]) {
    test('rejects $scenario and waits for child shutdown', () async {
      final task = CoreTask.start(
        ProcessFixtureClient(scenario),
        'install_patch',
        {},
      );
      await task.done;
      expect(task.error, isNotNull);
      expect(task.exitCode, isNotNull);
      if (scenario == 'gap' || scenario == 'wrong_event') {
        expect(task.diagnostics, contains('cancel_count=1'));
      }
    });
  }
  test(
    'source keys deduplicate while install keeps its own slot; shutdown waits',
    () async {
      final gate = Completer<void>();
      final host = AppTasks(ProcessFixtureClient('cancel_wait', gate: gate));
      final a = host.startSource('update_source', {'source_id': 'fixture'});
      expect(
        identical(
          a,
          host.startSource('update_source', {'source_id': 'fixture'}),
        ),
        isTrue,
      );
      final b = host.startSource('match_games', {});
      expect(
        identical(
          b,
          host.startSource('match_games', {
            'instances': ['other'],
          }),
        ),
        isTrue,
      );
      host.startSource('rematch_game', {'instance_id': 'one'});
      host.startSource('rematch_game', {'instance_id': 'two'});
      host.startInstall({});
      expect(host.active.length, 5);
      expect(() => host.startInstall({}), throwsA(isA<CoreError>()));
      final shutdown = host.shutdown();
      expect(
        () => host.startSource('match_games', {}),
        throwsA(isA<CoreError>()),
      );
      gate.complete();
      expect(await shutdown, isEmpty);
      expect(host.active, isEmpty);
      host.resume();
      final next = host.startSource('match_games', {});
      expect(identical(next, b), isFalse);
      await next.cancel();
      await next.done;
      host.dispose();
    },
  );
  test('cancel from first progress callback is not reentrant and keeps every event', () async {
    final task = CoreTask.start(
      ProcessFixtureClient('cancel_wait'),
      'install_patch',
      {},
    );
    final events = <Map<String, dynamic>>[];
    task.events.listen((event) {
      events.add(event);
      task.cancel();
    });
    await task.done;
    expect(events, hasLength(1));
    expect(task.result['disposition'], 'cancelled');
    expect(task.diagnostics, contains('cancel_count=1'));
  });
}
