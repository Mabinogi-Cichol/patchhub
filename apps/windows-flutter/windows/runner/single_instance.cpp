#include "single_instance.h"

SingleInstance::SingleInstance(const wchar_t* name) : name_(name) {
  mutex_ = CreateMutexW(nullptr, FALSE, name_.c_str());
  if (!mutex_) {
    error_ = GetLastError();
    return;
  }
  const DWORD status = WaitForSingleObject(mutex_, 0);
  primary_ = status == WAIT_OBJECT_0 || status == WAIT_ABANDONED;
  if (status == WAIT_FAILED) error_ = GetLastError();
}

SingleInstance::~SingleInstance() {
  if (primary_) ReleaseMutex(mutex_);
  if (mutex_) CloseHandle(mutex_);
}

bool SingleInstance::RegisterWindow(HWND window) const {
  // Use our own property: Flutter's default window class is shared by other apps.
  return SetPropW(window, name_.c_str(), reinterpret_cast<HANDLE>(1)) != FALSE;
}

void SingleInstance::ActivateExistingWindow() const {
  struct Search {
    const wchar_t* property;
    HWND window;
  } search{name_.c_str(), nullptr};
  EnumWindows(
      [](HWND window, LPARAM parameter) -> BOOL {
        auto* search = reinterpret_cast<Search*>(parameter);
        if (!GetPropW(window, search->property)) return TRUE;
        search->window = window;
        return FALSE;
      },
      reinterpret_cast<LPARAM>(&search));
  if (search.window) {
    ShowWindowAsync(search.window, IsIconic(search.window) ? SW_RESTORE : SW_SHOW);
    SetForegroundWindow(search.window);
  }
  // If the first process is still creating its window, it will show it normally.
}
