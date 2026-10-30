import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'script_source_examples.dart';

class ScriptSourceEditor extends StatefulWidget {
  const ScriptSourceEditor({super.key, this.document, required this.onSave});
  final Map<String, dynamic>? document;
  final Future<void> Function(Map<String, Object?> params) onSave;

  @override
  State<ScriptSourceEditor> createState() => _ScriptSourceEditorState();
}

class _ScriptSourceEditorState extends State<ScriptSourceEditor> {
  late final rule = TextEditingController(
    text: widget.document == null
        ? ''
        : const JsonEncoder.withIndent('  ').convert(widget.document!['rule']),
  );
  late final code = TextEditingController(
    text: widget.document?['code'] as String? ?? '',
  );
  late final api = TextEditingController(
    text: '${widget.document?['api_version'] ?? 1}',
  );
  bool saving = false;
  bool example = false;
  String? error;
  String? copied;
  String? get target => widget.document?['rule']['id'] as String?;

  @override
  void dispose() {
    rule.dispose();
    code.dispose();
    api.dispose();
    super.dispose();
  }

  Future<void> save() async {
    setState(() {
      saving = true;
      error = null;
    });
    try {
      final value = jsonDecode(rule.text);
      if (value is! Map<String, dynamic>) {
        throw const FormatException('规则必须是 JSON 对象');
      }
      if (target != null && value['id'] != target) {
        throw FormatException('替换时来源 ID 必须为 $target');
      }
      final version = int.tryParse(api.text);
      if (version == null) throw const FormatException('API 版本必须是整数');
      await widget.onSave({
        'rule': value,
        'api_version': version,
        'code': code.text,
        'replace': target != null,
      });
      if (mounted) Navigator.of(context).pop(true);
    } catch (failure) {
      if (mounted) setState(() => error = '$failure');
    } finally {
      if (mounted) setState(() => saving = false);
    }
  }

  Future<void> copy(String value, String label) async {
    try {
      await Clipboard.setData(ClipboardData(text: value));
      if (mounted) setState(() => copied = '$label 已复制，请修改后导入');
    } catch (failure) {
      if (mounted) setState(() => error = '复制失败：$failure');
    }
  }

  @override
  Widget build(BuildContext context) => PopScope(
    canPop: !saving,
    child: AlertDialog(
      title: Text(target == null ? '导入脚本来源' : '编辑脚本：$target'),
      content: SizedBox(
        width: 760,
        child: SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const Text(
                '仅导入你信任的 Lua 脚本。网络请求限于规则声明的域名。保存仅检查语法，不执行脚本；保存后需更新来源或重新匹配游戏。',
              ),
              if (target != null) ...[
                const SizedBox(height: 12),
                SelectableText(
                  '当前 API ${widget.document!['api_version']} · SHA-256 ${widget.document!['sha256']}',
                ),
                const Text('替换同 ID 来源，保留名称覆盖、启用、订阅及更新间隔。'),
              ],
              const SizedBox(height: 16),
              TextField(
                key: const ValueKey('script-rule'),
                controller: rule,
                enabled: !saving,
                minLines: 4,
                maxLines: 8,
                style: const TextStyle(fontFamily: 'monospace'),
                decoration: const InputDecoration(
                  labelText: '脚本规则 JSON',
                  border: OutlineInputBorder(),
                ),
              ),
              const SizedBox(height: 16),
              TextField(
                key: const ValueKey('script-api'),
                controller: api,
                enabled: !saving,
                keyboardType: TextInputType.number,
                decoration: const InputDecoration(
                  labelText: 'Lua API 版本',
                  helperText: '当前核心支持 API 1',
                ),
              ),
              const SizedBox(height: 16),
              TextField(
                key: const ValueKey('script-code'),
                controller: code,
                enabled: !saving,
                minLines: 6,
                maxLines: 12,
                style: const TextStyle(fontFamily: 'monospace'),
                decoration: const InputDecoration(
                  labelText: 'Lua 源码',
                  border: OutlineInputBorder(),
                ),
              ),
              if (error != null)
                Padding(
                  padding: const EdgeInsets.only(top: 12),
                  child: SelectableText(
                    error!,
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.error,
                    ),
                  ),
                ),
              TextButton(
                onPressed: () => setState(() => example = !example),
                child: Text(example ? '收起脚本示例' : '查看脚本示例'),
              ),
              if (example) ...[
                const Text('示例只用于展示和复制，不会更改当前输入。请替换 ID、名称、域名及选择器。'),
                const SelectableText(
                  scriptRuleExample,
                  style: TextStyle(fontFamily: 'monospace'),
                ),
                TextButton(
                  onPressed: () => copy(scriptRuleExample, '规则示例'),
                  child: const Text('复制规则示例'),
                ),
                const SelectableText(
                  scriptCodeExample,
                  style: TextStyle(fontFamily: 'monospace'),
                ),
                TextButton(
                  onPressed: () => copy(scriptCodeExample, '源码示例'),
                  child: const Text('复制源码示例'),
                ),
                if (copied != null) Text(copied!),
              ],
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: saving ? null : () => Navigator.of(context).pop(),
          child: const Text('取消'),
        ),
        FilledButton(
          onPressed: saving ? null : save,
          child: Text(
            saving
                ? '正在保存…'
                : target == null
                ? '保存并导入'
                : '保存并替换 $target',
          ),
        ),
      ],
    ),
  );
}
