import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/presentation/library_presentation.dart';

void main() {
  test('Windows drive and UNC paths use familiar display forms', () {
    expect(
      displayGamePath(r'\\?\F:\SteamLibrary\steamapps\common\中文游戏'),
      r'F:\SteamLibrary\steamapps\common\中文游戏',
    );
    expect(
      displayGamePath(r'\\?\UNC\server\share\中文游戏'),
      r'\\server\share\中文游戏',
    );
  });

  test('other path forms remain unchanged', () {
    for (final path in [
      r'F:\SteamLibrary\中文游戏',
      r'\\server\share\中文游戏',
      '/home/deck/.steam/steam/steamapps/common/Game',
      r'\\?\Volume{example}\Game',
      r'\\.\device',
    ]) {
      expect(displayGamePath(path), path);
    }
  });
}
