import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'core_paths.dart';
import 'core_task.dart';
import 'protocol_models.dart';
export 'protocol_models.dart';

class CoreClient {
  CoreClient(this.paths, this.protocol);
  final CorePaths paths;
  final WireProtocol protocol;
  Future<Process> startProcess(String channel) =>
      Process.start(paths.binary, [channel]);
  final _calls = <Future<void>>{};
  final _reads = <CoreTask>{};
  bool _closing = false;

  Future<void> cancelReads() async {
    _closing = true;
    await Future.wait(_reads.toList().map((task) => task.cancel()));
  }

  void resume() => _closing = false;
  Future<T> _tracked<T>(Future<T> Function() action) async {
    final done = Completer<void>();
    _calls.add(done.future);
    try {
      return await action();
    } finally {
      _calls.remove(done.future);
      done.complete();
    }
  }

  Future<void> waitForIdle() async {
    while (_calls.isNotEmpty) {
      await Future.wait(_calls.toList());
    }
  }

  int _counter = 0;
  String requestId() =>
      'flutter-$pid-${DateTime.now().microsecondsSinceEpoch}-${_counter++}';

  Future<Object?> call(String method, [Map<String, Object?> args = const {}]) =>
      _tracked(() async {
        if (_closing) throw const CoreError('app_closing');
        if (['scan', 'project_instances', 'preview'].contains(method)) {
          // Reuse the existing task channel: these reads may hash large files.
          final task = CoreTask.start(this, method, args);
          _reads.add(task);
          try {
            await task.done;
            if (task.error != null) throw task.error!;
            if (task.cancelRequested) throw const CoreError('task_cancelled');
            return task.result;
          } finally {
            _reads.remove(task);
          }
        }
        if (method != 'get_capabilities') await capabilities();
        return _invoke(method, args);
      });

  Future<Map<String, dynamic>> capabilities() => _tracked(
    () async =>
        await _invoke('get_capabilities', const {}) as Map<String, dynamic>,
  );
  Future<Object?> _invoke(String method, Map<String, Object?> args) async {
    final id =
        'flutter-$pid-${DateTime.now().microsecondsSinceEpoch}-${_counter++}';
    final process = await startProcess('serve');
    // Both pipes are drained immediately so neither can block the child.
    final output = process.stdout.transform(utf8.decoder).join();
    final diagnostic = process.stderr.transform(utf8.decoder).join();
    Object? writeError;
    try {
      process.stdin.writeln(
        jsonEncode({
          'protocol_version': 5,
          'request_id': id,
          'method': method,
          'params': {...args, 'host': paths.host},
        }),
      );
      await process.stdin.close();
    } catch (error) {
      writeError = error;
    }
    final exitCode = await process.exitCode;
    final stdout = await output;
    final stderr = await diagnostic;
    if (writeError != null || exitCode != 0) {
      throw CoreError(
        'core_process_failed',
        'exit=$exitCode\n$stderr${writeError ?? ''}',
      );
    }
    if (!stdout.endsWith('\n') ||
        stdout.substring(0, stdout.length - 1).contains('\n')) {
      throw const CoreError('invalid_core_response');
    }
    return protocol.response(jsonDecode(stdout), id, method);
  }
}
