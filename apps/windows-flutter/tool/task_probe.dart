import 'dart:convert';
import 'dart:io';

import 'package:steam_patch_windows/host/core_client.dart';
import 'package:steam_patch_windows/host/core_paths.dart';
import 'package:steam_patch_windows/host/core_task.dart';

Future<void> main(List<String> args) async {
  if (args.length != 4 && args.length != 5) {
    throw ArgumentError(
      'task_probe <core> <state-copy> <resources> <source_id> [complete]',
    );
  }
  final protocol = WireProtocol(
    jsonDecode(
      await File.fromUri(
        Platform.script.resolve('../assets/protocol-v5.schema.json'),
      ).readAsString(),
    ) as Map<String, dynamic>,
  );
  final client = CoreClient(
    CorePaths(
      binary: args[0],
      stateDirectory: args[1],
      resourceDirectory: args[2],
    ),
    protocol,
  );
  final task = CoreTask.start(client, 'update_source', {
    'source_id': args[3],
    'force': args.length != 5,
  });
  final events = <Map<String, dynamic>>[];
  task.events.listen((event) {
    events.add(event);
    if (args.length != 5) task.cancel();
  });
  await task.done;
  stdout.writeln(
    jsonEncode({
      'request_id': task.requestId,
      'cancel_requested': task.cancelRequested,
      'events': events,
      'result': task.result,
      'error': task.error?.toString(),
      'control_error': task.controlError?.toString(),
      'exit_code': task.exitCode,
      'diagnostics': task.diagnostics,
    }),
  );
}
