# SGLang Docker 部署流程（ADM 桌面端）

> 现状：ADM 已完全移除 llama.cpp 方案，模型推理统一走 **SGLang + Docker**。
> 当前适配机型：`dgx-spark-128G`（NVIDIA DGX Spark，GB10，128GB 统一内存，arm64）。
> 首期平台：Ubuntu（Docker）。Windows / macOS 后续分系统针对性适配。

## 1. 总体架构

```
┌─────────────────────────── ADM 桌面端（Tauri）───────────────────────────┐
│  首页模型列表（机型下拉 → 按 model_support_devices 过滤）                   │
│     │ 下载（hf-mirror 多文件目录下载，.part 断点续传）                      │
│     ▼                                                                     │
│  models/<model_id>/  （config.json + model.safetensors* + .done 标记）    │
│     │ 启动（start_model → start_sglang_docker）                           │
│     ▼                                                                     │
│  docker run --gpus all --shm-size 64g -p <port>:<port>                   │
│     -v <模型目录>:/models/<model_id>:ro                                   │
│     lmsysorg/sglang:v0.5.17                                               │
│     python3 -m sglang.launch_server --model-path /models/<model_id> ...  │
│     │                                                                    │
│     ▼                                                                    │
│  OpenAI 兼容 API : http://127.0.0.1:<port>（/v1/chat/completions 等）   │
│     ▲                                                                    │
│  前端「查看模型」→ 系统浏览器打开 WebUI                                   │
└──────────────────────────────────────────────────────────────────────────┘
```

## 2. 前置条件

| 项目 | 要求 |
|---|---|
| 操作系统 | Ubuntu（Docker 支持） |
| Docker | 已安装且 `docker info` 可用（NVIDIA Container Toolkit 已配置 `--gpus all`） |
| 机型 | 首页顶部「适配机型」下拉选择 `dgx-spark-128G`（后续机型扩展处：`model_list.rs` 的 `start_sglang_docker` 机型匹配） |
| 镜像 | `lmsysorg/sglang:v0.5.17`（固定 tag，见 §3） |
| 网络 | 可访问 `hf-mirror.com`（自动镜像替换 `huggingface.co`） |

## 3. 镜像版本策略（已调研确认）

**固定使用 `lmsysorg/sglang:v0.5.17`（2026-08-08 最新稳定版）**

- v0.5.14 起正式支持 GB10（`Add GB10 FP8 fused MoE Triton config`），v0.5.17 是当前最稳妥版本
- 多架构镜像：`v0.5.10+` 统一 tag，arm64（DGX Spark 架构）自动拉取对应层，无需 `-arm64` 后缀
- 版本线现状：无 v0.6.x；`latest` 是可变 tag（不推荐生产）；官方 `spark` tag 已过时（v0.5.3.post3）
- 升级流程：改 `config.json` 的 `sglang_args.image` → `docker rm -f` 旧容器 → 重新启动拉新镜像

**⚠️ NBFP4 注意事项**：NVFP4（FP4）在 GB10 上有已知 CUDA 崩溃问题（社区反馈），
DGX Spark 上**优先推荐 FP8 权重**。模型清单配置 NVFP4 模型时若启动异常，换 FP8 版本。

## 4. 模型清单格式（远端 model.json）

```json
[
    {
        "model_id": "Qwen3.5-9B-Q4_K_M",
        "model_download_files": [
            "https://huggingface.co/unsloth/Qwen3.8-27B-NVFP4/resolve/main/config.json",
            "https://huggingface.co/unsloth/Qwen3.8-27B-NVFP4/resolve/main/model.safetensors",
            "https://huggingface.co/unsloth/Qwen3.8-27B-NVFP4/resolve/main/model_mtp.safetensors",
            "https://huggingface.co/unsloth/Qwen3.8-27B-NVFP4/resolve/main/tokenizer.json",
            "..."
        ],
        "model_support_devices": ["dgx-spark-128G"],
        "model_size": "23.4 GB",
        "model_type": "视觉多模态理解",
        "model_description": "支持对话,推理,图片识别",
        "support_tools": true,
        "support_reasoning": true,
        "support_images": true
    }
]
```

- `model_download_files`：HF 仓库多文件清单（safetensors 目录模型必需；旧 `model_url` 单文件不再支持启动）
- `model_support_devices`：适配机型列表；**空数组 = 全机型可用**；首页下拉按此过滤
- 下载时自动替换 `huggingface.co` → `hf-mirror.com`

## 5. 模型下载（多文件目录）

- 目标目录：`<data_dir>/models/<model_id>/`
- 逐文件下载，每个文件 `.part` 断点续传（`download_with_resume`）
- 已存在且无 `.part` 残留的文件跳过
- 前端总进度 = 已完成文件数 + 当前文件进度折算
- 全部完成后写 `.done` 标记（`scan_local_models` 据此识别完整模型，本地文件列表不含 `.done`）
- 断点续传判定：`download-complete` 事件带 `all: true` → 前端一次性写入全部文件名

## 6. 启动流程（start_model → SGLang Docker）

调起条件：目录模型（存在 `.done`，或 `config.json` + `model.safetensors`）。
旧 GGUF 模型启动时报错：「当前仅支持 SGLang（safetensors 目录）模型」。

### 6.0 启动前 Docker 环境预检（check_docker_env）

`start_sglang_docker` 在拼装容器命令前依序检查，任一失败即中止启动并返回可读错误：

| 顺序 | 检查项 | 命令 | 失败提示 |
|---|---|---|---|
| 1 | Docker CLI 存在 | `docker --version` | 未检测到 Docker CLI，请先安装 Docker |
| 2 | daemon 运行 | `docker info` | daemon 未运行或不可访问，请启动 Docker 服务 |
| 3 | NVIDIA runtime | `docker info` 输出含 `nvidia` | 仅日志提示（未配置时可能 --gpus all 失败） |
| 4 | 镜像存在 | `docker image inspect <image>` | 不存在 → **自动 `docker pull`**（进度逐行转发 `[docker pull]` 到 model-log）；**直接拉取失败自动回退国内镜像源**：`docker.1ms.run` → `docker.m.daocloud.io` → `docker.xuanyuan.me` → `hub.rat.dev`，任一成功则用镜像源前缀镜像启动（如 `docker.1ms.run/lmsysorg/sglang:v0.5.17`）；全部失败提示手动 pull |
| 5 | 端口未被占用 | `TcpListener::bind("0.0.0.0:<port>")` | 端口已被占用，请换端口/关进程 |

预检通过后才清理同名残留容器并启动。镜像源列表维护在 `model_list.rs` 的 `check_docker_env`（`attempts` 数组）：Docker Hub 被墙/超时时自动按序回退，无需用户配置；若用户设置页镜像已带 registry 域名（如 `docker.1ms.run/...`），不再套前缀。

> **镜像源提示**：上述国内源列表时效性强（部分可能失效），失效时可在设置页把镜像直接改成可用源前缀，或在 `/etc/docker/daemon.json` 配 `registry-mirrors` 后 `sudo systemctl restart docker`。

### 6.1 生成的 docker run 命令

```bash
docker run \
  --name adm-sglang-<model_id> \
  --gpus all \
  --shm-size 64g \
  -p <port>:<port> \
  -v <data_dir>/models/<model_id>:/models/<model_id>:ro \
  lmsysorg/sglang:v0.5.17 \
  python3 -m sglang.launch_server \
    --model-path /models/<model_id> \
    --host 0.0.0.0 \
    --port <port> \
    [--context-length N] \
    [--tensor-parallel-size N] \
    [--mem-fraction-static 0.xx] \
    [--dtype ...] \
    [--quantization ...] \
    [--kv-cache-dtype ...] \
    [--schedule-policy ...] \
    [--max-running-requests N] \
    [--max-queued-requests N] \
    [--chunked-prefill-size N] \
    [--log-level ...] [--log-requests] [--enable-metrics] \
    [--reasoning-parser ...] [--tool-call-parser ...] \
    [--<extra_key> <extra_value> ...]
```

要点：
- **容器内 host 固定 `0.0.0.0`**（`-p` 端口映射要求容器内监听全接口），设置页 host 对 Docker 不生效
- 端口沿用设置页 `port`（默认 5678）
- 仅非空/非默认值的参数才拼入命令（避免污染 SGLang 默认行为）
- 额外参数（`extra_args`，每行 `key=value`）原样追加为 `--key value`，`#` 行忽略
- **MTP 自动启用**：模型目录含 MTP 权重（文件名含 `mtp` 且以 `.safetensors`/`.bin` 结尾，如 `model_mtp.safetensors`）时，自动追加 `--speculative-algorithm NEXTN --speculative-num-steps 3`（Qwen3-Next MTP 官方用法，草稿权重与主模型同目录自动加载）；用户已在 `extra_args` 自定义 `speculative-algorithm` 时跳过，可用 `speculative-algorithm=NONE` 显式关闭

### 6.2 就绪与状态

- 就绪信号：stdout 出现 `Uvicorn running on` 或 `The server is fired up and ready to rock!` → 前端 emit `model-started`
- 日志：stdout/stderr 逐行转发 `model-log` 事件（模型卡片/控制台可见）
- 运行记录：`AppState.running_process`（docker run 进程 PID）+ `running_container`（容器名）

### 6.3 停止与清理

- `stop_model`：`docker stop -t 5 <容器>` → `docker rm -f <容器>` → 兜底杀进程树
- 容器自然退出（崩溃/被外部停止）：后台线程 `docker rm -f` + 清除 AppState + emit `model-stopped`
- 应用退出（托盘/窗口关闭/ExitRequested）：`cleanup_processes` 统一 `docker stop/rm`
- 启动前会先 `docker rm -f` 同名残留容器，保证幂等

## 7. 设置页参数（config.json → `sglang_args`）

存储位置：`<data_dir>/config.json` 的 `sglang_args` 对象（`save_settings` 持久化）。

| 表单字段 | 配置 key | 默认 | 说明 |
|---|---|---|---|
| 镜像 | `image` | `lmsysorg/sglang:v0.5.17` | 固定版本 tag |
| 共享内存 | `shm_size` | `64g`（dgx-spark）/ `32g`（其他） | docker --shm-size |
| 上下文大小 | `context_length` | 0 = 模型默认 | 优先取 launch_params.ctx_size |
| 张量并行 | `tensor_parallel_size` | 1 | >1 才传 |
| 静态内存占比 | `mem_fraction_static` | 0 = 自动 | OOM 调小 |
| 数据类型 | `dtype` | 空 = auto | half/float16/bfloat16/float/float32 |
| 量化方法 | `quantization` | 空 = 自动识别 | fp8/modelopt_fp8/modelopt_fp4/nvfp4_online/awq/gptq/w8a8_fp8 |
| KV Cache 类型 | `kv_cache_dtype` | 空 = auto | fp8_e5m2/fp8_e4m3/bf16/nvfp4 |
| 调度策略 | `schedule_policy` | 空 = fcfs | lpm/random/dfs-weight/lof/priority |
| 最大运行请求数 | `max_running_requests` | 0 = 自动 | |
| 最大排队请求数 | `max_queued_requests` | 0 = 自动 | |
| Chunked Prefill | `chunked_prefill_size` | 0 = 自动 | -1 禁用；长提示 OOM 调小如 4096 |
| 日志级别 | `log_level` | 空 = info | debug/warning/error |
| 请求日志 | `log_requests` | false | bool flag |
| 监控指标 | `enable_metrics` | false | Prometheus |
| 推理解析器 | `reasoning_parser` | 空 | deepseek-r1/deepseek-v3/glm45/qwen3/qwen3-thinking/kimi |
| 工具解析器 | `tool_call_parser` | 空 | qwen/qwen25/qwen3_coder/deepseekv3/deepseekv31/glm/llama3/mistral |
| 额外参数 | `extra_args` | 空 | 每行 `key=value` → `--key value` |

MTP（Multi-Token Prediction）：模型目录有 `model_mtp.safetensors` 时自动启用 NEXTN 投机解码（见 §6.1），无需手动配置；如 MTP 启动异常，在 `extra_args` 写 `speculative-algorithm=NONE` 关闭，或按官方文档调整 `speculative-num-steps`。

改动保存后对**后续启动**生效（无需重启应用；正在运行的容器需手动重启）。

## 8. 参数参考（官方文档精选）

完整列表：https://docs.sglang.io/docs/advanced_features/server_arguments
（文档说 `python3 -m sglang.launch_server --help` 可看全部参数；也支持 `--config config.yaml`）

常用场景速查：
- **KV 缓存 OOM**：`--mem-fraction-static 0.7` 或 `--kv-cache-dtype fp8_e4m3`
- **长提示词 prefill OOM**：`--chunked-prefill-size 4096`
- **多 GPU**：`--tensor-parallel-size 2`（peer access 报错加 `--enable-p2p-check`）
- **多卡数据并行（吞吐）**：`--dp 2`（可与 tp 组合）
- **FP8 KV 量化**：`--kv-cache-dtype fp8_e4m3` / `fp8_e5m2`
- **推理模型思考内容分离**：`--reasoning-parser qwen3`（或 deepseek-r1 等）
- **工具调用**：`--tool-call-parser qwen`（模型族匹配）
- **确定性推理**：`--enable-deterministic-inference`
- **自定义聊天模板**：`--hf-chat-template-name tool_use`（tokenizer 含多模板时选）

## 9. 常见问题排查

| 现象 | 排查方向 |
|---|---|
| `docker 未运行或不可用` | 启动前已做预检：依次确认 `docker --version`、`docker info`、镜像存在（缺失自动 pull）、端口未占用 |
| 启动即退 / 疯狂重启 | `model-log` 看 stderr（含 `[docker pull]` 转发）：拉取失败则先手动 `docker pull lmsysorg/sglang:v0.5.17` |
| `The platform 'linux/amd64' could not be found` | 镜像无对应架构层：DGX Spark 是 arm64，确认 tag 为多架构版本（v0.5.17 含 arm64） |
| NVFP4 模型 CUDA 崩溃 | GB10 上 NVFP4 已知问题，换 FP8 权重 |
| 端口占用 | `-p <port>:<port>` 冲突：改设置页端口或停掉占用进程 |
| 模型没下载完就点启动 | `is_dir_model` 要求 `.done` 或 config.json+model.safetensors 齐全 |
| 前端一直不出现「查看模型」 | 就绪信号匹配 `Uvicorn running on`；先看 model-log 是否有该行 |
| 想改参数不生效 | 参数持久化在 config.json；已运行容器需 stop 后重新 start |