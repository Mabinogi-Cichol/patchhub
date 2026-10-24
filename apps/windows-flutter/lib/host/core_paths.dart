import 'dart:io';

class CorePaths {
  CorePaths({
    required this.binary,
    required this.stateDirectory,
    required this.resourceDirectory,
  });

  final String binary;
  final String stateDirectory;
  final String resourceDirectory;

  static CorePaths resolve() {
    const root = String.fromEnvironment('PATCHHUB_ROOT');
    final executableDirectory = File(Platform.resolvedExecutable).parent.path;
    final resourceDirectory = root.isEmpty
        ? '$executableDirectory/resources'
        : '$root/catalog';
    final binary = root.isEmpty
        ? '$executableDirectory/patchhub-core.exe'
        : '$root/target/debug/patchhub-core.exe';
    if (!File(binary).existsSync()) {
      throw FileSystemException('未找到核心程序，请先构建 Rust CLI', binary);
    }
    const override = String.fromEnvironment('PATCHHUB_STATE_DIR');
    final appData = Platform.environment['APPDATA'];
    if (override.isEmpty && appData == null) {
      throw StateError('无法获取应用数据目录 APPDATA');
    }
    return CorePaths(
      binary: binary,
      stateDirectory: override.isNotEmpty
          ? override
          : '$appData/PatchHub',
      resourceDirectory: resourceDirectory,
    );
  }

  String? _resource(String name) {
    final path = '$resourceDirectory/$name';
    return File(path).existsSync() ? path : null;
  }

  Map<String, Object?> get host => {
    'roots': <String>[],
    'platform': 'windows',
    'state_dir': stateDirectory,
    'catalog': null,
    'signed_catalog': _resource('signed-catalog.json'),
    'trust_keys': _resource('trust-keys.json'),
  };
}
