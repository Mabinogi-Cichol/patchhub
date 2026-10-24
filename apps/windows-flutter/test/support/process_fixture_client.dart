import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_paths.dart';

String dartExecutable() {
  final config = File('.dart_tool/package_config.json').absolute;
  final packages = jsonDecode(config.readAsStringSync())['packages'] as List;
  final flutter = packages.singleWhere(
    (package) => package['name'] == 'flutter',
  );
  final sdk = Directory.fromUri(
    config.uri.resolve(flutter['rootUri'] as String),
  ).parent.parent;
  return '${sdk.path}/bin/cache/dart-sdk/bin/dart${Platform.isWindows ? '.exe' : ''}';
}

class ProcessFixtureClient extends CoreClient {
  ProcessFixtureClient(this.scenario, {this.gate})
    : super(
        CorePaths(binary: '', stateDirectory: '', resourceDirectory: ''),
        WireProtocol(
          jsonDecode(File('assets/protocol-v5.schema.json').readAsStringSync())
              as Map<String, dynamic>,
        ),
      );
  final String scenario;
  final Completer<void>? gate;
  @override
  Future<Map<String, dynamic>> capabilities() async {
    await gate?.future;
    return super.capabilities();
  }

  @override
  Future<Process> startProcess(String channel) => Process.start(
    dartExecutable(),
    [File('test/support/fake_core.dart').absolute.path, channel, scenario],
  );
}
