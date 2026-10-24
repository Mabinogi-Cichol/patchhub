import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

class DesktopToolbar extends StatelessWidget {
  const DesktopToolbar({super.key, required this.actions});
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) {
    final brightness = Theme.of(context).brightness;
    return Material(
      color: Theme.of(context).colorScheme.surface,
      child: SizedBox(
        height: 52,
        child: Row(
          children: [
            Expanded(
              child: GestureDetector(
                onSecondaryTap: windowManager.popUpWindowMenu,
                child: DragToMoveArea(
                  child: Padding(
                    padding: const EdgeInsets.symmetric(horizontal: 20),
                    child: Row(
                      children: [
                        Image.asset(
                          'assets/branding/logo.png',
                          width: 24,
                          height: 24,
                        ),
                        const SizedBox(width: 12),
                        const Flexible(
                          child: Text(
                            'PatchHub',
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontSize: 16,
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
            ...actions,
            const SizedBox(width: 8),
            Tooltip(
              message: '最小化',
              child: WindowCaptionButton.minimize(
                brightness: brightness,
                onPressed: windowManager.minimize,
              ),
            ),
            Tooltip(
              message: '最大化或还原',
              child: WindowCaptionButton.maximize(
                brightness: brightness,
                onPressed: () async {
                  if (await windowManager.isMaximized()) {
                    await windowManager.unmaximize();
                  } else {
                    await windowManager.maximize();
                  }
                },
              ),
            ),
            Tooltip(
              message: '关闭',
              child: WindowCaptionButton.close(
                brightness: brightness,
                onPressed: windowManager.close,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
