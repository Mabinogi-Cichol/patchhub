import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_paths.dart';
import 'package:patchhub_windows/pages/script_source_examples.dart';

void main() {
  final binary = File(
    '../../target/debug/patchhub-core${Platform.isWindows ? '.exe' : ''}',
  ).absolute;
  test(
    'actual CoreClient roundtrips large scripts and rejected replacements across processes',
    () async {
      final temp = await Directory.systemTemp.createTemp(
        'patchhub-script-client-',
      );
      addTearDown(() => temp.delete(recursive: true));
      final protocol = WireProtocol(
        jsonDecode(File('assets/protocol-v5.schema.json').readAsStringSync())
            as Map<String, dynamic>,
      );
      final paths = CorePaths(
        binary: binary.path,
        stateDirectory: '${temp.path}/state',
        resourceDirectory: temp.path,
      );
      final client = CoreClient(paths, protocol);
      final rule = jsonDecode(scriptRuleExample) as Map<String, dynamic>;
      final code = '-- ${'脚本源码' * 15000}\nwhile true do end\nreturn {}';
      final imported = await client.call('import_script_source', {
        'rule': rule,
        'api_version': 1,
        'code': code,
      }) as Map;
      expect(imported['rule']['script']['api_version'], 1);
      final document = await client.call('get_script_source', {
        'source_id': rule['id'],
      }) as Map;
      expect(document['code'], code);
      await expectLater(
        client.call('import_script_source', {
          'rule': rule,
          'api_version': 1,
          'code': 'return {',
          'replace': true,
        }),
        throwsA(
          isA<CoreError>()
              .having((e) => e.code, 'code', 'script_source_syntax_error')
              .having((e) => e.detail, 'detail', contains(rule['id'])),
        ),
      );
      final reopened = await CoreClient(
        paths,
        protocol,
      ).call('get_script_source', {'source_id': rule['id']}) as Map;
      expect(reopened['code'], code);
      await client.call('import_script_source', {
        'rule': rule,
        'api_version': 1,
        'code': 'return {}',
        'replace': true,
      });
      expect(
        (await client.call('get_script_source', {'source_id': rule['id']})
            as Map)['code'],
        'return {}',
      );
    },
    skip: binary.existsSync()
        ? false
        : 'Build the Rust CLI before this local transport check',
  );
}
