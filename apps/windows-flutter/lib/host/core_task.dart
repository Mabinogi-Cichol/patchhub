import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'core_client.dart';
import 'download_rate.dart';

class EventSequence {
  EventSequence(this.protocol, this.requestId);
  final WireProtocol protocol;
  final String requestId;
  int next = 0;
  bool terminal = false;

  bool accept(Map<String, dynamic> event) {
    protocol.event(event, requestId);
    if (terminal) return false;
    final sequence = event['sequence'] as int;
    if (sequence < next) return false;
    if (sequence != next) throw const CoreError('invalid_core_response');
    next++;
    return true;
  }
}

/// One CLI lifecycle. A cancellation request never substitutes for a receipt.
/// Malformed output closes stdin (the core's EOF cancellation contract) and
/// continues draining until the process has actually finished cleanup.
class CoreTask {
  CoreTask.start(this.client, this.method, this.arguments)
    : requestId = client.requestId() {
    unawaited(_run());
  }
  final CoreClient client;
  final String method;
  final Map<String, Object?> arguments;
  final String requestId;
  final _changes = StreamController<void>.broadcast();
  final _events = StreamController<Map<String, dynamic>>.broadcast();
  Stream<Map<String, dynamic>> get events => _events.stream;
  final _finished = Completer<void>();
  Stream<void> get changes => _changes.stream;
  Future<void> get done => _finished.future;
  bool running = true;
  bool cancelRequested = false;
  bool _cancelSent = false;
  bool _requestWritten = false;
  bool _controlClosed = false;
  Process? _process;
  final downloadRate = DownloadRate();
  final _progressClock = Stopwatch()..start();
  Map<String, dynamic>? progress;
  Map<String, dynamic>? lastEvent;
  dynamic result;
  CoreError? error;
  CoreError? controlError;
  String diagnostics = '';
  int? exitCode;

  void _notify() => _changes.add(null);

  Future<void> cancel() async {
    if (!running || cancelRequested) return;
    cancelRequested = true;
    _notify();
    if (_requestWritten) await _sendCancel();
  }

  Future<void> _sendCancel() async {
    if (_cancelSent || _controlClosed || _process == null) return;
    _cancelSent = true;
    try {
      _process!.stdin.writeln('cancel');
      await _process!.stdin.flush();
    } catch (failure) {
      controlError = CoreError('control_write_failed', '$failure');
      _notify();
      await _closeControl();
    }
  }

  Future<void> _closeControl() async {
    if (_controlClosed || _process == null) return;
    _controlClosed = true;
    try {
      await _process!.stdin.close();
    } catch (failure) {
      controlError = CoreError(
        'control_write_failed',
        '${controlError?.detail ?? ''}\n$failure',
      );
      _notify();
    }
  }

  bool _allowed(String event) => switch (method) {
    'install_patch' => event == 'install_progress',
    'update_source' => [
      'source_progress',
      'source_update',
      'match_progress',
      'match_update',
    ].contains(event),
    'match_games' ||
    'rematch_game' => event == 'match_progress' || event == 'match_update',
    _ => false,
  };

  Future<void> _run() async {
    Future<void>? readOutput;
    Future<void>? readErrors;
    var receivedTerminal = false;
    CoreError? rejected;
    try {
      await client.capabilities();
      final process = await client.startProcess('serve-task');
      _process = process;
      final sequence = EventSequence(client.protocol, requestId);
      readErrors = process.stderr
          .transform(utf8.decoder)
          .join()
          .then(
            (value) {
              diagnostics += value;
            },
            onError: (Object failure) {
              rejected ??= CoreError('core_process_failed', '$failure');
            },
          );
      // Keep framing explicit: LineSplitter alone accepts an unterminated EOF.
      readOutput = (() async {
        var pending = '';
        try {
          await for (final chunk in process.stdout.transform(utf8.decoder)) {
            pending += chunk;
            int newline;
            while ((newline = pending.indexOf('\n')) != -1) {
              final line = pending.substring(0, newline);
              pending = pending.substring(newline + 1);
              if (rejected != null) continue;
              try {
                if (receivedTerminal) {
                  try {
                    final tail = jsonDecode(line);
                    if (tail is! Map || !tail.containsKey('event')) {
                      diagnostics += '\ninvalid_core_response_after_terminal';
                    }
                  } catch (_) {
                    diagnostics += '\ninvalid_core_response_after_terminal';
                  }
                  continue;
                }
                final value = jsonDecode(line);
                if (value is Map<String, dynamic> &&
                    value.containsKey('event')) {
                  if (!sequence.accept(value)) continue;
                  if (!_allowed(value['event'] as String)) {
                    throw const CoreError('invalid_core_response');
                  }
                  if (value['event'] == 'install_progress') {
                    downloadRate.update(
                      value['stage'] as String,
                      value['bytes_done'] as int,
                      _progressClock.elapsed,
                    );
                  }
                  lastEvent = value;
                  _events.add(value);
                  if ((value['event'] as String).endsWith('_progress')) {
                    progress = value;
                  }
                  _notify();
                } else {
                  // A valid domain failure is also a terminal receipt. Its
                  // error is distinct from a broken stream with no receipt.
                  try {
                    result = client.protocol.response(value, requestId, method);
                  } on CoreError catch (failure) {
                    if (value is Map &&
                        value['ok'] == false &&
                        value['error'] is Map &&
                        failure.code == value['error']['code'] &&
                        failure.detail == value['error']['detail'] &&
                        value['protocol_version'] == 5 &&
                        value['request_id'] == requestId &&
                        client.protocol.matches(
                          value,
                          client.protocol.schema[r'$defs']['ErrorResponse'],
                        )) {
                      error = failure;
                    } else {
                      rethrow;
                    }
                  }
                  receivedTerminal = true;
                  sequence.terminal = true;
                  await _closeControl();
                  _notify();
                }
              } catch (failure) {
                rejected = failure is CoreError
                    ? failure
                    : CoreError('invalid_core_response', '$failure');
                // Do not kill an installer while it may be rolling back.
                cancelRequested = true;
                await _sendCancel();
                await _closeControl();
                _notify();
              }
            }
          }
          if (!receivedTerminal) {
            cancelRequested = true;
            await _sendCancel();
            await _closeControl();
          }
          if (pending.isNotEmpty && !receivedTerminal) {
            rejected ??= const CoreError('invalid_core_response');
          }
        } catch (failure) {
          rejected ??= CoreError('invalid_core_response', '$failure');
          cancelRequested = true;
          await _sendCancel();
          await _closeControl();
        }
      })();
      try {
        process.stdin.writeln(
          jsonEncode({
            'protocol_version': 5,
            'request_id': requestId,
            'method': method,
            'params': {...arguments, 'host': client.paths.host},
          }),
        );
        await process.stdin.flush();
        _requestWritten = true;
        if (cancelRequested) await _sendCancel();
      } catch (failure) {
        rejected ??= CoreError('core_process_failed', '$failure');
        await _closeControl();
      }
      exitCode = await process.exitCode;
      await readOutput;
      await readErrors;
      if (rejected != null) {
        error = rejected;
      } else if (!receivedTerminal) {
        error = CoreError(
          'core_process_failed',
          '未收到任务终态；exit=$exitCode\n$diagnostics',
        );
      } else if (exitCode != 0) {
        // Preserve the domain receipt, but expose teardown failure separately.
        diagnostics += '\ncore_process_failed_after_terminal: exit=$exitCode';
      }
    } catch (failure) {
      error = failure is CoreError
          ? failure
          : CoreError('core_process_failed', '$failure');
      await _closeControl();
      if (_process != null) exitCode = await _process!.exitCode;
      if (readOutput != null) await readOutput;
      if (readErrors != null) await readErrors;
    } finally {
      await _closeControl();
      running = false;
      _notify();
      _finished.complete();
      await _changes.close();
      await _events.close();
    }
  }
}
