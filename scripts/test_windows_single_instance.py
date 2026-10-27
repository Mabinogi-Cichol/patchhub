"""Compile the production Windows startup gate and exercise real processes/windows."""
import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest
import uuid

RUNNER = Path(__file__).resolve().parents[1] / 'apps/windows-flutter/windows/runner'


@unittest.skipUnless(os.name == 'nt', 'Single-instance gate uses Win32')
class WindowsSingleInstanceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.build = tempfile.TemporaryDirectory(prefix='patchhub-single-instance-')
        cls.root = Path(cls.build.name)
        cls.executable = cls.root / 'instance.exe'
        source = cls.root / 'instance.cpp'
        source.write_text(r'''
#include "single_instance.h"
#include <iostream>
#include <string>

LRESULT CALLBACK WindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
  if (message == WM_DESTROY) { PostQuitMessage(0); return 0; }
  return DefWindowProcW(window, message, wparam, lparam);
}

int wmain(int argc, wchar_t** argv) {
  if (argc < 2) return 2;
  SingleInstance instance(argv[1]);
  if (instance.error()) return 3;
  if (!instance.is_primary()) {
    instance.ActivateExistingWindow();
    std::cout << "secondary" << std::endl;
    return 0;
  }
  if (argc > 2 && std::wstring(argv[2]) == L"--defer-window") {
    std::cout << "locked" << std::endl;
    std::cin.get();
  }
  WNDCLASSW window_class{};
  window_class.lpfnWndProc = WindowProc;
  window_class.hInstance = GetModuleHandleW(nullptr);
  window_class.lpszClassName = L"PatchHubSingleInstanceTest";
  if (!RegisterClassW(&window_class)) return 4;
  HWND window = CreateWindowW(window_class.lpszClassName, L"PatchHub test",
      WS_OVERLAPPEDWINDOW, 0, 0, 400, 240, nullptr, nullptr,
      window_class.hInstance, nullptr);
  if (!window || !instance.RegisterWindow(window)) return 5;
  ShowWindow(window, SW_SHOWNOACTIVATE);
  std::cout << "primary " << reinterpret_cast<uintptr_t>(window) << std::endl;
  MSG message;
  while (GetMessageW(&message, nullptr, 0, 0) > 0) {
    TranslateMessage(&message);
    DispatchMessageW(&message);
  }
  return 0;
}
''', encoding='utf-8')
        vswhere = Path(os.environ['ProgramFiles(x86)']) / 'Microsoft Visual Studio/Installer/vswhere.exe'
        installation = subprocess.check_output([
            str(vswhere), '-latest', '-products', '*',
            '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
            '-property', 'installationPath'], text=True).strip()
        vcvars = Path(installation) / 'VC/Auxiliary/Build/vcvars64.bat'
        build_script = cls.root / 'build.cmd'
        compile_command = subprocess.list2cmdline([
            'cl.exe', '/nologo', '/std:c++17', '/EHsc', '/W4', '/WX', '/wd4100',
            '/DUNICODE', '/D_UNICODE', '/utf-8', f'/I{RUNNER}',
            str(source), str(RUNNER / 'single_instance.cpp'),
            f'/Fe:{cls.executable}', '/link', 'user32.lib'])
        build_script.write_text(
            f'@echo off\ncall "{vcvars}" >nul\nif errorlevel 1 exit /b %errorlevel%\n'
            f'{compile_command}\nexit /b %errorlevel%\n', encoding='utf-8')
        result = subprocess.run(['cmd.exe', '/d', '/c', str(build_script)],
                                cwd=cls.root, capture_output=True, text=True)
        if result.returncode:
            cls.build.cleanup()
            raise RuntimeError(result.stdout + result.stderr)
        cls.user32 = ctypes.WinDLL('user32', use_last_error=True)
        cls.user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
        cls.user32.ShowWindow.argtypes = [wintypes.HWND, ctypes.c_int]
        cls.user32.IsIconic.argtypes = [wintypes.HWND]
        cls.kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
        cls.kernel32.OpenMutexW.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.LPCWSTR]
        cls.kernel32.OpenMutexW.restype = wintypes.HANDLE
        cls.kernel32.CloseHandle.argtypes = [wintypes.HANDLE]

    @classmethod
    def tearDownClass(cls):
        cls.build.cleanup()

    def setUp(self):
        self.name = 'Local\\PatchHub.SingleInstance.Test.' + uuid.uuid4().hex
        self.processes = []
        self.windows = {}

    def tearDown(self):
        for process in self.processes:
            if process.poll() is None:
                window = self.windows.get(process.pid)
                if window:
                    self.user32.PostMessageW(window, 0x0010, 0, 0)  # WM_CLOSE
                else:
                    process.terminate()
                process.wait(timeout=10)
            process.stdin.close()
            process.stdout.close()
            process.stderr.close()

    def start(self, *arguments, executable=None):
        process = subprocess.Popen([str(executable or self.executable), self.name, *arguments],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True)
        self.processes.append(process)
        return process

    def receipt(self, process):
        line = process.stdout.readline().strip()
        if line.startswith('primary '):
            self.windows[process.pid] = int(line.split()[1])
            return 'primary'
        self.assertIn(line, ['secondary', 'locked'], process.stderr.read() if not line else line)
        return line

    def test_duplicate_exits_and_restores_minimized_window(self):
        first = self.start()
        self.assertEqual(self.receipt(first), 'primary')
        window = self.windows[first.pid]
        self.user32.ShowWindow(window, 6)  # SW_MINIMIZE
        self.assertTrue(self.user32.IsIconic(window))
        duplicate = self.start()
        self.assertEqual(self.receipt(duplicate), 'secondary')
        self.assertEqual(duplicate.wait(timeout=10), 0)
        deadline = time.monotonic() + 5
        while self.user32.IsIconic(window) and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertFalse(self.user32.IsIconic(window))
        self.assertIsNone(first.poll())

    def test_simultaneous_launches_keep_only_one_primary(self):
        processes = [self.start() for _ in range(8)]
        roles = [self.receipt(process) for process in processes]
        self.assertEqual(roles.count('primary'), 1)
        for process, role in zip(processes, roles):
            if role == 'secondary':
                self.assertEqual(process.wait(timeout=10), 0)

    def test_duplicate_during_initialization_does_not_create_a_window(self):
        first = self.start('--defer-window')
        self.assertEqual(self.receipt(first), 'locked')
        duplicate = self.start()
        self.assertEqual(self.receipt(duplicate), 'secondary')
        self.assertEqual(duplicate.wait(timeout=10), 0)
        first.stdin.write('\n')
        first.stdin.flush()
        self.assertEqual(self.receipt(first), 'primary')

    def test_another_installation_uses_the_same_gate(self):
        other = self.root / 'other-installation'
        other.mkdir(exist_ok=True)
        shutil.copyfile(self.executable, other / 'instance.exe')
        first = self.start()
        self.assertEqual(self.receipt(first), 'primary')
        duplicate = self.start(executable=other / 'instance.exe')
        self.assertEqual(self.receipt(duplicate), 'secondary')
        self.assertEqual(duplicate.wait(timeout=10), 0)

    def test_normal_exit_allows_update_probe_and_probe_cannot_bypass_gate(self):
        first = self.start()
        self.assertEqual(self.receipt(first), 'primary')
        self.user32.PostMessageW(self.windows[first.pid], 0x0010, 0, 0)
        self.assertEqual(first.wait(timeout=10), 0)
        updated = self.start('--update-probe')
        self.assertEqual(self.receipt(updated), 'primary')
        duplicate = self.start('--update-probe')
        self.assertEqual(self.receipt(duplicate), 'secondary')
        self.assertEqual(duplicate.wait(timeout=10), 0)

    def test_abandoned_mutex_after_crash_can_be_acquired(self):
        first = self.start()
        self.assertEqual(self.receipt(first), 'primary')
        handle = self.kernel32.OpenMutexW(0x00100001, False, self.name)
        self.assertTrue(handle)
        try:
            first.kill()
            first.wait(timeout=10)
            replacement = self.start()
            self.assertEqual(self.receipt(replacement), 'primary')
        finally:
            self.kernel32.CloseHandle(handle)


if __name__ == '__main__':
    unittest.main()
