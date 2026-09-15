// ===== 轻量 i18n：原文即 key =====
// 无框架 / 无打包工具，直接以中文原文作为词典 key。
// 未命中 key 时原样返回中文，保证零遗漏风险（漏译只影响英文用户）。
// 语言持久化：localStorage["adm_lang"]，与 Rust Settings.language 同步。

const EN = {
  // ===== 壳层 index.html =====
  "首页": "Home",
  "设置": "Settings",
  "内存": "Memory",
  "显存": "VRAM",
  "无显卡": "No GPU",
  "加载失败": "Load failed",
  "页面加载中...": "Loading page...",
  "页面加载失败: ": "Failed to load page: ",
  "正在检查更新...": "Checking for updates...",
  "发现新版本": "New version available",
  "有新版本可用，是否前往下载？": "A new version is available. Download it now?",
  "当前版本": "Current version",
  "最新版本": "Latest version",
  "下载更新": "Download update",
  "稍后再说": "Later",
  "已是最新版本": "You are up to date",
  "当前版本 v": "You are on the latest version v",
  "已是最新": "latest",
  "检查更新失败": "Update check failed",
  "下载安装": "Download & Install",
  "请完成安装": "Complete the installation",
  "安装完成": "Done",
  "无法获取下载地址": "Unable to get download URL",
  "关闭": "Close",
  "开始下载": "Start download",
  "准备下载...": "Preparing download...",
  "下载失败: 缺少下载地址": "Download failed: missing download URL",
  "重试": "Retry",
  "下载失败: ": "Download failed: ",
  "本地模型请手动放置文件并写入 .done 标记后点击刷新，无需下载": "For local models, place the files manually and create a .done marker, then refresh; no download needed",
  "正在下载... ": "Downloading... ",
  "正在解压安装...": "Extracting and installing...",
  "安装完成！": "Installation complete!",
  "该功能即将开放，敬请期待": "This feature is coming soon",
  "当前仅支持 Apple Silicon (M 系列) Mac": "Currently only supported on Apple Silicon (M-series) Macs",
  "未安装": "Not installed",
  "已加入 docker 组，注销重新登录后重启 ADM-BE 即可免 sudo 执行 docker": "Added to the docker group; after logging out and back in, restart ADM-BE to use docker without sudo",

  // ===== settings.js =====
  "返回": "Back",
  "模型启动参数": "Model Launch Params",
  "外观主题": "Appearance",
  "多机互联": "Multi-Node",
  "DGX直连配置": "DGX Direct Connect",
  "DGX-Spark 双机直连 · 一键从零部署": "DGX-Spark Direct Link · One-Click Setup from Zero",
  "A 为控制机，B 为对等节点": "A = controller, B = peer node",
  "⚠ 准备提示": "⚠ Prep note",
  "直连线型号：200G QSFP56 DAC 直连铜缆（DGX-Spark 网口是 QSFP56（200G），不是 QSFP112（400G），不要买错，买 1 条就够——插 2 条受内存带宽限制提升不大，反而增加配置复杂度）。插好直连线后 A、B 必须插同一方向的口（如都插左边），插口方向不一致会自动检测到并报错。": "Cable: 200G QSFP56 DAC direct-attach copper (DGX-Spark port is QSFP56 (200G), NOT QSFP112 (400G); 1 cable is enough — 2 cables add complexity). Plug A and B into the SAME-side port (e.g. both left); mismatched sides are detected and rejected.",
  "A 控制机（本机）": "A Controller (this machine)",
  "B 对等节点": "B Peer node",
  "当前可达地址（管理网 IP）": "Currently reachable address (mgmt IP)",
  "开始一键部署": "Start one-click setup",
  "部署中...": "Deploying...",
  "点击「开始一键部署」后在此显示运行日志": "Click \"Start one-click setup\" to show the runtime log here",
  "部署中，请勿关闭窗口...": "Deploying, do not close the window...",
  "✔ 部署完成！多机互联节点表已就绪，可直接使用": "✔ Deploy done! Multi-node table is ready",
  "==== 开始一键部署 ====": "==== Start one-click setup ====",
  "==== 全部完成 ====": "==== All done ====",
  "请填写 A 控制机用户名和密码": "Please fill in A username and password",
  "请填写 B 对等节点地址、用户名和密码": "Please fill in B address, username and password",
  "部署失败：": "Deploy failed: ",
  "用户名": "Username",
  "输入密码": "Enter password",
  "IP 地址，例如 192.168.1.X": "IP address, e.g. 192.168.1.X",
  "免密 SSH 配置": "Passwordless SSH",
  "光口探测": "Optical port detect",
  "固定 IP 配置": "Static IP config",
  "连通性检测": "Connectivity check",
  "Docker 检测安装": "Docker check/install",
  "Docker 组权限": "Docker group",
  "写回多机互联节点表": "Write back multi-node table",
  "失败：": "Failed: ",
  "回滚：": "Rollback: ",
  "回滚完成": "Rollback done",
  "已自动回滚，可修复问题后重新执行": "Auto-rolled back; fix and re-run",
  "部署失败，已自动回滚（详见日志）": "Deploy failed, auto-rolled back (see log)",
  "启用多机模式": "Enable multi-node mode",
  "节点清单": "Node List",
  "互联参数": "Interconnect",
  "引导端口": "Bootstrap port",
  "NCCL 端口": "NCCL port",
  "互连网卡": "Interconnect NIC",
  "主节点（本机）": "Master node (this machine)",
  "管理网卡": "Management NIC",
  "ConnectX-7 网卡": "ConnectX-7 NIC",
  "SSH 密钥": "SSH key",
  "生成 / 查看 SSH Key": "Generate / view SSH key",
  "公钥（加入各远端节点 ~/.ssh/authorized_keys）": "Public key (add to each remote's ~/.ssh/authorized_keys)",
  "复制": "Copy",
  "已生成": "generated (new)",
  "已存在": "already exists",
  "未生成": "Not generated",
  "自动检测中...": "Detecting...",
  "未检测到": "Not detected",
  "未检测到 ConnectX-7": "No ConnectX-7 NIC detected",
  "无 IP": "no IP",
  "已复制公钥": "Public key copied",
  "SSH Key 生成失败: ": "SSH key generation failed: ",
  "SSH 私钥": "SSH private key",
  "添加节点": "Add node",
  "全部测试": "Test all",
  "测试": "Test",
  "未测试": "Not tested",
  "测试中...": "Testing...",
  "本机": "This machine",
  "正常": "OK",
  "失败": "Failed",
  "模型未同步，请先同步模型": "Model not synced, sync it first",
  "镜像缺失": "Image missing",
  "仅 rank 0（第 1 行）可为本机": "Only rank 0 (first row) can be the local machine",
  "至少保留 1 个节点（本机）": "Keep at least 1 node (local machine)",
  "请先添加远端节点": "Add remote nodes first",
  "测试连通": "Test connectivity",
  "推送镜像": "Push image",
  "推送失败: ": "Push failed: ",
  "导出中...": "Exporting...",
  "传输中...": "Transferring...",
  "导入中...": "Importing...",
  "推送中...": "Pushing...",
  "请先在模型启动参数中选择镜像": "Select an image in Model Launch Params first",
  "同步镜像到直连节点": "Sync image to direct node",
  "同步模型到直连节点": "Sync model to direct node",
  "镜像同步中...": "Syncing image...",
  "模型同步中...": "Syncing model...",
  "正在同步": "Syncing",
  "选择本地模型": "Select local model",
  "同步中...": "Syncing...",
  "同步失败: ": "Sync failed: ",
  "请选择要同步的本地模型": "Select a local model to sync",
  "请先在直连节点行填写模型目录": "Fill in the model directory on the direct node row first",
  "测试 = SSH 检查远端 Docker / GPU / 模型目录": "Test = SSH checks remote Docker / GPU / model directory",
  "系统版本号": "System Version",
  "关于": "About",
  "推荐模式": "Recommended Mode",
  "选择模式": "Mode",
  "快速配置": "Quick config",
  "默认（日常聊天）": "Default (chat)",
  "创意写作": "Creative writing",
  "写代码 / 编程（推荐）": "Coding (recommended)",
  "选择后自动填充并保存采样参数，可手动微调后自动保存": "Selecting auto-fills and saves sampling params; tweaks are saved automatically",
  "基础参数": "Basic Params",
  "上下文大小": "Context size",
  "预测 token 数": "Tokens to predict",
  "-1 表示无限": "-1 means unlimited",
  "批处理大小": "Batch size",
  "微批次大小": "Micro batch size",
  "GPU 参数": "GPU Params",
  "GPU 层数": "GPU layers",
  "auto (自动)": "auto (automatic)",
  "all (全部)": "all",
  "0 (仅 CPU)": "0 (CPU only)",
  "自定义": "Custom",
  "自定义 GPU 层数": "Custom GPU layers",
  "性能参数": "Performance Params",
  "线程数": "Threads",
  "自动": "auto",
  "留空为自动": "Leave empty for auto",
  "批处理线程数": "Batch threads",
  "同线程数": "Same as threads",
  "KV 缓存类型 K": "KV cache type K",
  "KV 缓存类型 V": "KV cache type V",
  "内存锁定": "Memory lock",
  "强制模型驻留 RAM": "Lock model in RAM",
  "内存映射": "Memory map",
  "启用内存映射": "Enable mmap",
  "采样参数": "Sampling Params",
  "温度": "Temperature",
  "重复惩罚": "Repeat penalty",
  "重复窗口": "Repeat window",
  "重复惩罚的上下文窗口大小，-1 表示使用 ctx_size": "Context window for repeat penalty; -1 uses ctx_size",
  "DRY 乘数": "DRY multiplier",
  "DRY 采样乘数，0.0 表示禁用": "DRY sampling multiplier; 0.0 disables it",
  "DRY 允许长度": "DRY allowed length",
  "DRY 采样允许的重复长度，代码模式建议设为 1": "Allowed repeated length for DRY; 1 is recommended for coding",
  "DRY 惩罚窗口": "DRY penalty window",
  "DRY 惩罚的最后 n 个 token，-1 表示使用上下文大小": "Last n tokens for DRY penalty; -1 uses context size",
  "存在惩罚": "Presence penalty",
  "重复 alpha 存在惩罚，0.0 表示禁用": "Repeat alpha presence penalty; 0.0 disables it",
  "频率惩罚": "Frequency penalty",
  "重复 alpha 频率惩罚，0.0 表示禁用": "Repeat alpha frequency penalty; 0.0 disables it",
  "推理/思考": "Reasoning",
  "auto (自动检测)": "auto (detect)",
  "on (开启)": "on",
  "off (关闭)": "off",
  "控制模型是否启用推理/思考模式": "Controls whether the model uses reasoning mode",
  "服务参数": "Server Params",
  "监听端口": "Listen port",
  "监听地址": "Listen address",
  "127.0.0.1 (本地)": "127.0.0.1 (local)",
  "0.0.0.0 (所有接口)": "0.0.0.0 (all interfaces)",
  "恢复默认": "Reset to default",
  "主题配色": "Theme",
  "选择界面配色，风格参考主流 VS Code 主题，点击即可一键切换（立即生效）": "Pick a color scheme inspired by popular VS Code themes; click to switch instantly",
  "ADM-BE 版本": "ADM-BE version",
  "检测中...": "Checking...",
  "✓ 最新": "✓ Latest",
  "检查新版本": "Check for updates",
  "Tauri 版本": "Tauri version",
  "llama.cpp 版本": "llama.cpp version",
  "删除": "Delete",
  "操作系统": "OS",
  "ADM-BE 是一个大模型部署图形化管理工具，让用户能够便捷地在本地部署和运行大语言模型。": "ADM-BE is a GUI management tool for local LLM deployment, making it easy to deploy and run large language models on your own machine.",
  "如需定制服务 联系方式：微信: litai686": "Custom services: WeChat litai686",
  "项目官网：": "Website: ",
  "删除提示": "Delete Confirmation",
  "当前模式上下文大小不能低于 ": "Context size for this mode cannot be below ",
  "当前模式最小 ": "Minimum for this mode: ",
  "设置已保存，重启模型后生效": "Settings saved; restart the model to apply",
  "保存失败: ": "Save failed: ",
  "未知": "Unknown",
  "未安装或无法检测": "Not installed or undetectable",
  "确定要删除模型文件夹吗？删除后需要重新下载才能使用相关功能。": "Delete the model folder? You will need to re-download it to use related features.",
  "删除失败: ": "Delete failed: ",
  "检查中...": "Checking...",
  "发现新版本 v": "New version v",
  "检查失败: ": "Check failed: ",
  "已切换主题": "Theme applied",

  // ===== settings.js - 网络代理 =====

  // ===== model_list.js =====
  "模型列表": "Model List",
  "模型类型": "Model Type",
  "全部模型": "All Models",
  "正在加载模型列表...": "Loading model list...",
  "确认删除": "Confirm Delete",
  "确定要删除此模型吗？删除后无法恢复。": "Delete this model? This cannot be undone.",
  "确认删除按钮": "Delete",
  "暂无可用模型": "No models available",
  "已启动": "Running",
  "可用": "Available",
  "不可用": "Unavailable",
  "继续下载": "Resume",
  "下载": "Download",
  "停止下载": "Stop",
  "已停止下载": "Download stopped",
  "停止下载失败: ": "Stop download failed: ",
  "查看模型": "View Model",
  "关闭模型": "Stop Model",
  "推理": "Reasoning",
  "图片识别": "Vision",
  "音视频生成": "Audio-video generation",
  " · 需内存 ": " · RAM: ",
  " GB": " GB",
  "继续下载中...": "Resuming...",
  "启动中...": "Starting...",
  "启动失败: ": "Start failed: ",
  "镜像拉取失败：": "Image pull failed: ",
  "；请检查网络或手动 docker pull，启动前需先补拉镜像": "; check the network or run `docker pull` manually; the image must be present before starting",
  "停止失败: ": "Stop failed: ",
  "停止中...": "Stopping...",
  "确定要删除模型 \"": "Delete model \"",
  "\" 吗？删除后无法恢复。": "\"? This cannot be undone.",
  "下载失败 [": "Download failed [",
  "]: ": "]: ",
  "模型错误 [": "Model error [",
  "获取模型列表失败: ": "Failed to fetch model list: ",

  "需内存 ": "RAM needed: ",
  "语言 / Language": "Language",
  "界面语言": "UI Language",
  "切换后立即生效": "Applies immediately after switching",
  "工具调用": "Tool use",
  "取消": "Cancel",
  "确定": "OK",
  "启动": "Start",
  "适配机型": "Device",
  "全部机型": "All devices",
  "拉取镜像 ": "Pulling image ",
  "（校验/解压中...）": " (verifying/extracting...)",
  "切换镜像源重试中...": "Retrying with next mirror...",
  "启动中": "Starting",
  "推理引擎": "Inference Engine",
  "模型最大上下文长度，默认 256000": "Max context length; defaults to 256000",
  "Docker 端口映射（容器内始终监听 0.0.0.0）": "Docker port mapping (container always listens on 0.0.0.0)",
  "Docker 部署": "Docker Deployment",
  "镜像": "Image",
  "共享内存": "Shared Memory",
  "DGX Spark 建议 64g，其他机型 32g": "64g recommended for DGX Spark, 32g otherwise",
  "推理参数": "Inference",
  "张量并行": "Tensor Parallelism",
  "静态内存占比": "Static Memory Fraction",
  "0 = 自动；KV 缓存池/权重内存占比，OOM 时调小": "0 = auto; KV cache/weights memory fraction, lower when OOM",
  "单机多卡时拆分模型权重到多张 GPU（如 2 张卡填 2）；单卡填 1。2 台 Spark 跨机部署需要多节点模式，当前版本暂不支持": "Split model weights across multiple GPUs on one machine (e.g. set 2 for 2 GPUs); set 1 for single GPU. Multi-node across 2 Spark devices requires distributed mode, not supported in this version.",
  "数据类型": "Data Type",
  "auto（默认）": "auto (default)",
  "量化方法": "Quantization",
  "不指定（NVFP4/FP8 模型自动从 config 解析）": "Not set (NVFP4/FP8 models auto-detected from config)",
  "预量化模型（如 unsloth NVFP4）无需指定，加载时自动识别": "Pre-quantized models (e.g. unsloth NVFP4) need no flag; auto-detected",
  "KV Cache 与调度": "KV Cache & Scheduling",
  "KV Cache 类型": "KV Cache Dtype",
  "调度策略": "Schedule Policy",
  "fcfs（默认）": "fcfs (default)",
  "最大运行请求数": "Max Running Requests",
  "0 = 自动": "0 = auto",
  "最大排队请求数": "Max Queued Requests",
  "Chunked Prefill": "Chunked Prefill",
  "0 = 自动，-1 = 禁用；长提示词 OOM 时调小（如 4096）": "0 = auto, -1 = disabled; lower (e.g. 4096) on long-prompt OOM",
  "模型解析与日志": "Parsers & Logging",
  "日志级别": "Log Level",
  "info（默认）": "info (default)",
  "请求日志": "Request Logging",
  "记录所有请求的元数据/输入/输出": "Log metadata/inputs/outputs of all requests",
  "监控指标": "Metrics",
  "启动 Prometheus metrics": "Enable Prometheus metrics",
  "高级自定义参数": "Advanced Custom Args",
  "额外参数": "Extra Args",
  "每行一个 key=value，启动时拼成 --key value 追加到命令尾部；布尔值写 true/false；# 开头为注释": "One key=value per line, appended as --key value; booleans as true/false; lines starting with # are comments",
  "完整参数说明见 eugr/spark-vllm-docker 社区 fork（针对 DGX Spark 优化）：https://github.com/eugr/spark-vllm-docker": "Full argument reference: eugr/spark-vllm-docker community fork (optimized for DGX Spark): https://github.com/eugr/spark-vllm-docker",

  // ===== settings.js - vLLM 基础参数面板 =====
  "vLLM 基础参数": "vLLM Basic",
  "分布式后端": "Distributed Executor Backend",
  "auto（单机）": "auto (single node)",
  "ray（多机）": "ray (multi-node)",
  "加载格式": "Load Format",
  "不指定（vLLM 默认）": "Not set (vLLM default)",
  "Block Size": "Block Size",
  "Tokenizer 模式": "Tokenizer Mode",
  "工具解析器": "Tool Call Parser",
  "推理解析器": "Reasoning Parser",
  "自动工具选择": "Enable Auto Tool Choice",
  "信任远程代码": "Trust Remote Code",

  // ===== settings.js - Docker 镜像配置 =====
  "Docker 镜像配置": "Docker Mirror Config",
  "镜像加速（registry-mirrors）": "Mirror acceleration (registry-mirrors)",
  "写回 Docker daemon.json 的 registry-mirrors 实现国内镜像加速（仅影响后续镜像拉取），保存后自动重启 Docker 服务使配置生效": "Writes registry-mirrors into Docker daemon.json for mirror acceleration (affects future pulls only); Docker is restarted automatically to apply",
  "配置文件": "Config file",
  "加速器地址": "Mirror URLs",
  "每行一个加速器地址，留空表示直连 Docker Hub": "One mirror URL per line; empty means direct Docker Hub",
  "保存并重启 Docker": "Save & Restart Docker",
  "（不存在，保存时将新建）": " (missing, will be created on save)",
  "读取失败: ": "Failed to read: ",
  "正在写入配置并重启 Docker，请稍候...": "Writing config and restarting Docker, please wait...",
  "配置已写入，Docker 已重启": "Config saved, Docker restarted",
  "镜像加速配置已生效": "Mirror acceleration is now active",
  "需要权限，请手动执行": "Admin privileges required, please run manually",
  "需要管理员权限手动执行以下命令：\n\n": "Run the following commands as administrator:\n\n",
  "已取消": "Cancelled",

  // ===== settings.js - 代理 =====
  "代理": "Proxy",
  "本地代理": "Local Proxy",
  "代理地址": "Proxy URL",
  "模型文件下载（HF 模型）走该代理，填写后保存即生效；留空 = 直连。仅支持 HTTP(S) 代理地址": "Model downloads (HF) go through this proxy and take effect on save; empty = direct connection. HTTP(S) proxy URLs only",
  "Docker 镜像拉取": "Docker Image Pull",
  "Docker 镜像由 Docker 守护进程下载，需把代理写入 daemon.json 的 proxies 配置并重启 Docker 服务后生效（镜像拉取期间不要关闭代理软件）": "Images are pulled by the Docker daemon; write the proxy into daemon.json proxies and restart Docker to apply (keep your proxy software running while pulling)",
  "当前 daemon 代理: ": "Current daemon proxy: ",
  "代理配置已生效，镜像拉取将走代理": "Proxy applied, image pulls will go through the proxy",
  "已清除 Docker 代理配置": "Docker proxy config cleared",

  // ===== settings.js - 运行日志 =====
  "选择日期": "Date",
  "刷新": "Refresh",
  "打开日志目录": "Open Log Folder",
  "暂无日志": "No logs",
  "加载中...": "Loading...",
  "加载失败: ": "Failed to load: ",
  "打开失败: ": "Failed to open: ",

  // ===== model_list.js - Docker 权限修复 =====
  "Docker 权限不足": "Docker Permission Required",
  "当前用户不在 docker 组，无法启动模型。点击下方按钮，系统会弹出密码框自动修复权限，修复后需重启 ADM-BE 生效。": "The current user is not in the docker group. Click the button below to fix it via a system password prompt. Restart ADM-BE after fixing.",
  "一键修复": "Auto Fix",
  "修复中...": "Fixing...",
  "已修复，请重启": "Fixed, please restart",
  "修复失败: ": "Fix failed: ",
  "已取消，可重新点击修复": "Cancelled, click fix to retry",
  "系统不支持自动修复，请在终端手动执行以下命令，然后重新登录后重启 ADM-BE：": "Auto-fix is not supported on this system. Please run the following command in a terminal, then re-login and restart ADM-BE:",
  "权限修复成功，系统即将自动注销登录，重新登录后即可使用。": "Permission fixed. The system will log out automatically. Please log back in to continue.",
  "清空日志": "Clear Logs",
  "确定清空所有日志吗？": "Are you sure you want to clear all logs?",
  "日志已清空": "Logs cleared",
  "清空失败: ": "Failed to clear: ",

  // ===== video.js（视频生成页 / ComfyUI）=====
  "视频生成": "Video Generation",
  "状态": "Status",
  "地址": "Address",
  "端口": "Port",
  "容器": "Container",
  "挂载": "Mounts",
  "保存": "Save",
  "未启动": "Not running",
  "运行中": "Running",
  "启动 ComfyUI": "Start ComfyUI",
  "停止": "Stop",
  "重启": "Restart",
  "打开 WebUI": "Open WebUI",
  "复制产物路径": "Copy output path",
  "停止当前模型并启动 ComfyUI": "Stop current model and start ComfyUI",
  "首次使用指引": "First-time guide",
  "点击「打开 WebUI」→ 模板库 → Video → MiniMax H3 T2V / I2V / R2V": "Click \"Open WebUI\" → Template Library → Video → MiniMax H3 T2V / I2V / R2V",
  "跟随弹窗下载权重（NVFP4-AWQ 编码器 + 量化 DiT，约 44 GB，仅一次）": "Follow the popup to download weights (NVFP4-AWQ encoder + quantized DiT, ~44 GB, one time only)",
  "首次建议 480P / 5 秒跑通；低延迟可接 turbo 8step LoRA（步数 8）": "Start with 480P / 5 s; for lower latency attach the turbo 8step LoRA (8 steps)",
  "与 SGLang 模型互斥：128 GB 统一内存不能同时驻留两套权重": "Mutually exclusive with SGLang models: 128 GB unified memory cannot hold both weight sets",
  "日志": "Logs",
  "清空": "Clear",
  "未配置 ComfyUI 条目（检查远程 model.json）": "No ComfyUI entry configured (check remote model.json)",
  "未配置附加挂载（权重与产物将落在容器内）": "No extra mounts configured (weights and outputs will live inside the container)",
  "当前有模型正在运行（": "Another model is running (",
  "），128 GB 统一内存不能同时驻留两套 H3 权重，请先停止它": "); 128 GB unified memory cannot hold two H3 weight sets — stop it first",
  "请先停止当前模型再启动 ComfyUI": "Stop the current model before starting ComfyUI",
  "未找到产物目录挂载": "Output directory mount not found",
  "已复制: ": "Copied: ",
  "端口需在 1024-65535 之间": "Port must be between 1024 and 65535",
  "端口已保存，重启 ComfyUI 后生效": "Port saved; restart ComfyUI to apply",
  "刷新状态": "Refresh",
  "状态已刷新": "Status refreshed",
  "环境准备（首次使用）": "Setup (first run)",
  "已构建": "Built",
  "未构建": "Not built",
  "构建镜像": "Build image",
  "构建中...": "Building...",
  "构建失败: ": "Build failed: ",
  "已下载": "Downloaded",
  "未下载": "Not downloaded",
  "（首次约 67 GB）": " (≈67 GB on first run)",
  "下载权重": "Download weights",
  "下载中": "Downloading",
  "已请求停止下载": "Stop requested",
  "模型清单未配置 model_download_files": "model_download_files is not configured in the model manifest",
  "未配置 engine_image": "engine_image is not configured",
  "镜像构建完成": "Image build finished",
  "需先构建镜像": "Build the image first",
  "权重目录": "Weights dir",
  "构建目录": "Build dir",

  // ===== test.js =====
  "模型测试": "Model Test",
  "请先在首页启动模型，启动后即可在此对话测试": "Start a model on the Home page first — you can chat with it here afterwards.",
  "测试页面加载中...": "Loading test page...",
  "测试页面加载失败: ": "Failed to load test page: ",
  "在浏览器中打开": "Open in browser",
  "当前模型为音视频生成模型，请通过视频生成 API 调用：": "This is an audio-video generation model. Call the video generation API instead: ",
};

let lang = "zh";

/** @param {string} l */
function normalize(l) {
  if (l === "en" || l === "zh") return l;
  return "zh";
}

/** @param {string} l @param {boolean} [persist] 是否写入 localStorage（默认 true；系统语言检测路径传 false，避免固化检测结果） */
export function setLanguage(l, persist) {
  lang = normalize(l);
  if (persist !== false) {
    try { localStorage.setItem("adm_lang", lang); } catch (_) {}
  }
  try { document.documentElement.lang = lang === "en" ? "en" : "zh-CN"; } catch (_) {}
}

export function getLanguage() {
  return lang;
}

// 检测系统语言：仅用于「用户从未显式选择」时的默认值。
// WebView（WebView2 / WKWebView）的 navigator.language 跟随操作系统显示语言。
/** @returns {"zh" | "en"} */
export function detectSystemLanguage() {
  try {
    const navLang = String(navigator.language || (navigator.languages && navigator.languages[0]) || "").toLowerCase();
    if (navLang.startsWith("zh")) return "zh";
    return "en";
  } catch (_) {
    return "zh";
  }
}

// 同步语言优先级：显式设置（Settings.language）> localStorage > 系统语言检测。
// 检测结果不写入 localStorage：用户从未显式选择时，每次启动跟随系统语言。
export function syncLanguageFromSettings(settings) {
  let l = null;
  if (settings && settings.language) l = settings.language;
  if (!l) { try { l = localStorage.getItem("adm_lang"); } catch (_) {} }
  if (!l) { setLanguage(detectSystemLanguage(), false); return; }
  setLanguage(l);
}

/** @param {string} str @returns {string} */
export function t(str) {
  if (lang === "zh") return str;
  if (str == null) return str;
  const v = EN[str];
  return v !== undefined ? v : str;
}

// 带变量替换：tV("下载失败: {err}", { err: e })
/** @param {string} str @param {Object<string,string|number>|undefined} vars @returns {string} */
export function tV(str, vars) {
  let out = t(str);
  if (vars) {
    for (const k of Object.keys(vars)) {
      out = out.replace(new RegExp("\\{" + k + "\\}", "g"), String(vars[k]));
    }
  }
  return out;
}

// 将根节点下所有带 data-i18n 的元素文本替换为当前语言
export function applyToDOM(root) {
  if (!root) return;
  const nodes = root.querySelectorAll ? root.querySelectorAll("[data-i18n]") : [];
  nodes.forEach((el) => {
    const key = el.getAttribute("data-i18n");
    if (key) el.textContent = t(key);
  });
  const placeholders = root.querySelectorAll ? root.querySelectorAll("[data-i18n-placeholder]") : [];
  placeholders.forEach((el) => {
    const key = el.getAttribute("data-i18n-placeholder");
    if (key) el.setAttribute("placeholder", t(key));
  });
}

// 挂到全局，供 index.html 壳层（非 ESM 上下文）使用
if (typeof window !== "undefined") {
  // 类型注释放宽：window.ADM 的形状由 types.d.ts 声明（含 showUpdateDialog 等），
  // 这里只合并注入 i18n，运行时由 index.html 先定义 ADM 再被本模块增强
  window.ADM = /** @type {any} */ (Object.assign(window.ADM || {}, {
    i18n: { t, tV, setLanguage, getLanguage, syncLanguageFromSettings, applyToDOM },
  }));
}

export default { t, tV, setLanguage, getLanguage, syncLanguageFromSettings, applyToDOM };
