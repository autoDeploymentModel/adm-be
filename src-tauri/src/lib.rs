mod app_state;
mod common;
mod pages;

use app_state::AppState;
use pages::{index, model_list, model_image, settings, benchmark};

use tauri::Manager;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

/// 显示主窗口并带到前台。
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_skip_taskbar(false);
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 清理所有子进程（模型 / SD）。
/// 从原 `on_window_event(CloseRequested)` 提取，供托盘"退出"和正常关闭复用。
fn cleanup_processes(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();

    // 强杀记录中的模型/SD 进程（整棵进程树）
    let pid_opt = state.running_process.lock().ok().and_then(|l| *l);
    if let Some(pid) = pid_opt {
        crate::common::utils::platform::kill_process_tree(pid);
    }
    // SGLang Docker 模式：停止并删除容器
    let container_opt = state.running_container.lock().ok().and_then(|l| l.clone());
    if let Some(container) = container_opt {
        let _ = crate::common::utils::platform::docker_cmd()
            .args(["stop", "-t", "3", &container])
            .output();
        let _ = crate::common::utils::platform::docker_cmd()
            .args(["rm", "-f", &container])
            .output();
    }
    // 兜底：按进程名清理任何残留的 llama-server / SD 子进程
    #[cfg(target_os = "windows")]
    {
        crate::common::utils::platform::kill_process_by_name("llama-server.exe");
        crate::common::utils::platform::kill_process_by_name("sd-cli.exe");
    }
    #[cfg(not(target_os = "windows"))]
    {
        crate::common::utils::platform::kill_process_by_name("llama-server");
        crate::common::utils::platform::kill_process_by_name("sd-cli");
    }
}


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 单实例：第二个实例启动时，让第一个实例显示窗口
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_hwinfo::init())
        .manage(AppState::new())
        .setup(|app| {
            crate::common::utils::logger::init();

            // ===== 系统托盘 =====
            let show_item = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出 ADM-BE", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("ADM-BE")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        show_main_window(app);
                    }
                    "quit" => {
                        cleanup_processes(app);
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    // Windows 上单击会触发两次 Click（Down + Up），
                    // 仅在鼠标释放（Up）时处理，避免 show→hide 闪一下又消失
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_minimized().unwrap_or(false) {
                                // 最小化在任务栏：恢复显示并带到前台
                                show_main_window(app);
                            } else if window.is_visible().unwrap_or(false) {
                                // 隐藏到托盘
                                let _ = window.hide();
                                let _ = window.set_skip_taskbar(true);
                            } else {
                                show_main_window(app);
                            }
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Windows/Linux：拦截关闭，隐藏到系统托盘（模型继续运行）
                api.prevent_close();
                let _ = window.hide();
                let _ = window.set_skip_taskbar(true);
            }
        })
        .invoke_handler(tauri::generate_handler![
            // index.rs
            index::get_system_info,
            index::check_update,
            // model_list.rs
            model_list::scan_local_models,
            model_list::scan_part_files,
            model_list::fetch_model_list,
            model_list::download_model,
            model_list::start_model,
            model_list::stop_model,
            model_list::get_model_status,
            model_list::delete_local_model,
            model_list::get_downloading_models,
            model_list::get_downloading_phases,
            // model_image.rs
            model_image::get_sd_status,
            model_image::download_and_extract_sd,
            model_image::start_sd_generation,
            model_image::stop_sd,
            model_image::save_sd_image_as,
            // settings.rs
            settings::save_settings,
            settings::load_settings,
            settings::get_app_version,
            settings::read_log,
            settings::list_log_dates,
            settings::open_log_dir,
            settings::fix_docker_permission,
            settings::write_app_log,
            settings::clear_all_logs,
            // benchmark.rs
            benchmark::start_benchmark,
            benchmark::get_benchmark_status,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Cmd+Q / 系统退出走 RunEvent::ExitRequested（不是 CloseRequested），
    // 只有在这里才能统一拦截清理子进程，否则残留为孤儿
    // （端口被占用、下次启动模型报"端口占用"）。cleanup_processes 幂等，重复调用安全。
    app.run(|app_handle, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            cleanup_processes(app_handle);
        }
    });
}
