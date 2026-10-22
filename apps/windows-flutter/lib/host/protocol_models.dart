import 'dart:convert';

class CoreError implements Exception {
  const CoreError(this.code, [this.detail]);
  final String code;
  final String? detail;
  @override
  String toString() => detail == null ? code : '$code: $detail';
}

const coreMethods = [
  'get_capabilities',
  'scan',
  'project_instances',
  'preview',
  'install_patch',
  'list_transactions',
  'recover',
  'discard_plan',
  'cleanup_cache',
  'cache_status',
  'backup_status',
  'cleanup_backups',
  'list_sources',
  'list_game_sources',
  'set_source_enabled',
  'set_game_source',
  'add_source',
  'set_source_update_interval',
  'update_source',
  'match_games',
  'rematch_game',
];

void validateCapabilities(Map<String, dynamic> value) {
  try {
    _validateCapabilities(value);
  } on CoreError {
    rethrow;
  } on TypeError {
    throw const CoreError('capability_mismatch');
  } on NoSuchMethodError {
    throw const CoreError('capability_mismatch');
  }
}

void _validateCapabilities(Map<String, dynamic> value) {
  if (value['protocol_version'] != 5) {
    throw const CoreError('unsupported_protocol');
  }
  bool equal(Object? left, Object? right) =>
      jsonEncode(left) == jsonEncode(right);
  if (!equal(value['methods'], coreMethods) ||
      !equal(value['sources']['methods'], coreMethods.sublist(12))) {
    throw const CoreError('capability_mismatch');
  }
  final contracts = value['contracts'] as Map<String, dynamic>;
  for (final key in [
    'projection_version',
    'actions_version',
    'outcome_version',
    'request_validation_version',
  ]) {
    if (contracts[key] != 1) throw const CoreError('capability_mismatch');
  }
  final installer = value['installer'] as Map<String, dynamic>;
  final sources = value['sources'] as Map<String, dynamic>;
  for (final key in [
    'schema_version',
    'selection_version',
    'content_version',
    'adapter_version',
    'policy_schema_version',
    'report_schema_version',
  ]) {
    if (installer[key] != 1) {
      throw const CoreError('installer_schema_unsupported');
    }
  }
  for (final key in [
    'index_schema_version',
    'detail_schema_version',
    'match_schema_version',
    'schedule_schema_version',
    'adapter_version',
    'encoding_version',
  ]) {
    if (sources[key] != 1) {
      throw const CoreError('installer_schema_unsupported');
    }
  }
  if (sources['parser_version'] != 2) {
    throw const CoreError('installer_schema_unsupported');
  }
  if (installer['formats']['zip'] != 'supported' ||
      installer['formats']['exe'] != 'supported' ||
      !equal(installer['strategies'], [
        'managed_zip',
        'publisher_zip',
        'publisher_exe',
      ])) {
    throw const CoreError('capability_mismatch');
  }
  for (final format in ['zip', 'exe']) {
    final link = installer['links'][format];
    if (link['available'] != (link['blockers'] as List).isEmpty) {
      throw const CoreError('capability_mismatch');
    }
  }
}

Object? decodeResponse(String line, String requestId) {
  final value = jsonDecode(line);
  if (value is! Map<String, dynamic>) {
    throw const CoreError('invalid_core_response');
  }
  if (value['protocol_version'] != 5) {
    throw const CoreError('unsupported_protocol');
  }
  if (value['request_id'] != requestId || value.length != 4) {
    throw const CoreError('invalid_core_response');
  }
  if (value['ok'] == false && value['error'] is Map<String, dynamic>) {
    final error = value['error'] as Map<String, dynamic>;
    if (error['code'] is! String ||
        (error['detail'] != null && error['detail'] is! String) ||
        error.keys.any((key) => key != 'code' && key != 'detail')) {
      throw const CoreError('invalid_core_response');
    }
    throw CoreError(error['code'] as String, error['detail'] as String?);
  }
  if (value['ok'] != true || !value.containsKey('result')) {
    throw const CoreError('invalid_core_response');
  }
  return value['result'];
}

/// Interprets the enriched schema from crates/protocol, using the same subset
/// and field constraints as its matches_schema function. This is not a second DTO graph.
class WireProtocol {
  WireProtocol(this.schema);
  final Map<String, dynamic> schema;
  static const results = {
    'get_capabilities': 'Capabilities',
    'scan': 'ScanReport',
    'project_instances': 'ProjectInstancesResult',
    'preview': 'PreviewReport',
    'install_patch': 'InstallerOutcome',
    'list_transactions': 'Transactions',
    'recover': 'Transactions',
    'discard_plan': 'Transaction',
    'cleanup_cache': 'CleanupResult',
    'cache_status': 'CacheInventory',
    'backup_status': 'BackupInventory',
    'cleanup_backups': 'BackupCleanupResult',
    'list_sources': 'Sources',
    'list_game_sources': 'GameSources',
    'set_source_enabled': 'SourceEnabledResult',
    'set_game_source': 'GameSourceResult',
    'add_source': 'AddSourceResult',
    'set_source_update_interval': 'SetSourceIntervalResult',
    'update_source': 'SourceUpdateResult',
    'match_games': 'MatchGamesResult',
    'rematch_game': 'MatchGamesResult',
  };

  static bool equal(dynamic a, dynamic b) {
    if (a is List && b is List) {
      return a.length == b.length &&
          Iterable.generate(a.length).every((i) => equal(a[i], b[i]));
    }
    if (a is Map && b is Map) {
      return a.length == b.length &&
          a.keys.every((k) => b.containsKey(k) && equal(a[k], b[k]));
    }
    return a == b;
  }

  bool matches(dynamic value, dynamic rule) {
    if (rule is bool) return rule;
    if (rule[r'$ref'] case final String ref) {
      dynamic target = schema;
      for (final token in ref.substring(2).split('/')) {
        target = target[token.replaceAll('~1', '/').replaceAll('~0', '~')];
      }
      return matches(value, target);
    }
    if (rule.containsKey('const') && !equal(value, rule['const'])) return false;
    if (rule['enum'] case final List values) {
      if (!values.any((item) => equal(value, item))) return false;
    }
    for (final name in ['anyOf', 'oneOf', 'allOf']) {
      if (rule[name] case final List variants) {
        final count = variants
            .where((variant) => matches(value, variant))
            .length;
        if ((name == 'anyOf' && count == 0) ||
            (name == 'oneOf' && count != 1) ||
            (name == 'allOf' && count != variants.length)) {
          return false;
        }
      }
    }
    final type = rule['type'];
    if (type != null) {
      bool valid(dynamic t) => switch (t) {
        'object' => value is Map,
        'array' => value is List,
        'string' => value is String,
        'boolean' => value is bool,
        'null' => value == null,
        'integer' => value is int,
        'number' => value is num,
        _ => false,
      };
      if (type is List ? !type.any(valid) : !valid(type)) return false;
    }
    if (value is num) {
      if (rule['minimum'] is num && value < rule['minimum']) return false;
      if (rule['maximum'] is num && value > rule['maximum']) return false;
      if (rule['format'] == 'uint32' && (value < 0 || value > 4294967295)) {
        return false;
      }
    }
    if (value is String && rule['x-max-utf8-bytes'] is int) {
      if (utf8.encode(value).length > rule['x-max-utf8-bytes']) return false;
    }
    if (value is Map) {
      if ((rule['required'] as List? ?? []).any(
        (key) => !value.containsKey(key),
      )) {
        return false;
      }
      final properties = rule['properties'] as Map? ?? {};
      for (final key in value.keys) {
        if (properties.containsKey(key)) {
          if (!matches(value[key], properties[key])) return false;
        } else if (rule['additionalProperties'] == false) {
          return false;
        }
      }
    }
    if (value is List) {
      if (rule['minItems'] is int && value.length < rule['minItems']) {
        return false;
      }
      if (rule['maxItems'] is int && value.length > rule['maxItems']) {
        return false;
      }
      if (rule.containsKey('items') &&
          !value.every((item) => matches(item, rule['items']))) {
        return false;
      }
    }
    return true;
  }

  void bounds(dynamic value) {
    if (value is num && (value is! int || value.abs() > 9007199254740991)) {
      throw const CoreError('invalid_core_response');
    }
    if (value is List) {
      for (final item in value) {
        bounds(item);
      }
    }
    if (value is Map) {
      for (final entry in value.entries) {
        final key = entry.key;
        final item = entry.value;
        if (key == 'code' &&
            item is String &&
            !RegExp(r'^[a-z0-9_]{1,128}$').hasMatch(item)) {
          throw const CoreError('invalid_core_response');
        }
        if ([
              'id',
              'request_id',
              'instance_id',
              'release_id',
              'selection_id',
              'plan_id',
              'source_id',
              'transaction_id',
              'content_id',
              'candidate_id',
            ].contains(key) &&
            item is String) {
          final limit = key == 'request_id'
              ? 128
              : key == 'instance_id'
              ? 16384
              : 256;
          if (item.isEmpty ||
              utf8.encode(item).length > limit ||
              item.runes.any((c) => c < 32 || (c >= 127 && c <= 159))) {
            throw const CoreError('invalid_core_response');
          }
        }
        bounds(item);
      }
    }
  }

  dynamic response(dynamic value, String id, String method) {
    if (value is! Map<String, dynamic>) {
      throw const CoreError('invalid_core_response');
    }
    if (value['protocol_version'] != 5) {
      throw const CoreError('unsupported_protocol');
    }
    if (value['request_id'] != id || value.length != 4) {
      throw const CoreError('invalid_core_response');
    }
    bounds(value);
    if (value['ok'] == false) {
      if (!matches(value, schema[r'$defs']['ErrorResponse'])) {
        throw const CoreError('invalid_core_response');
      }
      throw CoreError(
        value['error']['code'] as String,
        value['error']['detail'] as String?,
      );
    }
    if (value['ok'] != true || !value.containsKey('result')) {
      throw const CoreError('invalid_core_response');
    }
    final result = value['result'];
    if (method == 'get_capabilities') {
      validateCapabilities(result as Map<String, dynamic>);
    }
    if (!matches(result, schema[r'$defs'][results[method]])) {
      throw const CoreError('invalid_core_response');
    }
    return result;
  }

  void event(Map<String, dynamic> value, String id) {
    if (value['protocol_version'] != 5) {
      throw const CoreError('unsupported_protocol');
    }
    if (value['request_id'] != id) {
      throw const CoreError('invalid_core_response');
    }
    bounds(value);
    if (!matches(value, schema[r'$defs']['Event'])) {
      throw const CoreError('invalid_core_response');
    }
    // Required nullable telemetry fields are mandated by validate_event, even
    // where JSON schema marks them as optional to match serde Option<T>.
    final fields = switch (value['event']) {
      'install_progress' => [
        'instance_id',
        'selection_id',
        'substage',
        'total_bytes',
        'total_files',
        'current_file',
        'archive_bytes',
      ],
      'match_update' => ['instance_id'],
      _ => <String>[],
    };
    if (fields.any((key) => !value.containsKey(key))) {
      throw const CoreError('invalid_core_response');
    }
  }
}
