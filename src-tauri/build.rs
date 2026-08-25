fn main() {
    tauri_build::build();

    // 仅在 Windows 上设置子系统为 WINDOWS，避免命令行窗口；
    // 只作用于 bin 目标（-bins）：cargo test 的测试可执行文件不受影响，避免 WinMain 未解析
    #[cfg(target_os = "windows")]
    println!("cargo:rustc-link-arg-bins=/SUBSYSTEM:WINDOWS");
}