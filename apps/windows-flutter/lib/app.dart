import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

class SteamPatchApp extends StatefulWidget {
  const SteamPatchApp({super.key, required this.home});
  final Widget home;

  static ThemeMode themeModeOf(BuildContext context) =>
      context.findAncestorStateOfType<_SteamPatchAppState>()!.themeMode;

  static void setThemeMode(BuildContext context, ThemeMode mode) => context
      .findAncestorStateOfType<_SteamPatchAppState>()!
      .setThemeMode(mode);

  @override
  State<SteamPatchApp> createState() => _SteamPatchAppState();
}

class _SteamPatchAppState extends State<SteamPatchApp> {
  ThemeMode themeMode = ThemeMode.system;

  void setThemeMode(ThemeMode mode) {
    setState(() => themeMode = mode);
  }

  ThemeData theme(Brightness brightness) => ThemeData(
    useMaterial3: true,
    colorScheme: ColorScheme.fromSeed(
      seedColor: const Color(0xff42685e),
      brightness: brightness,
    ),
  );

  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'Steam 黄油补丁助手',
    debugShowCheckedModeBanner: false,
    locale: const Locale('zh', 'CN'),
    supportedLocales: const [Locale('zh', 'CN')],
    localizationsDelegates: GlobalMaterialLocalizations.delegates,
    theme: theme(Brightness.light),
    darkTheme: theme(Brightness.dark),
    themeMode: themeMode,
    home: widget.home,
  );
}
