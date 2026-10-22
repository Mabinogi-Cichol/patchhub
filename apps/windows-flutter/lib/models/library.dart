class GameRow {
  GameRow(this.value) {
    game = value['game'] as Map<String, dynamic>;
    patches = (value['patches'] as List).cast<Map<String, dynamic>>();
  }
  final Map<String, dynamic> value;
  late final Map<String, dynamic> game;
  late final List<Map<String, dynamic>> patches;
  String get id => game['instance_id'] as String;
  String get name {
    final localized = game['localized_name'] as String?;
    return localized != null && localized.trim().isNotEmpty
        ? localized.trim()
        : game['name'] as String;
  }

  Map<String, dynamic>? get discovery =>
      value['discovery'] as Map<String, dynamic>?;
  bool get matched =>
      patches.isNotEmpty ||
      ((discovery?['candidates'] as List?) ?? []).any(
        (candidate) => [
          'appid',
          'verified_alias',
          'official_title',
        ].contains(candidate['match_basis']),
      );
}

class ScanSnapshot {
  ScanSnapshot(Map<String, dynamic> value)
    : rows = (value['installs'] as List)
          .map((row) => GameRow(row as Map<String, dynamic>))
          .toList(),
      diagnostics = (value['diagnostics'] as List).cast<Map<String, dynamic>>(),
      catalogError = value['catalog_error'] as Map<String, dynamic>?;
  final List<GameRow> rows;
  final List<Map<String, dynamic>> diagnostics;
  final Map<String, dynamic>? catalogError;
}
