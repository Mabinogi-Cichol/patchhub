import 'dart:convert';
import 'dart:io';

import 'package:steam_patch_windows/host/core_client.dart';
import 'package:steam_patch_windows/host/core_paths.dart';
import 'package:steam_patch_windows/host/core_task.dart';

class ProbePaths extends CorePaths {
  ProbePaths(this.spec)
    : super(
        binary: spec['binary'],
        stateDirectory: spec['state'],
        resourceDirectory: spec['resources'],
      );
  final Map<String, dynamic> spec;
  @override
  Map<String, Object?> get host => {
    ...super.host,
    if (spec['roots'] != null) 'roots': spec['roots'],
    if (spec['signed_catalog'] != null)
      'signed_catalog': spec['signed_catalog'],
    if (spec['trust_keys'] != null) 'trust_keys': spec['trust_keys'],
  };
}

Future<void> main(List<String> args) async {
  final spec = jsonDecode(
    await File(args.single).readAsString(),
  ) as Map<String, dynamic>;
  final protocol = WireProtocol(
    jsonDecode(
      await File.fromUri(
        Platform.script.resolve('../assets/protocol-v5.schema.json'),
      ).readAsString(),
    ) as Map<String, dynamic>,
  );
  final client = CoreClient(ProbePaths(spec), protocol);
  final scan = await client.call('scan') as Map<String, dynamic>;
  final rows = scan['installs'] as List;
  final row = rows.singleWhere((row) => row['game']['appid'] == spec['appid']);
  final patch = (row['patches'] as List).firstWhere(
    (patch) => spec['release_id'] != null
        ? patch['release_id'] == spec['release_id']
        : patch['source_id'] == spec['source_id'],
  );
  final arguments = <String, Object?>{
    'instance_id': row['game']['instance_id'],
    'operation': spec['operation'] ?? 'overwrite',
    'cleanup_policy': 'disabled',
    if (spec['release_id'] != null)
      'release_id': spec['release_id']
    else
      'selection_id': patch['selection_id'],
    if (spec['archive'] != null) 'archive': spec['archive'],
  };
  final task = CoreTask.start(client, 'install_patch', arguments);
  final events = <Map<String, dynamic>>[];
  task.events.listen((event) {
    events.add(event);
    if (event['stage'] == spec['cancel_stage'] &&
        (event['bytes_done'] as int? ?? 0) >=
            (spec['cancel_bytes'] as int? ?? 0)) {
      task.cancel();
    }
  });
  await task.done;
  final transactions = await client.call('list_transactions');
  final value = {
    'request': arguments,
    'events': events,
    'result': task.result,
    'error': task.error?.toString(),
    'control_error': task.controlError?.toString(),
    'exit_code': task.exitCode,
    'diagnostics': task.diagnostics,
    'cancel_requested': task.cancelRequested,
    'transactions': transactions,
  };
  await File(spec['output'])
      .writeAsString(const JsonEncoder.withIndent('  ').convert(value));
  stdout.writeln(
    jsonEncode({
      'result': task.result,
      'error': task.error?.toString(),
      'events': events.length,
      'cancel_requested': task.cancelRequested,
    }),
  );
  if (task.error != null || task.controlError != null) exitCode = 1;
}
