import 'dart:io';

class CorePaths {
  CorePaths({required this.binary, required this.stateDirectory});

  final String binary;
  final String stateDirectory;

  static CorePaths resolve() {
    const root = String.fromEnvironment('PATCHHUB_ROOT');
    final executableDirectory = File(Platform.resolvedExecutable).parent.path;
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
      stateDirectory: override.isNotEmpty ? override : '$appData/PatchHub',
    );
  }

  Map<String, Object?> get host => {
    'roots': <String>[],
    'platform': 'windows',
    'state_dir': stateDirectory,
  };
}
