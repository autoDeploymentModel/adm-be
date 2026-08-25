// test 构建保留控制台入口（否则 Windows 上 cargo test 链接失败：WinMain 未解析）
#![cfg_attr(not(test), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

fn main() {
    // Linux + NVIDIA GPU 上 WebKitGTK DMABUF 渲染器与驱动不兼容，导致空白窗口。
    // 禁用 DMABUF renderer，回退到共享内存渲染（Tauri 官方推荐方案）。
    #[cfg(target_os = "linux")]
    {
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    // Windows高DPI感知，防止图标、窗口拉伸模糊
    #[cfg(target_os = "windows")]
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    adm_lib::run()
}
