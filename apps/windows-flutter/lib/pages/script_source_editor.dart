import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

class ScriptSourceEditor extends StatefulWidget {
  const ScriptSourceEditor({
    super.key,
    this.document,
    this.readOnly = false,
    required this.onSave,
  });
  final Map<String, dynamic>? document;

  /// 内置来源随版本更新，只展示规则与源码。
  final bool readOnly;
  final Future<void> Function(Map<String, Object?> params) onSave;

  @override
  State<ScriptSourceEditor> createState() => _ScriptSourceEditorState();
}

class _ScriptSourceEditorState extends State<ScriptSourceEditor> {
  static const optionalStrings = {
    'url': '起始 URL',
    'language': '语言',
    'table_id': '表格 ID',
  };
  static const listFields = {
    'pages': '起始页面（必填，每行一项）',
    'page_hosts': '页面域名（必填，每行一项）',
    'download_hosts': '下载域名（每行一项）',
  };
  static const columnFields = {
    'title': '标题列',
    'download': '下载列',
    'date': '日期列',
    'language': '语言列',
    'appid': 'Steam AppID 列',
  };

  late final original = widget.document?['rule'] as Map<String, dynamic>? ?? {};
  late final controllers = <String, TextEditingController>{
    for (final key in ['id', 'name', 'priority', ...optionalStrings.keys])
      key: TextEditingController(text: original[key]?.toString() ?? ''),
    for (final key in listFields.keys)
      key: TextEditingController(
        text: (original[key] as List<dynamic>?)?.join('\n') ?? '',
      ),
    for (final key in columnFields.keys)
      'column-$key': TextEditingController(
        text: (original['columns'] as Map?)?[key]?.toString() ?? '',
      ),
    'code': TextEditingController(
      text: widget.document?['code'] as String? ?? '',
    ),
  };
  late final presentStrings = {
    for (final key in optionalStrings.keys)
      if (original[key] != null) key,
  };
  late String role = original['role'] as String? ?? 'publisher';
  late bool discoveryOnly = original['discovery_only'] as bool? ?? false;
  late bool enabled = original['enabled'] as bool? ?? true;
  late bool useColumns = original['columns'] != null;
  bool saving = false;
  String? error;
  bool copied = false;
  String? get target => original['id'] as String?;
  bool get disabled => saving || widget.readOnly;
  bool get discovery => discoveryOnly || role == 'fallback_indexer';

  @override
  void dispose() {
    for (final controller in controllers.values) {
      controller.dispose();
    }
    super.dispose();
  }

  String value(String key) => controllers[key]!.text;

  List<String> lines(String key) =>
      value(key)
          .split('\n')
          .map((line) => line.trim())
          .where((line) => line.isNotEmpty)
          .toList();

  int? integer(String key, String label, {bool required = false}) {
    final text = value(key).trim();
    if (text.isEmpty && !required) return null;
    return int.tryParse(text, radix: 10) ??
        (throw FormatException('$label必须是整数'));
  }

  Future<void> save() async {
    setState(() {
      saving = true;
      error = null;
    });
    try {
      final downloads = lines('download_hosts');
      final priority = integer('priority', '优先级');
      await widget.onSave({
        'rule': <String, Object?>{
          'id': target ?? value('id'),
          'name': value('name'),
          'adapter': 'lua',
          'role': role,
          'discovery_only': discovery,
          if (!discovery) 'root_layout': 'game_root',
          'priority': ?priority,
          'enabled': enabled,
          'pages': lines('pages'),
          'page_hosts': lines('page_hosts'),
          if (downloads.isNotEmpty || original['download_hosts'] != null)
            'download_hosts': downloads,
          for (final key in presentStrings) key: value(key),
          if (useColumns)
            'columns': <String, int?>{
              for (final entry in columnFields.entries)
                entry.key: integer(
                  'column-${entry.key}',
                  entry.value,
                  required: entry.key == 'title' || entry.key == 'download',
                ),
            },
        },
        'api_version': widget.document?['api_version'] ?? 1,
        'code': value('code'),
        'replace': target != null,
      });
      if (mounted) Navigator.of(context).pop(true);
    } catch (failure) {
      if (mounted) setState(() => error = '$failure');
    } finally {
      if (mounted) setState(() => saving = false);
    }
  }

  Future<void> copyCode() async {
    try {
      await Clipboard.setData(ClipboardData(text: value('code')));
      if (mounted) setState(() => copied = true);
    } catch (failure) {
      if (mounted) setState(() => error = '复制失败：$failure');
    }
  }

  Widget field(String key, String label, {bool locked = false, int rows = 1}) =>
      Padding(
        padding: const EdgeInsets.only(bottom: 12),
        child: TextField(
          key: ValueKey('script-$key'),
          controller: controllers[key],
          enabled: !disabled && !locked,
          readOnly: disabled || locked,
          minLines: rows,
          maxLines: rows == 1 ? 1 : rows * 2,
          style: key == 'code'
              ? const TextStyle(fontFamily: 'monospace')
              : null,
          decoration: InputDecoration(
            labelText: label,
            border: const OutlineInputBorder(),
          ),
        ),
      );

  @override
  Widget build(BuildContext context) => PopScope(
    canPop: !saving,
    child: AlertDialog(
      title: Text(
        widget.readOnly
            ? '查看脚本：$target'
            : target == null
            ? '导入脚本来源'
            : '编辑脚本：$target',
      ),
      content: SizedBox(
        width: 760,
        child: SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(widget.readOnly ? '内置来源随版本更新，只读查看。' : '仅导入你信任的 Lua 脚本。'),
              const SizedBox(height: 12),
              field('id', '来源 ID（必填）', locked: target != null),
              field('name', '规则名称（必填）'),
              DropdownButtonFormField<String>(
                initialValue: role,
                decoration: const InputDecoration(labelText: '角色'),
                items: const [
                  DropdownMenuItem(value: 'publisher', child: Text('索引源')),
                  DropdownMenuItem(
                    value: 'fallback_indexer',
                    child: Text('补查源'),
                  ),
                ],
                onChanged: disabled
                    ? null
                    : (value) => setState(() => role = value!),
              ),
              SwitchListTile(
                title: const Text('仅发现'),
                value: discovery,
                onChanged: disabled || role == 'fallback_indexer'
                    ? null
                    : (value) => setState(() => discoveryOnly = value),
              ),
              ListTile(
                title: const Text('安装布局'),
                subtitle: Text(discovery ? '无' : '游戏根目录（game_root）'),
              ),
              field('priority', '优先级（默认 100）'),
              SwitchListTile(
                title: const Text('启用'),
                value: enabled,
                onChanged: disabled || target != null
                    ? null
                    : (value) => setState(() => enabled = value),
              ),
              for (final entry in listFields.entries)
                field(entry.key, entry.value, rows: 2),
              for (final entry in optionalStrings.entries) ...[
                CheckboxListTile(
                  title: Text('设置${entry.value}'),
                  value: presentStrings.contains(entry.key),
                  onChanged: disabled
                      ? null
                      : (checked) => setState(() {
                          if (checked == true) {
                            presentStrings.add(entry.key);
                          } else {
                            presentStrings.remove(entry.key);
                          }
                        }),
                ),
                if (presentStrings.contains(entry.key))
                  field(entry.key, entry.value),
              ],
              CheckboxListTile(
                title: const Text('设置表格列（从 0 开始）'),
                value: useColumns,
                onChanged: disabled
                    ? null
                    : (value) => setState(() => useColumns = value ?? false),
              ),
              if (useColumns)
                for (final entry in columnFields.entries)
                  field('column-${entry.key}', entry.value),
              field('code', 'Lua 源码', rows: 6),
              if (error != null)
                SelectableText(
                  error!,
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
              if (copied) const Text('Lua 源码已复制'),
            ],
          ),
        ),
      ),
      actions: [
        if (widget.readOnly)
          TextButton(onPressed: copyCode, child: const Text('复制 Lua 源码')),
        TextButton(
          onPressed: saving ? null : () => Navigator.of(context).pop(),
          child: Text(widget.readOnly ? '关闭' : '取消'),
        ),
        if (!widget.readOnly)
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
