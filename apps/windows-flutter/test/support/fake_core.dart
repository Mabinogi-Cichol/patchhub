import 'dart:async';
import 'dart:convert';
import 'dart:io';

Future<void> main(List<String> args) async {
  final channel = args[0];
  final scenario = args[1];
  final corpus = jsonDecode(
    File('../../packages/contracts/fixtures/v5/cases.json').readAsStringSync(),
  );
  final input = StreamIterator(
    stdin.transform(utf8.decoder).transform(const LineSplitter()),
  );
  if (!await input.moveNext()) exit(1);
  final request = jsonDecode(input.current);
  final id = request['request_id'];
  final method = request['method'];
  Map<String, dynamic> response(String name) => {
    ...((corpus['responses'] as List).firstWhere(
          (c) => c['name'] == name,
        )['response']
        as Map<String, dynamic>),
    'request_id': id,
  };
  if (channel == 'serve') {
    stdout.writeln(jsonEncode(response('$method-success')));
    await stdout.flush();
    exit(0);
  }
  if (scenario == 'missing') exit(0);
  final eventName =
      ['install_patch', 'scan', 'project_instances', 'preview'].contains(method)
      ? 'install_progress'
      : 'match_progress';
  final event = {
    ...((corpus['events'] as List).firstWhere(
          (c) => c['name'] == eventName,
        )['event']
        as Map<String, dynamic>),
    'request_id': id,
  };
  Future<void> emit(dynamic value) async {
    final bytes = utf8.encode(
      '${value is String ? value : jsonEncode(value)}\n',
    );
    if (scenario == 'chunked') {
      for (var offset = 0; offset < bytes.length; offset += 3) {
        stdout.add(
          bytes.sublist(
            offset,
            offset + 3 > bytes.length ? bytes.length : offset + 3,
          ),
        );
        await stdout.flush();
      }
    } else {
      stdout.add(bytes);
      await stdout.flush();
    }
  }

  if (scenario == 'stderr') {
    stderr.write(List.filled(1024 * 1024, 'x').join());
    await stderr.flush();
  }
  if (scenario == 'gap') event['sequence'] = 1;
  if (scenario == 'wrong_event') {
    event['event'] = 'source_progress';
    event['source_id'] = 'fixture';
  }
  if (scenario == 'chunked' && method == 'install_patch') {
    event['current_file'] = '中文/补丁.dat';
  }
  await emit(event);
  if (scenario == 'duplicate') {
    await emit(event);
    await emit({...event, 'sequence': 1});
  }
  if ([
    'cancel_wait',
    'recover_wait',
    'failed_wait',
    'gap',
    'wrong_event',
  ].contains(scenario)) {
    var count = 0;
    while (await input.moveNext()) {
      if (input.current == 'cancel') {
        count++;
        break;
      }
    }
    stderr.writeln('cancel_count=$count');
    if (method == 'install_patch') {
      await emit(
        response(
          scenario == 'recover_wait'
              ? 'outcome-needs_recovery-failed'
              : scenario == 'failed_wait'
              ? 'outcome-failed-failed'
              : 'outcome-rolled_back-cancelled',
        ),
      );
    } else {
      await emit({
        'protocol_version': 5,
        'request_id': id,
        'ok': false,
        'error': {'code': 'task_cancelled'},
      });
    }
    exit(0);
  }
  if (scenario == 'no_terminal') exit(7);
  if (scenario == 'unterminated') {
    stdout.write(jsonEncode(response('$method-success')));
    await stdout.flush();
    exit(0);
  }
  await emit(response('$method-success'));
  if (scenario == 'late_noise') {
    await emit('not json');
    await emit(response('$method-success'));
  }
  await stdout.flush();
  exit(scenario == 'terminal_nonzero' ? 7 : 0);
}
