import 'dart:convert';
import 'dart:io';

import 'package:steam_patch_windows/host/core_client.dart';
import 'package:steam_patch_windows/host/core_paths.dart';
import 'package:steam_patch_windows/models/library.dart';

Future<void> main(List<String> args) async {
  if (args.length != 3) {
    throw ArgumentError('core_probe <core.exe> <state-copy> <resources>');
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
  final capabilities = await client.capabilities();
  final scan = await client.call('scan') as Map<String, dynamic>;
  final snapshot = ScanSnapshot(scan);
  final transactions = await client.call('list_transactions') as List;
  final sources = await client.call('list_sources') as List;
  stdout.writeln(
    jsonEncode({
      'protocol': capabilities['protocol_version'],
      'installs': snapshot.rows.length,
      'scan': scan,
      'transactions': transactions,
      'sources': sources,
      'state_directory': client.paths.stateDirectory,
    }),
  );
}
