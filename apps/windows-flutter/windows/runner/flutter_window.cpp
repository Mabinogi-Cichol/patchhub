#include "flutter_window.h"

#include <optional>
#include <flutter/standard_method_codec.h>
#include <vector>

#include "flutter/generated_plugin_registrant.h"

FlutterWindow::FlutterWindow(const flutter::DartProject& project)
    : project_(project) {}

FlutterWindow::~FlutterWindow() {}

bool FlutterWindow::OnCreate() {
  if (!Win32Window::OnCreate()) {
    return false;
  }

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());
  desktop_channel_ = std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
      flutter_controller_->engine()->messenger(), "steam_patch/desktop",
      &flutter::StandardMethodCodec::GetInstance());
  desktop_channel_->SetMethodCallHandler([](
      const flutter::MethodCall<flutter::EncodableValue>& call,
      std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result) {
    if (call.method_name() != "steamArtworkContext") {
      result->NotImplemented();
      return;
    }
    DWORD bytes = 0;
    auto status = RegGetValueW(HKEY_CURRENT_USER, L"Software\\Valve\\Steam",
        L"SteamPath", RRF_RT_REG_SZ, nullptr, nullptr, &bytes);
    if (status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND) {
      result->Success();
      return;
    }
    if (status != ERROR_SUCCESS) {
      result->Error("steam_registry", "Cannot read Steam install location");
      return;
    }
    std::vector<wchar_t> path(bytes / sizeof(wchar_t));
    status = RegGetValueW(HKEY_CURRENT_USER, L"Software\\Valve\\Steam",
        L"SteamPath", RRF_RT_REG_SZ, nullptr, path.data(), &bytes);
    if (status != ERROR_SUCCESS) {
      result->Error("steam_registry", "Cannot read Steam install location");
      return;
    }
    const int length = WideCharToMultiByte(CP_UTF8, 0, path.data(), -1, nullptr, 0, nullptr, nullptr);
    std::string root(length, '\0');
    WideCharToMultiByte(CP_UTF8, 0, path.data(), -1, root.data(), length, nullptr, nullptr);
    root.resize(length - 1);
    DWORD user = 0;
    DWORD user_bytes = sizeof(user);
    const auto user_status = RegGetValueW(HKEY_CURRENT_USER,
        L"Software\\Valve\\Steam\\ActiveProcess", L"ActiveUser",
        RRF_RT_REG_DWORD, nullptr, &user, &user_bytes);
    if (user_status != ERROR_SUCCESS && user_status != ERROR_FILE_NOT_FOUND &&
        user_status != ERROR_PATH_NOT_FOUND) {
      result->Error("steam_registry", "Cannot read Steam active account");
      return;
    }
    result->Success(flutter::EncodableValue(flutter::EncodableMap{
      {flutter::EncodableValue("root"), flutter::EncodableValue(root)},
      {flutter::EncodableValue("activeUser"), flutter::EncodableValue(static_cast<int64_t>(user))},
    }));
  });
  SetChildContent(flutter_controller_->view()->GetNativeWindow());

  flutter_controller_->engine()->SetNextFrameCallback([&]() {
    this->Show();
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  return true;
}

void FlutterWindow::OnDestroy() {
  desktop_channel_ = nullptr;
  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  switch (message) {
    case WM_FONTCHANGE:
      flutter_controller_->engine()->ReloadSystemFonts();
      break;
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}
