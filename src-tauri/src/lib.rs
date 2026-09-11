mod app_state;
mod common;
mod pages;

use app_state::AppState;
use pages::{index, model_list, settings, dgx_deploy, test_ui};

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
    // vLLM Docker 模式：停止并删除容器
    let container_opt = state.running_container.lock().ok().and_then(|l| l.clone());
    if let Some(container) = container_opt {
        // 多机模式（容器名 adm-vllm-<model>-rank-0）：先 SSH 停止远端节点容器（尽力，3s/台）
        if container.ends_with("-rank-0") {
            if let Ok(settings_path) = crate::common::config::get_data_dir(Some(app)).map(|d| d.join("config.json")) {
                if let Ok(json) = std::fs::read_to_string(settings_path) {
                    if let Ok(parsed) = serde_json::from_str::<crate::common::types::Settings>(&json) {
                        let mn = parsed.multi_node_args;
                        let key: Option<String> = if mn.ssh_key_path.trim().is_empty() {
                            None
                        } else {
                            Some(mn.ssh_key_path.trim().to_string())
                        };
                        // 远端容器名与运行中容器同源（<base>-rank-<i>，兼容 vLLM / SGLang 两种前缀）
                        let base = container.strip_suffix("-rank-0").unwrap_or(&container);
                        for (i, node) in mn.nodes.iter().enumerate().skip(1) {
                            if node.is_self {
                                continue;
                            }
                            let c = format!("{}-rank-{}", base, i);
                            let script = crate::common::ssh::stop_container_script(&c);
                            let _ = crate::common::ssh::ssh_run_blocking(
                                &node.ip,
                                &node.ssh_user,
                                node.ssh_port,
                                key.as_deref(),
                                &script,
                                std::time::Duration::from_secs(3),
                            );
                        }
                    }
                }
            }
        }
        let _ = crate::common::utils::platform::docker_cmd()
            .args(["stop", "-t", "3", &container])
            .output();
        let _ = crate::common::utils::platform::docker_cmd()
            .args(["rm", "-f", &container])
            .output();
    }
    // 兜底：按进程名清理任何残留的 llama-server 子进程
    #[cfg(target_os = "windows")]
    {
        crate::common::utils::platform::kill_process_by_name("llama-server.exe");
    }
    #[cfg(not(target_os = "windows"))]
    {
        crate::common::utils::platform::kill_process_by_name("llama-server");
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

            // ===== 启动权限预检（Linux）：免密 sudo / docker 组缺失时自动弹 pkexec 修复 =====
            #[cfg(not(target_os = "windows"))]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // 延迟 3 秒：等待桌面会话与 polkit 认证代理就绪（与更新检查一致）
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    crate::common::utils::platform::startup_ensure_docker_permission(handle).await;
                });
            }

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
            model_list::cancel_download,
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
            settings::get_docker_mirror_config,
            settings::save_docker_mirror_config,
            settings::get_docker_proxy_config,
            settings::save_docker_proxy_config,
            settings::multi_node_probe,
            settings::ensure_ssh_key,
            settings::push_image_to_remote,
            settings::sync_model_to_remote,
            settings::get_app_data_dir,
            settings::get_local_username,
            // dgx_deploy.rs
            dgx_deploy::dgx_deploy_run,
            // test_ui.rs
            test_ui::start_test_ui,
            test_ui::stop_test_ui,
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
