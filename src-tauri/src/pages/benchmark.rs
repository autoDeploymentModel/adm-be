// benchmark.rs - 模型性能测试

use crate::common::error::AppError;
use crate::app_state::AppState;
use crate::bail;
use tauri::Manager;
use tauri::Emitter;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkParams {
    pub input_len: Option<u64>,
    pub output_len: Option<u64>,
    pub num_prompts: Option<u64>,
}

/// 在 vLLM 容器内执行 `vllm bench serve` 性能测试
///
/// 注意：`--backend vllm-scan` 是 `eugr/spark-vllm` fork 自定义后端
/// （同 fork 的 Ray worker entrypoint 也是 `python -m vllm.distributed.ray_utils`），
/// upstream vLLM 用 `--backend vllm`。本应用部署的镜像均为 eugr fork，
/// 切到 upstream vLLM 镜像时需把后端改回 `vllm`。
#[tauri::command]
pub async fn start_benchmark(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    params: BenchmarkParams,
) -> Result<(), AppError> {
    let container = state.running_container.lock().map_err(|e| e.to_string())?.clone();
    let port = state.running_port.lock().map_err(|e| e.to_string())?.unwrap_or(8000);

    let container_name = container.ok_or("没有正在运行的模型容器")?;

    {
        let mut running = state.benchmark_running.lock().map_err(|e| e.to_string())?;
        if *running {
            bail!("测试正在进行中，请等待完成");
        }
        *running = true;
    }

    let input_len = params.input_len.unwrap_or(1024);
    let output_len = params.output_len.unwrap_or(256);
    let num_prompts = params.num_prompts.unwrap_or(5);

    let base_url = format!("http://127.0.0.1:{}", port);

    // vLLM `--dataset-name random` 自带随机 prompt 生成，无需额外 dataset-path 文件。
    let mut cmd = crate::common::utils::platform::docker_cmd();
    cmd.args([
        "exec", "-e", "HF_HUB_OFFLINE=1", &container_name,
        "vllm", "bench", "serve",
        "--backend", "vllm-scan",
        "--base-url", &base_url,
        "--model", "default",
        "--dataset-name", "random",
        "--random-input-len", &input_len.to_string(),
        "--random-output-len", &output_len.to_string(),
        "--num-prompts", &num_prompts.to_string(),
    ]);

    let app_clone = app.clone();

    std::thread::spawn(move || {
        let mut child = match cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = app_clone.emit("benchmark-log", serde_json::json!({"line": format!("[ERROR] 启动测试失败: {}", e)}));
                let _ = app_clone.emit("benchmark-complete", serde_json::json!({"success": false, "output": format!("启动失败: {}", e)}));
                let state = app_clone.state::<AppState>();
                if let Ok(mut r) = state.benchmark_running.lock() { *r = false; }
                return;
            }
        };

        use std::io::{BufRead, BufReader};

        // stdout 线程
        let app_stdout = app_clone.clone();
        let stdout_handle = if let Some(stdout) = child.stdout.take() {
            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    crate::common::utils::logger::write_log("INFO", "BENCH", &line);
                    let _ = app_stdout.emit("benchmark-log", serde_json::json!({"line": line}));
                }
            }))
        } else { None };

        // stderr 线程
        let app_stderr = app_clone.clone();
        let stderr_handle = if let Some(stderr) = child.stderr.take() {
            Some(std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    crate::common::utils::logger::write_log("WARN", "BENCH", &line);
                    let _ = app_stderr.emit("benchmark-log", serde_json::json!({"line": line}));
                }
            }))
        } else { None };

        if let Some(h) = stdout_handle { let _ = h.join(); }
        if let Some(h) = stderr_handle { let _ = h.join(); }

        let status = child.wait();
        let success = status.as_ref().map(|s| s.success()).unwrap_or(false);

        let _ = app_clone.emit("benchmark-complete", serde_json::json!({"success": success}));

        {
            let state = app_clone.state::<AppState>();
            let guard = state.benchmark_running.lock();
            if let Ok(g) = guard {
                let mut g = g;
                *g = false;
            }
        }
    });

    Ok(())
}

/// 查询测试是否正在进行
#[tauri::command]
pub async fn get_benchmark_status(state: tauri::State<'_, AppState>) -> Result<bool, AppError> {
    let running = state.benchmark_running.lock().map_err(|e| e.to_string())?;
    Ok(*running)
}
