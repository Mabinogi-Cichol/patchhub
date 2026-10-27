#ifndef RUNNER_SINGLE_INSTANCE_H_
#define RUNNER_SINGLE_INSTANCE_H_

#include <windows.h>

#include <string>

// Owns the startup gate until the native runner has finished shutting down.
class SingleInstance {
 public:
  explicit SingleInstance(const wchar_t* name);
  ~SingleInstance();
  SingleInstance(const SingleInstance&) = delete;
  SingleInstance& operator=(const SingleInstance&) = delete;

  bool is_primary() const { return primary_; }
  DWORD error() const { return error_; }
  bool RegisterWindow(HWND window) const;
  void ActivateExistingWindow() const;

 private:
  std::wstring name_;
  HANDLE mutex_ = nullptr;
  bool primary_ = false;
  DWORD error_ = ERROR_SUCCESS;
};

#endif  // RUNNER_SINGLE_INSTANCE_H_
