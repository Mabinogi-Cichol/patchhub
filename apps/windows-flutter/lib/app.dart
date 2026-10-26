import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

class PatchHubApp extends StatefulWidget {
  const PatchHubApp({super.key, required this.home});
  final Widget home;

  static ThemeMode themeModeOf(BuildContext context) =>
      context.findAncestorStateOfType<_PatchHubAppState>()!.themeMode;

  static void setThemeMode(BuildContext context, ThemeMode mode) =>
      context.findAncestorStateOfType<_PatchHubAppState>()!.setThemeMode(mode);

  @override
  State<PatchHubApp> createState() => _PatchHubAppState();
}

class _PatchHubAppState extends State<PatchHubApp> {
  ThemeMode themeMode = ThemeMode.system;

  void setThemeMode(ThemeMode mode) {
    setState(() => themeMode = mode);
  }

  ThemeData theme(Brightness brightness) {
    final dark = brightness == Brightness.dark;
    final accent = dark ? const Color(0xff78a9ff) : const Color(0xff245dd8);
    final scheme =
        ColorScheme.fromSeed(
          seedColor: const Color(0xff3b75ee),
          brightness: brightness,
          dynamicSchemeVariant: dark
              ? DynamicSchemeVariant.monochrome
              : DynamicSchemeVariant.tonalSpot,
        ).copyWith(
          primary: dark ? const Color(0xff346be0) : const Color(0xff245dd8),
          onPrimary: Colors.white,
          primaryContainer: dark
              ? const Color(0xff303030)
              : const Color(0xffe3edff),
          onPrimaryContainer: dark
              ? const Color(0xffeeeeee)
              : const Color(0xff18489f),
          secondary: accent,
          onSecondary: dark ? const Color(0xff161616) : Colors.white,
          secondaryContainer: dark
              ? const Color(0xff242424)
              : const Color(0xffeaf0f6),
          onSecondaryContainer: dark
              ? const Color(0xffeeeeee)
              : const Color(0xff202020),
          tertiary: accent,
          surface: dark ? const Color(0xff191919) : Colors.white,
          surfaceContainer: dark
              ? const Color(0xff242424)
              : const Color(0xffeaf0f6),
          onSurface: dark ? const Color(0xffeeeeee) : const Color(0xff172333),
          onSurfaceVariant: dark
              ? const Color(0xffb8b8b8)
              : const Color(0xff526176),
          outline: dark ? const Color(0xff626262) : const Color(0xff8695aa),
          outlineVariant: dark
              ? const Color(0xff363636)
              : const Color(0xffd5dee9),
          surfaceTint: Colors.transparent,
        );
    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      scaffoldBackgroundColor: dark
          ? const Color(0xff101010)
          : const Color(0xfff3f5f8),
      fontFamily: 'Segoe UI',
      fontFamilyFallback: const ['Microsoft YaHei UI', 'Microsoft YaHei'],
      appBarTheme: AppBarTheme(
        backgroundColor: scheme.surface,
        foregroundColor: scheme.onSurface,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
      ),
      cardTheme: CardThemeData(
        color: scheme.surface,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        margin: const EdgeInsets.only(bottom: 16),
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(16),
          side: BorderSide(color: scheme.outlineVariant),
        ),
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: scheme.surfaceContainer,
        isDense: true,
        contentPadding: const EdgeInsets.symmetric(
          horizontal: 14,
          vertical: 14,
        ),
        border: OutlineInputBorder(borderRadius: BorderRadius.circular(10)),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: scheme.outlineVariant),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: accent, width: 2),
        ),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          minimumSize: const Size(0, 44),
          padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(10),
          ),
        ),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          foregroundColor: scheme.onSurface,
          minimumSize: const Size(0, 40),
          side: BorderSide(color: scheme.outline),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(10),
          ),
        ),
      ),
      dividerColor: scheme.outlineVariant,
      focusColor: accent.withValues(alpha: 0.16),
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(foregroundColor: accent),
      ),
      progressIndicatorTheme: ProgressIndicatorThemeData(color: accent),
      textSelectionTheme: TextSelectionThemeData(
        cursorColor: accent,
        selectionColor: accent.withValues(alpha: 0.25),
        selectionHandleColor: accent,
      ),
    );
  }

  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'PatchHub',
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
