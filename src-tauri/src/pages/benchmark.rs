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

/// 探测容器内 `vllm bench serve` 支持的 backend
///
/// 原本硬编码 `--backend vllm-scan`，但该 backend 未在 `eugr/spark-vllm-docker`
/// fork 官方文档中出现（README/CHANGELOG 全部翻过只字未提；fork 推荐压测工具是
/// 独立的 `llama-benchy`，不是 vLLM 内置后端）。本地部署的 `eugr/spark-vllm`
/// nightly 镜像实测也不包含 `vllm-scan`（启动直接报 invalid choice），所以这里
/// 启动前先查 `--help`，能选到 `vllm-scan` 就用，否则回退 `vllm`（标准 vLLM
/// 压测后端，upstream 和 fork 都支持）。
fn detect_bench_backend(container: &str) -> String {
    use std::process::Stdio;
    let mut probe = crate::common::utils::platform::docker_cmd();
    probe.args(["exec", container, "vllm", "bench", "serve", "--help"]);
    let output = probe
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(o) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            if o.status.success() && text.contains("vllm-scan") {
                "vllm-scan".to_string()
            } else {
                "vllm".to_string()
            }
        }
        Err(_) => "vllm".to_string(),
    }
}

/// 在 vLLM 容器内执行 `vllm bench serve` 性能测试
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

    let app_clone = app.clone();

    std::thread::spawn(move || {
        // 先探测 backend（fork 镜像有 `vllm-scan`，upstream 只有 `vllm`）
        let backend = detect_bench_backend(&container_name);
        let _ = app_clone.emit(
            "benchmark-log",
            serde_json::json!({"line": format!("[INFO] 使用 benchmark backend: {}", backend)}),
        );

        // vLLM `--dataset-name random` 自带随机 prompt 生成，无需额外 dataset-path 文件。
        let mut cmd = crate::common::utils::platform::docker_cmd();
        cmd.args([
            "exec", "-e", "HF_HUB_OFFLINE=1", &container_name,
            "vllm", "bench", "serve",
            "--backend", &backend,
            "--base-url", &base_url,
            // 不传 --model，让 bench 从服务 /v1/models 自动取首个登记的 model 名。
            // 写死 'default' 大概率与服务侧 served-model-name 不匹配，API 会 404。
            "--dataset-name", "random",
            "--random-input-len", &input_len.to_string(),
            "--random-output-len", &output_len.to_string(),
            "--num-prompts", &num_prompts.to_string(),
        ]);

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
