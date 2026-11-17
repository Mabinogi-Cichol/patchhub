import 'package:flutter/material.dart';

enum PatchStatusTone {
  success,
  accent,
  querying,
  neutral;

  Color color(ColorScheme scheme) {
    final dark = scheme.brightness == Brightness.dark;
    return switch (this) {
      success => dark ? const Color(0xff70d7a7) : const Color(0xff17633e),
      accent => scheme.secondary,
      querying => dark ? const Color(0xfff5c451) : const Color(0xff9a6700),
      neutral => scheme.onSurfaceVariant,
    };
  }
}
