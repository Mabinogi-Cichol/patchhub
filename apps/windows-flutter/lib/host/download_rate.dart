/// Throughput over the latest two accepted download progress samples.
class DownloadRate {
  int? _bytes;
  Duration? _time;
  double? bytesPerSecond;

  void update(String stage, int bytes, Duration elapsed) {
    if (stage != 'downloading') {
      _bytes = null;
      _time = null;
      bytesPerSecond = null;
      return;
    }
    final previousBytes = _bytes;
    final previousTime = _time;
    if (previousBytes != null &&
        previousTime != null &&
        bytes >= previousBytes &&
        elapsed > previousTime) {
      bytesPerSecond =
          (bytes - previousBytes) *
          Duration.microsecondsPerSecond /
          (elapsed - previousTime).inMicroseconds;
    } else {
      bytesPerSecond = null;
    }
    _bytes = bytes;
    _time = elapsed;
  }
}
