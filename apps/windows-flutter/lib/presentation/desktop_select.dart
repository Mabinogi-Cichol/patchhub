import 'package:flutter/material.dart';

/// A non-editable desktop selector whose menu is anchored below its field.
class DesktopSelect extends StatefulWidget {
  const DesktopSelect({
    super.key,
    required this.label,
    required this.value,
    required this.options,
    required this.onSelected,
  });

  final String label;
  final String value;
  final List<({String value, String label})> options;
  final ValueChanged<String> onSelected;

  @override
  State<DesktopSelect> createState() => _DesktopSelectState();
}

class _DesktopSelectState extends State<DesktopSelect> {
  final menu = MenuController();
  final focus = FocusNode();
  final selectedFocus = FocusNode();

  @override
  void dispose() {
    focus.dispose();
    selectedFocus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    return LayoutBuilder(
      builder: (context, constraints) => MenuAnchor(
        controller: menu,
        childFocusNode: focus,
        animated: !reducedMotion,
        crossAxisUnconstrained: false,
        reservedPadding: EdgeInsets.zero,
        onOpen: () {
          setState(() {});
          // The overlay must attach and leave its initial ExcludeFocus state.
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (mounted && menu.isOpen) selectedFocus.requestFocus();
          });
        },
        onClose: () => setState(() {}),
        alignmentOffset: const Offset(0, 8),
        style: MenuStyle(
          alignment: AlignmentDirectional.bottomStart,
          minimumSize: WidgetStatePropertyAll(Size(constraints.maxWidth, 0)),
          maximumSize: WidgetStatePropertyAll(
            Size(constraints.maxWidth, double.infinity),
          ),
          backgroundColor: WidgetStatePropertyAll(
            scheme.brightness == Brightness.dark
                ? scheme.surfaceContainerHigh
                : scheme.surface,
          ),
          surfaceTintColor: const WidgetStatePropertyAll(Colors.transparent),
          elevation: const WidgetStatePropertyAll(8),
          shape: WidgetStatePropertyAll(
            RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(10),
              side: BorderSide(color: scheme.outline),
            ),
          ),
        ),
        menuChildren: [
          for (final option in widget.options)
            MenuItemButton(
              focusNode: option.value == widget.value ? selectedFocus : null,
              onPressed: () => widget.onSelected(option.value),
              trailingIcon: option.value == widget.value
                  ? const Icon(Icons.check, size: 18)
                  : const SizedBox(width: 18),
              style: MenuItemButton.styleFrom(
                foregroundColor: scheme.onSurface,
                backgroundColor: option.value == widget.value
                    ? scheme.primaryContainer
                    : null,
                overlayColor: scheme.onSurface.withValues(alpha: 0.12),
                padding: const EdgeInsets.symmetric(
                  horizontal: 16,
                  vertical: 12,
                ),
              ),
              child: Text(option.label),
            ),
        ],
        builder: (context, controller, child) => Semantics(
          label: widget.label,
          expanded: controller.isOpen,
          child: SizedBox(
            width: constraints.maxWidth,
            child: OutlinedButton(
              key: const ValueKey('desktop-select-field'),
              focusNode: focus,
              onPressed: () =>
                  controller.isOpen ? controller.close() : controller.open(),
              style: ButtonStyle(
                foregroundColor: WidgetStatePropertyAll(scheme.onSurface),
                backgroundColor: WidgetStatePropertyAll(
                  scheme.surfaceContainer,
                ),
                padding: const WidgetStatePropertyAll(EdgeInsets.all(14)),
                shape: WidgetStatePropertyAll(
                  RoundedRectangleBorder(
                    borderRadius: BorderRadius.circular(10),
                  ),
                ),
                side: WidgetStateProperty.resolveWith(
                  (states) => BorderSide(
                    color: states.contains(WidgetState.focused)
                        ? scheme.secondary
                        : scheme.outlineVariant,
                    width: states.contains(WidgetState.focused) ? 2 : 1,
                  ),
                ),
              ),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      widget.options
                          .firstWhere((option) => option.value == widget.value)
                          .label,
                      style: Theme.of(context).textTheme.bodyLarge,
                    ),
                  ),
                  const SizedBox(width: 12),
                  AnimatedRotation(
                    turns: controller.isOpen ? 0.5 : 0,
                    duration: reducedMotion
                        ? Duration.zero
                        : const Duration(milliseconds: 180),
                    curve: Curves.easeOutCubic,
                    child: const Icon(Icons.expand_more, size: 20),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
