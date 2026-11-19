import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

class ScriptSourceEditor extends StatefulWidget {
  const ScriptSourceEditor({
    super.key,
    this.document,
    this.readOnly = false,
    this.onLoadPrompt,
    required this.onSave,
  });

  final Map<String, dynamic>? document;

  /// 内置来源随版本更新，只展示只读摘要与源码。
  final bool readOnly;
  final Future<String> Function()? onLoadPrompt;
  final Future<void> Function(Map<String, Object?> params) onSave;

  @override
  State<ScriptSourceEditor> createState() => _ScriptSourceEditorState();
}

class _ScriptSourceEditorState extends State<ScriptSourceEditor> {
  late final Map<String, dynamic> original =
      widget.document?['rule'] as Map<String, dynamic>? ?? {};
  late final TextEditingController codeController = TextEditingController(
    text: widget.document?['code'] as String? ?? '',
  );

  bool saving = false;
  bool loadingPrompt = false;
  String? error;
  bool copiedCode = false;
  bool copiedPrompt = false;

  String? get target =>
      (widget.document?['source_id'] ?? original['id']) as String?;

  String get displayName =>
      (widget.document?['name'] ?? target ?? '').toString();

  String get roleLabel =>
      original['role'] == 'fallback_indexer' ? '补查源' : '索引源';

  String hostList(String key) {
    final values = (original[key] as List<dynamic>? ?? const [])
        .map((item) => item.toString())
        .where((item) => item.isNotEmpty)
        .toList();
    return values.isEmpty ? '无' : values.join(' · ');
  }

  @override
  void dispose() {
    codeController.dispose();
    super.dispose();
  }

  Future<void> copyPrompt() async {
    final loader = widget.onLoadPrompt;
    if (loader == null) return;
    setState(() {
      loadingPrompt = true;
      error = null;
      copiedPrompt = false;
    });
    try {
      final prompt = await loader();
      await Clipboard.setData(ClipboardData(text: prompt));
      if (mounted) setState(() => copiedPrompt = true);
    } catch (failure) {
      if (mounted) setState(() => error = '复制 AI 提示词失败：$failure');
    } finally {
      if (mounted) setState(() => loadingPrompt = false);
    }
  }

  Future<void> save() async {
    setState(() {
      saving = true;
      error = null;
      copiedPrompt = false;
    });
    try {
      await widget.onSave({
        'source_id': ?target,
        'api_version': widget.document?['api_version'] ?? 1,
        'code': codeController.text,
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
      await Clipboard.setData(ClipboardData(text: codeController.text));
      if (mounted) setState(() => copiedCode = true);
    } catch (failure) {
      if (mounted) setState(() => error = '复制失败：$failure');
    }
  }

  @override
  Widget build(BuildContext context) {
    final isCreate = target == null && !widget.readOnly;
    return PopScope(
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
                Text(
                  widget.readOnly
                      ? '内置来源随版本更新，只读查看。'
                      : isCreate
                      ? '仅导入你信任的 Lua 脚本。可先复制 AI 提示词发给 AI 工具生成脚本，再粘贴完整 Lua 源码保存。'
                      : '仅导入你信任的 Lua 脚本。替换代码会保留当前来源 ID、名称及订阅设置。',
                ),
                const SizedBox(height: 12),
                if (isCreate && widget.onLoadPrompt != null) ...[
                  Align(
                    alignment: Alignment.centerLeft,
                    child: OutlinedButton.icon(
                      key: const ValueKey('script-copy-prompt'),
                      onPressed: saving || loadingPrompt ? null : copyPrompt,
                      icon: const Icon(Icons.copy_all, size: 18),
                      label: Text(loadingPrompt ? '正在读取提示词…' : '复制 AI 提示词'),
                    ),
                  ),
                  if (copiedPrompt) ...[
                    const SizedBox(height: 6),
                    const Text('AI 提示词已复制，请粘贴发送给你的 AI 助手。'),
                  ],
                  const SizedBox(height: 12),
                ],
                if (target != null) ...[
                  Container(
                    padding: const EdgeInsets.all(12),
                    margin: const EdgeInsets.only(bottom: 12),
                    decoration: BoxDecoration(
                      border: Border.all(
                        color: Theme.of(context).colorScheme.outlineVariant,
                      ),
                      borderRadius: BorderRadius.circular(8),
                    ),
                    child: SelectableText(
                      '来源 ID：$target\n'
                      '显示名称：$displayName\n'
                      '来源类型：$roleLabel\n'
                      '允许访问域名：${hostList('page_hosts')}\n'
                      '允许下载域名：${hostList('download_hosts')}',
                      key: const ValueKey('script-summary'),
                    ),
                  ),
                ],
                TextField(
                  key: const ValueKey('script-code'),
                  controller: codeController,
                  enabled: !saving && !widget.readOnly,
                  readOnly: saving || widget.readOnly,
                  minLines: 12,
                  maxLines: 20,
                  style: const TextStyle(fontFamily: 'monospace'),
                  decoration: InputDecoration(
                    labelText: 'Lua 源码',
                    hintText: isCreate ? '粘贴包含 meta 表的完整 Lua 脚本…' : null,
                    border: const OutlineInputBorder(),
                  ),
                ),
                if (error != null) ...[
                  const SizedBox(height: 10),
                  SelectableText(
                    error!,
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.error,
                    ),
                  ),
                ],
                if (copiedCode) ...[
                  const SizedBox(height: 8),
                  const Text('Lua 源码已复制'),
                ],
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
}
