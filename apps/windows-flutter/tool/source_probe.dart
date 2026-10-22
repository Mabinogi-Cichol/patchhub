import 'dart:convert';
import 'dart:io';

import 'package:steam_patch_windows/host/core_client.dart';
import 'package:steam_patch_windows/host/core_paths.dart';
import 'package:steam_patch_windows/host/core_task.dart';

Future<void> main(List<String> args) async {
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
  final scan = await client.call('scan') as Map<String, dynamic>;
  final row = (scan['installs'] as List).firstWhere(
    (r) => r['game']['appid'] == 2303800,
  );
  final instance = row['game']['instance_id'] as String;
  final revisions = <int>[];
  Future<void> mutate(String method, Map<String, Object?> values) async {
    final response = await client.call(method, values) as Map<String, dynamic>;
    revisions.add(response['settings_revision'] as int);
  }

  for (final id in ['patchmoe', 'kagura-en', 'hgamer']) {
    await mutate('set_source_enabled', {'source_id': id, 'enabled': false});
  }
  await mutate('set_source_update_interval', {
    'source_id': 'kagura-cn',
    'interval_hours': 3,
  });
  final scheduled = await client.call('list_sources') as List;
  if (scheduled.firstWhere((s) => s['id'] == 'kagura-cn')['interval_hours'] !=
      3) {
    throw StateError('interval not saved');
  }
  await mutate('set_source_update_interval', {
    'source_id': 'kagura-cn',
    'interval_hours': 0,
  });
  await mutate('set_game_source', {
    'instance_id': instance,
    'source_id': 'kagura-cn',
    'subscribed': false,
  });
  final off =
      await client.call('list_game_sources', {'instance_id': instance}) as List;
  if (off.firstWhere((s) => s['id'] == 'kagura-cn')['subscribed'] != false) {
    throw StateError('subscription not saved');
  }
  final disabledProjection = await client.call('project_instances', {
    'instance_ids': [instance],
  });
  await mutate('set_game_source', {
    'instance_id': instance,
    'source_id': 'kagura-cn',
    'subscribed': true,
  });
  final on = await client.call('list_game_sources', {'instance_id': instance});
  final rules = jsonDecode(
    await File.fromUri(
      Platform.script.resolve('../../../crates/core/resources/sources.json'),
    ).readAsString(),
  );
  final rule = Map<String, dynamic>.from(
    (rules is List ? rules : rules['sources']).first,
  );
  rule['id'] = 'flutter-acceptance';
  rule['name'] = 'Flutter隔离规则验收';
  rule['enabled'] = false;
  await mutate('add_source', {'rule': rule});
  final tasks = <Map<String, Object?>>[];
  for (final method in ['match_games', 'rematch_game']) {
    final task = CoreTask.start(
      client,
      method,
      method == 'match_games'
          ? {
              'instances': [instance],
            }
          : {'instance_id': instance},
    );
    final events = <Map<String, dynamic>>[];
    task.events.listen(events.add);
    await task.done;
    tasks.add({
      'method': method,
      'events': events,
      'result': task.result,
      'error': task.error?.toString(),
      'exit_code': task.exitCode,
    });
  }
  final projected = await client.call('project_instances', {
    'instance_ids': [instance],
  });
  final sources = await client.call('list_sources');
  await File(args[3]).writeAsString(
    const JsonEncoder.withIndent('  ').convert({
      'settings_revisions': revisions,
      'subscriptions_off': off,
      'subscriptions_on': on,
      'disabled_projection': disabledProjection,
      'projection': projected,
      'sources': sources,
      'tasks': tasks,
    }),
  );
  stdout.writeln(
    jsonEncode({
      'revisions': revisions,
      'tasks': tasks
          .map(
            (t) => {
              'method': t['method'],
              'error': t['error'],
              'events': (t['events'] as List).length,
            },
          )
          .toList(),
    }),
  );
}
