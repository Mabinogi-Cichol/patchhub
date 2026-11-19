import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:patchhub_windows/host/core_client.dart';
import 'package:patchhub_windows/host/core_paths.dart';

void main() {
  final binary = File(
    '../../target/debug/patchhub-core${Platform.isWindows ? '.exe' : ''}',
  ).absolute;
  test(
    'actual CoreClient roundtrips prompt, auto-id scripts and rejected edits across processes',
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
      );
      final client = CoreClient(paths, protocol);
      final prompt =
          await client.call('get_script_source_prompt') as Map<String, dynamic>;
      expect(
        prompt['prompt'],
        contains('docs/custom-subscription-sources.md'),
      );
      expect(Directory(paths.stateDirectory).existsSync(), false);

      final bundled = await client.call('get_script_source', {
        'source_id': 'patchmoe',
      }) as Map;
      expect(bundled['rule']['adapter'], 'lua');
      expect(bundled['code'], contains('form_zh_cn'));
      expect(Directory(paths.stateDirectory).existsSync(), false);

      final validScript =
          '-- ${'脚本源码' * 15000}\n'
          'return {\n'
          '  meta = {\n'
          '    name = "原生客户端测试",\n'
          '    pages = {"https://example.com/patches/"},\n'
          '    page_hosts = {"example.com"},\n'
          '    download_hosts = {"dl.example.com"}\n'
          '  },\n'
          '  collect_index = function() while true do end end\n'
          '}';

      await expectLater(
        client.call('import_script_source', {
          'source_id': 'patchmoe',
          'api_version': 1,
          'code': validScript,
        }),
        throwsA(
          isA<CoreError>().having(
            (e) => e.code,
            'code',
            'source_builtin_readonly',
          ),
        ),
      );
      expect(
        (await client.call('get_script_source', {'source_id': 'patchmoe'})
            as Map)['code'],
        contains('form_zh_cn'),
      );

      final imported = await client.call('import_script_source', {
        'api_version': 1,
        'code': validScript,
      }) as Map;
      final assignedId = imported['rule']['id'] as String;
      expect(assignedId, 'example-com');
      expect(imported['name'], '原生客户端测试');
      expect(imported['rule']['script']['api_version'], 1);

      final document = await client.call('get_script_source', {
        'source_id': assignedId,
      }) as Map;
      expect(document['source_id'], assignedId);
      expect(document['name'], '原生客户端测试');
      expect(document['code'], validScript);

      for (final (badCode, expectedCode) in [
        ('return {', 'script_source_syntax_error'),
        ('return {}', 'invalid_source_meta'),
        (
          'while true do end\nreturn { meta = { pages = {"https://example.com/"} } }',
          'script_source_init_limit_exceeded',
        ),
      ]) {
        await expectLater(
          client.call('import_script_source', {
            'source_id': assignedId,
            'api_version': 1,
            'code': badCode,
          }),
          throwsA(
            isA<CoreError>().having((e) => e.code, 'code', expectedCode),
          ),
        );
      }

      final reopened = await CoreClient(
        paths,
        protocol,
      ).call('get_script_source', {'source_id': assignedId}) as Map;
      expect(reopened['code'], validScript);

      const replacementScript =
          'return {\n'
          '  meta = { pages = {"https://example.com/"} },\n'
          '  collect_index = function() error("not executed on save") end\n'
          '}';
      await client.call('import_script_source', {
        'source_id': assignedId,
        'api_version': 1,
        'code': replacementScript,
      });
      expect(
        (await client.call('get_script_source', {'source_id': assignedId})
            as Map)['code'],
        replacementScript,
      );

      await client.call('remove_source', {'source_id': assignedId});
      await expectLater(
        client.call('get_script_source', {'source_id': assignedId}),
        throwsA(
          isA<CoreError>().having((e) => e.code, 'code', 'unknown_source'),
        ),
      );
      final remaining = await client.call('list_sources', const {}) as List;
      expect(remaining.any((source) => source['id'] == assignedId), false);
    },
    skip: binary.existsSync()
        ? false
        : 'Build the Rust CLI before this local transport check',
  );
}
