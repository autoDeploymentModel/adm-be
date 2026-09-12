// @ts-nocheck -- 历史视图暂未类型化（jsconfig checkJs 全局开启，新代码请勿加此标记）
import { t as _t, setLanguage, getLanguage } from "../i18n.js";

// 默认国内镜像加速器（无已有配置时填入，同时也作为输入框 placeholder 的单一来源）
const DEFAULT_DOCKER_MIRRORS = [
  "https://docker.nju.edu.cn",
  "https://docker.m.daocloud.io",
  "https://docker.1ms.run",
  "https://docker.1panel.live",
];

const template = `
<style>
  /* 样式隔离约定：选择器限定在本视图容器内（id/class 带 settings- 前缀），
     全局 reset（*）与 body 样式由 index.html 壳层统一提供，视图内不得重复定义 */

  #settings-app { display: flex; flex-direction: column; height: 100%; }

  #settings-header {
    display: flex;
    align-items: center;
    padding: 10px 16px;
    background: var(--c-panel-2);
    border-bottom: 1px solid var(--c-border-soft);
    flex-shrink: 0;
    gap: 12px;
  }

  .back-btn {
    background: var(--c-overlay);
    border: none;
    color: var(--c-text);
    padding: 6px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
    transition: background 0.2s;
  }
  .back-btn:hover { background: var(--c-overlay-strong); }

  #settings-header .title { font-size: 16px; font-weight: 600; color: var(--c-text-hi); }

  #settings-layout { display: flex; flex: 1; overflow: hidden; }

  #settings-nav {
    width: 180px;
    background: var(--c-panel);
    border-right: 1px solid var(--c-border-soft);
    padding: 12px 0;
    flex-shrink: 0;
    overflow-y: auto;
  }

  .nav-item {
    padding: 10px 20px;
    cursor: pointer;
    font-size: 14px;
    color: var(--c-text-2);
    transition: all 0.2s;
    border-left: 3px solid transparent;
  }
  .nav-item:hover { background: rgba(var(--c-accent-rgb), 0.08); color: var(--c-text); }
  .nav-item.active {
    background: rgba(var(--c-accent-rgb), 0.12);
    color: var(--c-accent);
    border-left-color: var(--c-accent);
    font-weight: 500;
  }

  #settings-content { flex: 1; overflow-y: auto; padding: 20px 24px; }

  .panel { display: none; }
  .panel.active { display: block; }

  .panel-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--c-text-hi);
    margin-bottom: 20px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .panel-title::before {
    content: "";
    display: inline-block;
    width: 4px;
    height: 16px;
    background: var(--c-accent);
    border-radius: 2px;
  }

  .param-group { margin-bottom: 24px; }

  .param-group-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--c-accent);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 12px;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--c-border);
  }

  .param-row { display: flex; align-items: center; margin-bottom: 10px; gap: 12px; }

  .param-label { width: 160px; font-size: 13px; color: var(--c-text-2); flex-shrink: 0; }
  .param-label .param-key { font-size: 11px; color: var(--c-text-4); margin-top: 2px; }

  .param-input { flex: 1; max-width: 300px; }
  .param-input input, .param-input select {
    width: 100%;
    padding: 7px 12px;
    background: var(--c-panel-2);
    border: 1px solid var(--c-border);
    border-radius: 6px;
    color: var(--c-text);
    font-size: 13px;
    outline: none;
    transition: border-color 0.2s;
  }
  .param-input input:focus, .param-input select:focus { border-color: var(--c-accent); }
  .param-input select { cursor: pointer; }
  .param-input select option { background: var(--c-panel-2); color: var(--c-text); }
  .param-input .checkbox-wrap { display: flex; align-items: center; gap: 8px; }
  .param-input input[type="checkbox"] { width: 16px; height: 16px; cursor: pointer; accent-color: var(--c-accent); }

  .param-desc { font-size: 11px; color: var(--c-text-4); margin-top: 2px; }

  /* ===== 主题卡片 ===== */
  .theme-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 12px; max-width: 660px; }
  .theme-card {
    border: 2px solid var(--c-border);
    border-radius: 10px;
    padding: 10px;
    cursor: pointer;
    background: var(--c-panel);
    transition: border-color 0.2s, transform 0.1s;
  }
  .theme-card:hover { border-color: var(--c-accent); transform: translateY(-2px); }
  .theme-card.active { border-color: var(--c-accent); box-shadow: 0 0 0 1px var(--c-accent); }
  .theme-card .theme-preview { display: flex; height: 46px; border-radius: 6px; overflow: hidden; border: 1px solid var(--c-border-soft); }
  .theme-card .theme-preview span { flex: 1; }
  .theme-card .theme-name { font-size: 13px; color: var(--c-text); margin-top: 8px; display: flex; align-items: center; gap: 6px; }
  .theme-card .theme-check { color: var(--c-accent); font-weight: 700; visibility: hidden; }
  .theme-card.active .theme-check { visibility: visible; }

  .btn-reset {
    background: var(--c-overlay);
    color: var(--c-text);
    border: none;
    padding: 10px 28px;
    border-radius: 8px;
    font-size: 14px;
    cursor: pointer;
    transition: background 0.2s;
    margin-top: 16px;
    margin-left: 12px;
  }
  .btn-reset:hover { background: var(--c-overlay-strong); }

  .version-table { width: 100%; max-width: 500px; }
  .version-table tr { border-bottom: 1px solid var(--c-border-soft); }
  .version-table td { padding: 12px 0; font-size: 14px; }
  .version-table td:first-child { color: var(--c-text-2); width: 140px; }
  .version-table td:last-child { color: var(--c-text); font-weight: 500; }

  /* 多机互联：节点清单表 */
  .mn-table { width: 100%; border-collapse: collapse; }
  .mn-table th { text-align: left; font-size: 12px; color: var(--c-text-3); font-weight: 500; padding: 8px 6px; border-bottom: 1px solid var(--c-border); white-space: nowrap; }
  .mn-table td { padding: 6px; border-bottom: 1px solid var(--c-border-soft); vertical-align: middle; }
  .mn-table input.mn-cell {
    padding: 0 8px;
    height: 28px;
    line-height: 1.15;
    background: var(--c-panel-2);
    border: 1px solid var(--c-border);
    border-radius: 5px;
    color: var(--c-text);
    font-size: 12px;
    outline: none;
    width: 100%;
    box-sizing: border-box;
  }
  .mn-table input.mn-cell:focus { border-color: var(--c-accent); }
  .mn-table .mn-self-chk { width: 16px; height: 16px; cursor: pointer; accent-color: var(--c-accent); }

  /* 镜像管理表：自适应宽度，不限制最大宽度 */
  .version-table.engine-table { max-width: none; table-layout: auto; }
  .version-table.engine-table th { padding: 6px 10px; font-size: 12px; color: var(--c-text-3); font-weight: 500; }
  .version-table.engine-table td { padding: 8px 10px; }
  .version-table.engine-table td:first-child,
  .version-table.engine-table td:last-child { width: auto; color: inherit; font-weight: normal; }
  .version-table.engine-table td:first-child { font-family: 'SFMono-Regular',Consolas,monospace; font-size: 12px; white-space: nowrap; }

  .about-content { max-width: 500px; }
  .about-content h3 { font-size: 20px; color: var(--c-text-hi); margin-bottom: 8px; }
  .about-content .about-subtitle { font-size: 13px; color: var(--c-accent); margin-bottom: 20px; }
  .about-content p { font-size: 14px; color: var(--c-text-2); line-height: 1.8; margin-bottom: 12px; }
  .about-content a { color: var(--c-accent); text-decoration: none; }
  .about-content a:hover { text-decoration: underline; }

  .save-toast {
    position: fixed;
    top: 20px;
    right: 20px;
    background: #2e7d32;
    color: #fff;
    padding: 12px 20px;
    border-radius: 8px;
    font-size: 13px;
    z-index: 1000;
    animation: slideIn 0.3s ease;
  }

  .error-toast {
    position: fixed;
    top: 20px;
    right: 20px;
    background: #c62828;
    color: #fff;
    padding: 12px 20px;
    border-radius: 8px;
    font-size: 13px;
    z-index: 1000;
    animation: slideIn 0.3s ease;
  }

  @keyframes slideIn {
    from { transform: translateX(100%); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }

  .confirm-overlay {
    display: none;
    position: fixed;
    top: 0; left: 0; right: 0; bottom: 0;
    background: rgba(0, 0, 0, 0.6);
    z-index: 9999;
    justify-content: center;
    align-items: center;
  }
  .confirm-overlay.show { display: flex; }
  .confirm-dialog {
    background: var(--c-panel);
    border: 1px solid var(--c-border);
    border-radius: 12px;
    padding: 28px 32px;
    max-width: 400px;
    width: 90%;
    box-shadow: 0 8px 32px rgba(0,0,0,0.5);
    text-align: center;
  }
  .confirm-dialog .confirm-title { font-size: 18px; font-weight: 600; color: var(--c-text-hi); margin-bottom: 12px; }
  .confirm-dialog .confirm-message { font-size: 14px; color: var(--c-text-2); line-height: 1.6; margin-bottom: 24px; white-space: pre-line; }
  .confirm-dialog .confirm-buttons { display: flex; gap: 12px; justify-content: center; }
  .confirm-dialog .btn-confirm { background: #d32f2f; color: #fff; border: none; padding: 8px 28px; border-radius: 8px; font-size: 14px; cursor: pointer; transition: background 0.2s; }
  .confirm-dialog .btn-confirm:hover { background: #b71c1c; }
  .confirm-dialog .btn-cancel { background: var(--c-overlay); color: var(--c-text); border: none; padding: 8px 28px; border-radius: 8px; font-size: 14px; cursor: pointer; transition: background 0.2s; }
  .confirm-dialog .btn-cancel:hover { background: var(--c-overlay-strong); }
</style>
<div id="settings-app">
  <div id="settings-header">
    <button class="back-btn" id="back-btn">&#8592; ${_t("返回")}</button>
    <span class="title">${_t("设置")}</span>
  </div>
  <div id="settings-layout">
    <nav id="settings-nav">
      <div class="nav-item active" data-panel="launch-params" id="nav-launch-params">${_t("模型启动参数")}</div>
      <div class="nav-item" data-panel="docker-mirror" id="nav-docker-mirror">${_t("Docker 镜像配置")}</div>
      <div class="nav-item" data-panel="proxy" id="nav-proxy">${_t("代理")}</div>
      <div class="nav-item" data-panel="multinode" id="nav-multinode">${_t("DGX直连配置")}</div>
      <div class="nav-item" data-panel="appearance" id="nav-appearance">${_t("外观主题")}</div>
      <div class="nav-item" data-panel="logs" id="nav-logs">${_t("运行日志")}</div>
      <div class="nav-item" data-panel="version" id="nav-version">${_t("系统版本号")}</div>
      <div class="nav-item" data-panel="about" id="nav-about">${_t("关于")}</div>
    </nav>
    <div id="settings-content">
      <div id="panel-launch-params" class="panel active">
        <div class="panel-title">${_t("模型启动参数")}（${_t("推理引擎")}）</div>

        <div class="param-group">
          <div class="param-group-title">${_t("基础参数")}</div>
          <div class="param-row">
            <div class="param-label">${_t("最大上下文长度")}<div class="param-key">--max-model-len</div></div>
            <div class="param-input"><input type="number" id="ctx_size" value="256000" min="0"><div class="param-desc">${_t("模型最大上下文长度，默认 256000")}</div></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("共享内存")}<div class="param-key">--shm-size</div></div>
            <div class="param-input"><input type="text" id="vllm_shm" placeholder="64g" style="max-width:200px;"><div class="param-desc">${_t("DGX Spark 建议 64g，其他机型 32g")}</div></div>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("服务参数")}</div>
          <div class="param-row">
            <div class="param-label">${_t("监听端口")}<div class="param-key">--port</div></div>
            <div class="param-input"><input type="number" id="port" value="8000" min="1" max="65535"><div class="param-desc">${_t("Docker 端口映射（容器内始终监听 0.0.0.0）")}</div></div>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("vLLM 基础参数")}</div>
          <div class="param-row">
            <div class="param-label">${_t("加载格式")}<div class="param-key">--load-format</div></div>
            <div class="param-input">
              <select id="sg_load_format">
                <option value="">${_t("不指定（vLLM 默认）")}</option>
                <option value="safetensors">safetensors</option>
                <option value="instanttensor">instanttensor</option>
                <option value="auto">auto</option>
              </select>
            </div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("Block Size")}<div class="param-key">--block-size</div></div>
            <div class="param-input"><input type="number" id="sg_block_size" value="256" min="0"></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("Tokenizer 模式")}<div class="param-key">--tokenizer-mode</div></div>
            <div class="param-input"><input type="text" id="sg_tokenizer_mode" placeholder="deepseek_v4"></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("工具解析器")}<div class="param-key">--tool-call-parser</div></div>
            <div class="param-input"><input type="text" id="sg_tool_parser" placeholder="deepseek_v4"></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("推理解析器")}<div class="param-key">--reasoning-parser</div></div>
            <div class="param-input"><input type="text" id="sg_reasoning_parser" placeholder="deepseek_v4"></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("自动工具选择")}<div class="param-key">--enable-auto-tool-choice</div></div>
            <div class="param-input"><div class="checkbox-wrap"><input type="checkbox" id="sg_auto_tool_choice" checked></div></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("信任远程代码")}<div class="param-key">--trust-remote-code</div></div>
            <div class="param-input"><div class="checkbox-wrap"><input type="checkbox" id="sg_trust_remote" checked></div></div>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("推理参数")}</div>
          <div class="param-row">
            <div class="param-label">${_t("张量并行")}<div class="param-key">--tensor-parallel-size</div></div>
            <div class="param-input"><input type="number" id="sg_tp" value="1" min="1"><div class="param-desc">${_t("单机多卡时拆分模型权重到多张 GPU（如 2 张卡填 2）；单卡填 1。2 台 Spark 跨机部署需要多节点模式，当前版本暂不支持")}</div></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("GPU 显存利用率")}<div class="param-key">--gpu-memory-utilization</div></div>
            <div class="param-input"><input type="number" id="sg_gpu_mem_util" value="0" min="0" max="1" step="0.01"><div class="param-desc">${_t("0 = 自动；KV 缓存池/权重内存占比，OOM 时调小")}</div></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("量化方法")}<div class="param-key">--quantization</div></div>
            <div class="param-input">
              <select id="sg_quant">
                <option value="">${_t("不指定（自动从 config 解析）")}</option>
                <option value="deepseek_v4_fp8">deepseek_v4_fp8</option>
                <option value="fp8">fp8</option>
                <option value="mxfp8">mxfp8</option>
                <option value="nvfp4">nvfp4</option>
                <option value="awq">awq</option>
                <option value="gptq">gptq</option>
                <option value="compressed-tensors">compressed-tensors</option>
              </select>
              <div class="param-desc">${_t("预量化模型（如 unsloth NVFP4）无需指定，加载时自动识别")}</div>
            </div>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("KV Cache 与调度")}</div>
          <div class="param-row">
            <div class="param-label">${_t("KV Cache 类型")}<div class="param-key">--kv-cache-dtype</div></div>
            <div class="param-input">
              <select id="sg_kv_dtype">
                <option value="">${_t("auto（默认）")}</option>
                <option value="fp8">fp8</option>
                <option value="fp8_e4m3">fp8_e4m3</option>
                <option value="bf16">bf16</option>
              </select>
            </div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("最大并发序列数")}<div class="param-key">--max-num-seqs</div></div>
            <div class="param-input"><input type="number" id="sg_max_seqs" value="0" min="0"><div class="param-desc">${_t("0 = 自动")}</div></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("单批最大 Token 数")}<div class="param-key">--max-num-batched-tokens</div></div>
            <div class="param-input"><input type="number" id="sg_max_batched_tokens" value="0" min="-1"><div class="param-desc">${_t("0 = 自动，-1 = 禁用；长提示词 OOM 时调小（如 4096）")}</div></div>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("更多启动参数")}</div>
          <div class="param-row" style="align-items:flex-start;">
            <div class="param-label">${_t("额外参数")}<div class="param-key">extra_args</div></div>
            <div class="param-input" style="max-width:480px;">
              <textarea id="sg_extra_args" rows="4" style="width:100%;resize:vertical;background:var(--c-panel-2);border:1px solid var(--c-border);border-radius:6px;color:var(--c-text);font-size:13px;padding:8px 12px;font-family:monospace;outline:none;" placeholder="disable-cuda-graph&#10;reasoning-parser=deepseek&#10;# 注释行以 # 开头会被忽略"></textarea>
              <div class="param-desc">${_t("每行一个参数：key=value 拼成 --key value；无等号的整行作为纯开关参数（如 disable-cuda-graph）；# 开头为注释。多机 NCCL 卡死排查可加 disable-cuda-graph")}</div>
            </div>
          </div>
          <div class="param-row" style="align-items:flex-start;">
            <div class="param-label">${_t("额外环境变量")}<div class="param-key">extra_env</div></div>
            <div class="param-input" style="max-width:480px;">
              <textarea id="sg_extra_env" rows="3" style="width:100%;resize:vertical;background:var(--c-panel-2);border:1px solid var(--c-border);border-radius:6px;color:var(--c-text);font-size:13px;padding:8px 12px;font-family:monospace;outline:none;" placeholder="NCCL_DEBUG=TRACE&#10;NCCL_CUMEM_ENABLE=1&#10;# 每行 KEY=VALUE 注入容器环境变量"></textarea>
              <div class="param-desc">${_t("每行 KEY=VALUE 注入容器环境变量（docker run -e）")}</div>
            </div>
          </div>
        </div>

        <button class="btn-reset" id="reset-btn">${_t("恢复默认")}</button>
      </div>

      <div id="panel-docker-mirror" class="panel">
        <div class="panel-title">${_t("Docker 镜像配置")}</div>

        <div class="param-group">
          <div class="param-group-title">${_t("镜像加速（registry-mirrors）")}</div>
          <div class="param-desc" style="margin-bottom:10px;">${_t("写回 Docker daemon.json 的 registry-mirrors 实现国内镜像加速（仅影响后续镜像拉取），保存后自动重启 Docker 服务使配置生效")}</div>
          <div class="param-row">
            <div class="param-label">${_t("配置文件")}<div class="param-key">daemon.json</div></div>
            <div class="param-input" style="max-width:none;"><span id="mirror-daemon-path" style="font-family:monospace;font-size:12px;color:var(--c-text-2);"></span></div>
          </div>
          <div class="param-row" style="align-items:flex-start;">
            <div class="param-label">${_t("加速器地址")}<div class="param-key">registry-mirrors</div></div>
            <div class="param-input" style="max-width:480px;">
              <textarea id="mirror-list" rows="4" style="width:100%;resize:vertical;background:var(--c-panel-2);border:1px solid var(--c-border);border-radius:6px;color:var(--c-text);font-size:13px;padding:8px 12px;font-family:monospace;outline:none;" placeholder="${DEFAULT_DOCKER_MIRRORS.join("&#10;")}"></textarea>
              <div class="param-desc">${_t("每行一个加速器地址，留空表示直连 Docker Hub")}</div>
            </div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("操作")}</div>
            <div class="param-input" style="max-width:none;display:flex;align-items:center;gap:10px;">
              <button class="btn-save" id="mirror-save-btn" style="margin-top:0;font-size:13px;padding:8px 20px;">${_t("保存并重启 Docker")}</button>
              <span id="mirror-status" style="font-size:12px;color:var(--c-text-3);"></span>
            </div>
          </div>
        </div>
      </div>

      <div id="panel-proxy" class="panel">
        <div class="panel-title">${_t("代理")}</div>

        <div class="param-group">
          <div class="param-group-title">${_t("本地代理")}</div>
          <div class="param-row" style="align-items:flex-start;">
            <div class="param-label">${_t("代理地址")}<div class="param-key">proxy_url</div></div>
            <div class="param-input" style="max-width:480px;">
              <input type="text" id="proxy_url" placeholder="http://127.0.0.1:1080" style="max-width:320px;">
              <div class="param-desc">${_t("模型文件下载（HF 模型）走该代理，填写后保存即生效；留空 = 直连。仅支持 HTTP(S) 代理地址")}</div>
            </div>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("Docker 镜像拉取")}</div>
          <div class="param-desc" style="margin-bottom:10px;">${_t("Docker 镜像由 Docker 守护进程下载，需把代理写入 daemon.json 的 proxies 配置并重启 Docker 服务后生效（镜像拉取期间不要关闭代理软件）")}</div>
          <div class="param-row">
            <div class="param-label">${_t("配置文件")}<div class="param-key">daemon.json</div></div>
            <div class="param-input" style="max-width:none;"><span id="proxy-daemon-path" style="font-family:monospace;font-size:12px;color:var(--c-text-2);"></span></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("操作")}</div>
            <div class="param-input" style="max-width:none;display:flex;align-items:center;gap:10px;">
              <button class="btn-save" id="proxy-save-btn" style="margin-top:0;font-size:13px;padding:8px 20px;">${_t("保存并重启 Docker")}</button>
              <span id="proxy-status" style="font-size:12px;color:var(--c-text-3);"></span>
            </div>
          </div>
        </div>
      </div>

      <div id="panel-multinode" class="panel">
        <div class="panel-title">${_t("DGX直连配置")}</div>

        <div class="param-group" style="border:1px solid rgba(var(--c-accent-rgb),0.45);border-radius:10px;padding:16px;background:var(--c-panel-2);">
          <div class="param-group-title" style="font-size:14px;">${_t("DGX-Spark 双机直连 · 一键从零部署")} <span style="font-weight:400;text-transform:none;letter-spacing:0;color:var(--c-text-3);">(${_t("A 为控制机，B 为对等节点")})</span></div>
          <div class="param-desc" style="line-height:1.9;margin-bottom:14px;font-size:12px;color:var(--c-text-3);">
            <span style="color:#f44336;font-weight:600;">${_t("⚠ 准备提示")}</span>：${_t("直连线型号：200G QSFP56 DAC 直连铜缆（DGX-Spark 网口是 QSFP56（200G），不是 QSFP112（400G），不要买错，买 1 条就够——插 2 条受内存带宽限制提升不大，反而增加配置复杂度）。插好直连线后 A、B 必须插同一方向的口（如都插左边），插口方向不一致会自动检测到并报错。")}
          </div>
          <div class="param-row">
            <div class="param-label">${_t("A 控制机（本机）")}<div class="param-key">sudo 密码</div></div>
            <div class="param-input" style="max-width:460px;display:flex;align-items:center;gap:8px;">
              <input id="zd-a-user" placeholder="${_t("用户名")}" style="flex:1;width:auto;min-width:0;">
              <input id="zd-a-pass" type="text" placeholder="${_t("输入密码")}" autocomplete="off" style="flex:1;width:auto;min-width:0;">
            </div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("B 对等节点")}<div class="param-key">${_t("当前可达地址（管理网 IP）")}</div></div>
            <div class="param-input" style="max-width:460px;display:flex;align-items:center;gap:8px;">
              <input id="zd-b-addr" placeholder="${_t("IP 地址，例如 192.168.1.X")}" style="flex:1.6;width:auto;min-width:0;">
              <input id="zd-b-user" placeholder="${_t("用户名")}" style="flex:1;width:auto;min-width:0;">
              <input id="zd-b-pass" type="text" placeholder="${_t("输入密码")}" autocomplete="off" style="flex:1;width:auto;min-width:0;">
            </div>
          </div>
          <div class="param-row" style="margin-top:14px;">
            <button class="btn-save" id="zd-start-btn" style="margin-top:0;font-size:13px;padding:8px 22px;">${_t("开始一键部署")}</button>
            <span id="zd-status" style="font-size:12px;color:var(--c-text-3);"></span>
          </div>
          <div id="zd-steps" style="display:flex;flex-wrap:wrap;gap:6px;margin:8px 0 4px;"></div>
          <pre id="zd-log" style="margin:8px 0 0;padding:10px 12px;background:var(--c-panel);border:1px solid var(--c-border);border-radius:6px;max-height:220px;overflow-y:auto;font-size:12px;font-family:monospace;color:var(--c-text-2);line-height:1.7;white-space:pre-wrap;word-break:break-all;">${_t("点击「开始一键部署」后在此显示运行日志")}</pre>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("总开关")}</div>
          <div class="param-row">
            <div class="param-label">${_t("启用多机模式")}<div class="param-key">multiNodeArgs.enabled</div></div>
            <div class="param-input">
              <div class="checkbox-wrap"><input type="checkbox" id="multi_enabled" style="width:16px;height:16px;cursor:pointer;accent-color:var(--c-accent);"></div>
              <div class="param-desc">${_t("开启后启动模型走多节点集群（2+ 台 DGX Spark 组成 TP=N 推理集群）；关闭 = 单机模式不变")}</div>
            </div>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("直连节点")}</div>
          <div class="param-desc" style="margin-bottom:10px;">
            ${_t("按下标即 rank；第 1 行自动为主节点（本机），第 2 行为直连节点。")}
            <span style="color:#f44336;font-weight:600;">${_t("重点：IP 请填写已配置好的光口（ConnectX-7 QSFP）互连 IP，不是 RJ45 网卡的局域网 IP！")}</span>
          </div>
          <table class="mn-table">
            <thead><tr><th>rank</th><th>${_t("光口 IP")}</th><th>SSH 用户</th><th>SSH 端口</th><th>${_t("模型目录")}</th><th>${_t("状态")}</th><th>${_t("操作")}</th></tr></thead>
            <tbody id="mn-tbody"></tbody>
          </table>
          <div style="margin-top:8px;display:flex;gap:8px;align-items:center;flex-wrap:wrap;">
            <button class="btn-reset" id="mn-push-img-btn" style="font-size:13px;padding:6px 16px;">${_t("同步镜像到直连节点")}</button>
            <span id="mn-push-img-status" style="font-size:12px;color:var(--c-text-3);"></span>
          </div>
          <div style="margin-top:8px;display:flex;gap:8px;align-items:center;flex-wrap:wrap;">
            <button class="btn-reset" id="mn-sync-model-btn" style="font-size:13px;padding:6px 16px;">${_t("同步模型到直连节点")}</button>
            <span id="mn-sync-model-status" style="font-size:12px;color:var(--c-text-3);"></span>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("互联参数")}</div>
          <div class="param-row">
            <div class="param-label">${_t("master 端口")}<div class="param-key">--master-port</div></div>
            <div class="param-input"><input type="number" id="multi_dist_port" value="6379" min="1" max="65535" style="max-width:160px;"><div class="param-desc">${_t("分布式 master 端口（所有节点通过节点 0 的该端口握手，fork `launch-cluster.sh` 的 MASTER_PORT），不得与模型服务端口冲突")}</div></div>
          </div>
          <div class="param-row" style="align-items:flex-start;">
            <div class="param-label">${_t("额外环境变量")}<div class="param-key">extra_env</div></div>
            <div class="param-input" style="max-width:480px;">
              <textarea id="multi_extra_env" rows="3" style="width:100%;resize:vertical;background:var(--c-panel-2);border:1px solid var(--c-border);border-radius:6px;color:var(--c-text);font-size:13px;padding:8px 12px;font-family:monospace;outline:none;" placeholder="NCCL_DEBUG=TRACE&#10;NCCL_CUMEM_ENABLE=1&#10;NCCL_NVLS_ENABLE=1&#10;# 官方 inkling recipe 要求预置 CUMEM/NVLS=1；排查卡死时同步开 TRACE"></textarea>
              <div class="param-desc">${_t("每行 KEY=VALUE 注入容器环境变量（docker run -e），NCCL 排查用：NCCL_DEBUG=TRACE / NCCL_SOCKET_NTHREADS=1 / NCCL_IB_GID_INDEX=3 等。多机 NCCL 卡死排查时试预置 NCCL_CUMEM_ENABLE=1、NCCL_NVLS_ENABLE=1（官方 inkling recipe 要求）")}</div>
            </div>
          </div>
        </div>
      </div>

      <div id="panel-appearance" class="panel">
        <div class="panel-title">${_t("外观主题")}</div>
        <div class="param-group">
          <div class="param-group-title">${_t("语言 / Language")}</div>
          <div class="param-row">
            <div class="param-label">${_t("界面语言")}<div class="param-key">language</div></div>
            <div class="param-input">
              <select id="ui-lang-select">
                <option value="zh">中文</option>
                <option value="en">English</option>
              </select>
              <div class="param-desc">${_t("切换后立即生效")}</div>
            </div>
          </div>
          <div class="param-group-title">${_t("主题配色")}</div>
          <div class="param-desc" style="margin-bottom:14px;">${_t("选择界面配色，风格参考主流 VS Code 主题，点击即可一键切换（立即生效）")}</div>
          <div id="theme-grid" class="theme-grid"></div>
        </div>
      </div>

      <div id="panel-version" class="panel">
        <div class="panel-title">${_t("系统版本号")}</div>
        <table class="version-table">
          <tr><td>${_t("ADM-BE 版本")}</td><td id="v-adm">${_t("检测中...")} <span id="update-badge" style="display:none;color:#4caf50;font-size:12px;margin-left:6px;">${_t("✓ 最新")}</span></td></tr>
          <tr>
            <td style="padding-top:20px;" colspan="2">
              <button class="btn-save" id="check-update-btn" style="margin-top:0;font-size:13px;padding:8px 20px;">${_t("检查新版本")}</button>
              <span id="update-status" style="font-size:12px;color:var(--c-text-3);margin-left:12px;"></span>
            </td>
          </tr>
          <tr><td>${_t("Tauri 版本")}</td><td>2.11.2</td></tr>
          <tr><td>${_t("操作系统")}</td><td id="v-os">${_t("检测中...")}</td></tr>
        </table>
      </div>

      <div id="panel-about" class="panel">
        <div class="panel-title">${_t("关于")}</div>
        <div class="about-content">
          <h3>ADM-BE</h3>
          <div class="about-subtitle">Automatic Deployment Model</div>
          <p>${_t("ADM-BE 是一个大模型部署图形化管理工具，让用户能够便捷地在本地部署和运行大语言模型。")}</p>
          <p>${_t("如需定制服务 联系方式：微信: litai686")}</p>
          <p>${_t("项目官网：")}<a href="https://adm.tuduoduo.top/" target="_blank">https://adm.tuduoduo.top/</a></p>
        </div>
      </div>

      <div id="panel-logs" class="panel">
        <div class="panel-title">${_t("运行日志")}</div>
        <div class="param-group">
          <div class="param-row" style="margin-bottom:16px;align-items:center;">
            <div class="param-label">${_t("选择日期")}</div>
            <div class="param-input" style="display:flex;gap:8px;align-items:center;flex-wrap:wrap;">
              <select id="log-date-select" style="min-width:160px;width:auto;"></select>
              <button class="btn-reset" id="log-refresh-btn" style="margin:0;padding:6px 16px;font-size:13px;">${_t("刷新")}</button>
              <button class="btn-reset" id="log-open-dir-btn" style="margin:0;padding:6px 16px;font-size:13px;">${_t("打开日志目录")}</button>
              <button class="btn-reset" id="log-clear-btn" style="margin:0;padding:6px 16px;font-size:13px;">${_t("清空日志")}</button>
            </div>
          </div>
          <pre id="log-content" style="background:var(--c-bg-deep);color:var(--c-text-2);font-family:'SFMono-Regular',Consolas,'Liberation Mono',Menlo,monospace;font-size:12px;line-height:1.6;padding:12px 16px;border-radius:8px;border:1px solid var(--c-border);overflow-y:auto;max-height:60vh;white-space:pre-wrap;word-break:break-all;margin:0;">${_t("暂无日志")}</pre>
        </div>
      </div>
    </div>
  </div>
</div>

<div class="confirm-overlay" id="confirm-overlay">
  <div class="confirm-dialog">
    <div class="confirm-title" id="confirm-title">${_t("删除提示")}</div>
    <div class="confirm-message" id="confirm-message"></div>
    <div class="confirm-buttons">
      <button class="btn-cancel" id="confirm-cancel-btn">${_t("取消")}</button>
      <button class="btn-confirm" id="confirm-ok-btn">${_t("确定")}</button>
    </div>
  </div>
</div>
`;

const DEFAULT_CTX_SIZE = 256000;

const invoke = () => window.__adm_invoke;

// HTML 转义：远端 update.json 下发的版本号 / 地址会拼进弹窗 innerHTML，需真实转义防注入
function escHtml(s) {
  if (s == null) return "";
  return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

function switchPanel(panelId) {
  console.log("[settings] 切换面板:", panelId);
  document.querySelectorAll(".nav-item").forEach((el) => el.classList.remove("active"));
  document.querySelectorAll(".panel").forEach((el) => el.classList.remove("active"));
  document.querySelector('[data-panel="' + panelId + '"]').classList.add("active");
  document.getElementById("panel-" + panelId).classList.add("active");
}

function showToast(message, isError) {
  const existing = document.querySelector(".save-toast, .error-toast");
  if (existing) existing.remove();
  const toast = document.createElement("div");
  toast.className = isError ? "error-toast" : "save-toast";
  toast.textContent = message;
  document.body.appendChild(toast);
  setTimeout(() => toast.remove(), 3000);
}

// ===== DGX-Spark 双机直连 · 一键从零部署 =====
// 状态不落盘（密码仅内存）：切页回来时恢复步骤状态与日志展示
const ZD_STEPS = ["免密 SSH 配置", "光口探测", "固定 IP 配置", "连通性检测", "Docker 检测安装", "Docker 组权限", "写回多机互联节点表"];
let zdRunning = false;
let zdStepStates = [];
let zdLogText = "";

function zdStepStatusHtml(i) {
  const st = (zdStepStates[i] || {}).status || "idle";
  const conf = {
    idle: ["○", "var(--c-text-4)", ""],
    active: ["●", "var(--c-accent)", "border-color:var(--c-accent);font-weight:600;"],
    done: ["✓", "#4caf50", ""],
    error: ["✗", "#f44336", "border-color:#f44336;color:#f44336;"]
  }[st];
  return '<span style="border:1px solid var(--c-border);border-radius:4px;padding:2px 8px;font-size:12px;color:' + conf[1] + ';' + conf[2] + '">' + conf[0] + " " + _t(ZD_STEPS[i]) + "</span>";
}

function renderZdSteps() {
  const box = document.getElementById("zd-steps");
  if (!box) return;
  box.innerHTML = ZD_STEPS.map(function (_, i) { return zdStepStatusHtml(i); }).join("");
}

function zdLog(line) {
  const el = document.getElementById("zd-log");
  if (!el) return;
  if (el.textContent === _t("点击「开始一键部署」后在此显示运行日志")) el.textContent = "";
  zdLogText += line + "\n";
  el.textContent = zdLogText;
  el.scrollTop = el.scrollHeight;
}

function setZdRunning(run) {
  zdRunning = run;
  const btn = document.getElementById("zd-start-btn");
  if (btn) {
    btn.disabled = run;
    btn.textContent = run ? _t("部署中...") : _t("开始一键部署");
  }
}

async function startZdDeploy() {
  if (zdRunning) return;
  const val = function (id) {
    const el = document.getElementById(id);
    return el ? el.value.trim() : "";
  };
  const aUser = val("zd-a-user");
  const aPass = document.getElementById("zd-a-pass") ? document.getElementById("zd-a-pass").value : "";
  const bAddr = val("zd-b-addr");
  const bUser = val("zd-b-user");
  const bPass = document.getElementById("zd-b-pass") ? document.getElementById("zd-b-pass").value : "";
  // B 对等节点固定 SSH 端口 22（默认，无需填写）
  const bPort = 22;
  if (!aUser || !aPass) { showToast(_t("请填写 A 控制机用户名和密码"), true); return; }
  if (!bAddr || !bUser || !bPass) { showToast(_t("请填写 B 对等节点地址、用户名和密码"), true); return; }
  setZdRunning(true);
  const st = document.getElementById("zd-status");
  if (st) st.textContent = "";
  try {
    await invoke()("dgx_deploy_run", {
      bAddr: bAddr, bPort: bPort, bUser: bUser, bPass: bPass, aUser: aUser, aPass: aPass
    });
  } catch (e) {
    // 失败细节已由 dgx-zero-deploy error 事件进入日志/步骤条；此处兜底弹提示
    setZdRunning(false);
    const msg = String((e && e.message) || e || "");
    if (msg && msg.indexOf("DEPLOY_DONE") < 0) showToast(_t("部署失败：") + msg, true);
  }
}

function zdInitForm() {
  const aUserEl = document.getElementById("zd-a-user");
  if (aUserEl && !aUserEl.value.trim()) {
    invoke()("get_local_username").then(function (u) {
      if (aUserEl && u && !aUserEl.value) aUserEl.value = String(u);
    }).catch(function () {});
  }
  renderZdSteps();
  const el = document.getElementById("zd-log");
  if (el && zdLogText) { el.textContent = zdLogText; el.scrollTop = el.scrollHeight; }
  if (zdRunning) setZdRunning(true);
}

function handleZdEvent(p) {
  if (!p || typeof p.step !== "number") return;
  const step = p.step;
  const phase = p.phase;
  const detail = String(p.detail || "");
  if (step === 0) {
    if (phase === "start") {
      zdLogText = "";
      zdStepStates = ZD_STEPS.map(function () { return { status: "idle" }; });
      renderZdSteps();
      const el = document.getElementById("zd-log");
      if (el) el.textContent = "";
      setZdRunning(true);
      const st = document.getElementById("zd-status");
      if (st) st.textContent = _t("部署中，请勿关闭窗口...");
      zdLog(_t("==== 开始一键部署 ===="));
    } else if (phase === "done") {
      setZdRunning(false);
      renderZdSteps();
      const st = document.getElementById("zd-status");
      if (st) st.textContent = _t("✔ 部署完成！多机互联节点表已就绪，可直接使用");
      zdLog(_t("==== 全部完成 ===="));
      refreshMnNodesFromSettings();
    }
    return;
  }
  if (step < 1 || step > 7) return;
  const name = _t(ZD_STEPS[step - 1]);
  if (phase === "start") {
    zdStepStates[step - 1] = { status: "active" };
    renderZdSteps();
    zdLog("▶ " + name);
  } else if (phase === "run") {
    zdLog("  " + detail);
  } else if (phase === "done") {
    zdStepStates[step - 1] = { status: "done" };
    renderZdSteps();
    zdLog("✔ " + name + " 完成");
  } else if (phase === "error") {
    zdStepStates[step - 1] = { status: "error" };
    renderZdSteps();
    zdLog("✗ " + name + " 失败：" + detail);
    zdLog("⤿ " + _t("已自动回滚，可修复问题后重新执行"));
    setZdRunning(false);
    const st = document.getElementById("zd-status");
    if (st) st.textContent = _t("部署失败，已自动回滚（详见日志）");
  } else if (phase === "rollback") {
    zdLog("⤿ " + _t("回滚：") + detail);
  } else if (phase === "rollback_done") {
    zdLog("⤿ " + _t("回滚完成"));
  }
}

async function refreshMnNodesFromSettings() {
  // 部署成功后后端已写入 config.json（多机节点表），刷新上方节点表展示
  try {
    const settings = await invoke()("load_settings");
    if (settings && settings.multi_node_args) await fillMultiNodeArgsForm(settings.multi_node_args);
  } catch (_) {}
}

// ===== 多机互联（DGX Spark 集群）=====
// 节点清单内存态：mnNodes（每项 { ip, sshUser, sshPort, isSelf, modelDir }）；
// 输入 change 时写回并触发 autoSave，探活结果存 mnProbeState（rank -> { probing | ok, detail }）。
let mnNodes = [];
let mnProbeState = {};

function defaultMnNode(isSelf) {
  return { ip: "", sshUser: isSelf ? "" : "user", sshPort: 22, isSelf: !!isSelf, modelDir: "" };
}

function renderMultiNodeTable() {
  const tbody = document.getElementById("mn-tbody");
  if (!tbody) return;
  tbody.innerHTML = mnNodes.map(function (n, i) {
    const p = mnProbeState[i];
    let statusHtml;
    if (!p) {
      statusHtml = '<span style="color:var(--c-text-4);">' + _t("未测试") + '</span>';
    } else if (p.probing) {
      statusHtml = '<span style="color:var(--c-text-3);">' + _t("测试中...") + '</span>';
    } else if (p.ok) {
      statusHtml = '<span style="color:#4caf50;">' + _t("正常") + '</span><div style="font-size:11px;color:var(--c-text-3);max-width:200px;">' + escHtml(p.detail) + '</div>';
    } else {
      statusHtml = '<span style="color:#f44336;">' + _t("失败") + '</span><div style="font-size:11px;color:#f44336;max-width:200px;">' + escHtml(p.detail) + '</div>';
    }
    const portDisabled = i === 0 ? ' disabled' : "";
    return "<tr>" +
      '<td style="text-align:center;font-size:12px;color:var(--c-text-2);">' + i + '</td>' +
      '<td><input class="mn-cell" data-field="ip" data-rank="' + i + '" value="' + escHtml(n.ip) + '" placeholder="192.168.100.1（光口 IP）"></td>' +
      '<td><input class="mn-cell" data-field="sshUser" data-rank="' + i + '" value="' + escHtml(n.sshUser) + '" placeholder="user"></td>' +
      '<td><input class="mn-cell" data-field="sshPort" data-rank="' + i + '" type="number" min="1" max="65535" value="' + (n.sshPort || 22) + '" style="width:70px;"' + portDisabled + '></td>' +
      '<td><input class="mn-cell" data-field="modelDir" data-rank="' + i + '" value="' + escHtml(n.modelDir) + '" placeholder="' + (i === 0 ? "/data/models（自动）" : "/home/user/models/模型ID（留空自动）") + '" style="min-width:170px;"></td>' +
      '<td style="min-width:110px;">' + statusHtml + '</td>' +
      '<td style="white-space:nowrap;vertical-align:middle;">' +
        (i > 0
          ? '<button class="btn-reset mn-probe-btn" data-rank="' + i + '" style="font-size:12px;padding:0 10px;height:24px;line-height:22px;margin-top:2px;box-sizing:border-box;vertical-align:middle;display:inline-block;">' + _t("测试连通") + '</button>'
          : '<span style="color:var(--c-text-4);font-size:12px;line-height:28px;vertical-align:middle;display:inline-block;">' + _t("本机") + '</span>') +
      '</td>' +
    "</tr>";
  }).join("");
}

async function probeNode(rank) {
  const n = mnNodes[rank];
  if (!n || rank === 0) return;
  // 模型目录留空不提前拦截：后端按默认 /home/<SSH用户>/models 根目录探测
  mnProbeState[rank] = { probing: true };
  renderMultiNodeTable();
  // 镜像从当前运行中模型的 remote engine_image / vllm_image 取（未运行或字段缺失时不传，后端跳过镜像检查）
  const image = (() => {
    const state = window.__ADM_STATE || {};
    const runningId = state.runningModelId;
    const model = (state.modelList || []).find(function (m) { return m.model_id === runningId; });
    return (model && (model.engine_image || model.vllm_image)) || null;
  })();
  try {
    const res = await invoke()("multi_node_probe", {
      ip: n.ip,
      user: n.sshUser,
      port: n.sshPort || 22,
      key: null,
      modelDir: n.modelDir,
      image: image,
    });
    if (res.ok) {
      const detail = "GPU:" + (res.gpu || "?") + " Docker:" + (res.docker || "?");
      mnProbeState[rank] = { ok: true, detail: detail };
    } else {
      mnProbeState[rank] = { ok: false, detail: res.error };
    }
  } catch (e) {
    mnProbeState[rank] = { ok: false, detail: String(e) };
  }
  renderMultiNodeTable();
}

// 往「互连网卡」datalist 追加选项（去重）— 已移除，网卡自动从节点 IP 反查

async function probeAllNodes() {
  const ranks = [];
  for (let i = 1; i < mnNodes.length; i++) ranks.push(i);
  if (ranks.length === 0) {
    showToast(_t("请先配置远端节点"), true);
    return;
  }
  await Promise.all(ranks.map(function (r) { return probeNode(r); }));
}

function getMultiNodeArgsFromForm() {
  const b = function (id) { const el = document.getElementById(id); return el ? el.checked : false; };
  const s = function (id) { const el = document.getElementById(id); return el ? el.value.trim() : ""; };
  const n = function (id, def) { const el = document.getElementById(id); const v = parseInt(el ? el.value : ""); return v > 0 ? v : def; };
  return {
    enabled: b("multi_enabled"),
    // 双机直连 v1：只保留本机 + 1 台直连节点
    nodes: mnNodes.slice(0, 2).map(function (x) {
      return { ip: x.ip.trim(), ssh_user: x.sshUser.trim(), ssh_port: x.sshPort || 22, is_self: !!x.isSelf, model_dir: x.modelDir.trim() };
    }),
    dist_init_port: n("multi_dist_port", 6379),
    iface: "",
    ssh_key_path: "",
    extra_env: s("multi_extra_env"),
  };
}

// 同步任务进行中标志（切页回来 mount 时据此恢复按钮禁用态，防止重复触发同步）
let mnImgSyncBusy = false;
let mnModelSyncBusy = false;
// 最近一次同步状态文本（切页回来 mount 时恢复显示，含最终结果）
let mnImgSyncStatus = "";
let mnModelSyncStatus = "";
// 正在同步的镜像名（save/transfer 阶段状态文字显示用）
let mnImgSyncName = "";

// 按钮忙态：禁用 + 文案切换，结束恢复原文案
function setMnBtnBusy(btn, busy, busyText) {
  if (!btn) return;
  if (busy) {
    btn.dataset.idleText = btn.textContent || "";
    btn.disabled = true;
    btn.textContent = busyText;
  } else {
    btn.disabled = false;
    btn.textContent = btn.dataset.idleText || "";
  }
}

// 流式同步本机镜像到直连节点（docker save | gzip | ssh | docker load，不落盘）
async function pushImageToRemote() {
  const btn = document.getElementById("mn-push-img-btn");
  const statusEl = document.getElementById("mn-push-img-status");
  const n = mnNodes[1];
  if (!n) return;
  if (mnImgSyncBusy || (btn && btn.disabled)) return;
  mnImgSyncBusy = true;
  setMnBtnBusy(btn, true, _t("镜像同步中..."));
  mnImgSyncStatus = _t("正在枚举本机镜像...");
  if (statusEl) statusEl.textContent = mnImgSyncStatus;
  try {
    const res = await invoke()("push_image_to_remote", {
      ip: n.ip,
      user: n.sshUser,
      port: n.sshPort || 22,
      key: null,
    });
    if (statusEl) statusEl.textContent = String(res);
    mnImgSyncStatus = String(res);
    showToast(String(res));
    probeNode(1);
  } catch (e) {
    const failMsg = _t("同步失败: ") + e;
    if (statusEl) statusEl.textContent = failMsg;
    mnImgSyncStatus = failMsg;
    showToast(failMsg, true);
  } finally {
    mnImgSyncBusy = false;
    setMnBtnBusy(btn, false);
  }
}

// 同步本机模型到直连节点：全量同步所有已下载模型（远端已同步自动跳过）
async function syncModelToRemote() {
  const btn = document.getElementById("mn-sync-model-btn");
  const statusEl = document.getElementById("mn-sync-model-status");
  const n = mnNodes[1];
  if (!n) return;
  if (mnModelSyncBusy || (btn && btn.disabled)) return;
  mnModelSyncBusy = true;
  setMnBtnBusy(btn, true, _t("模型同步中..."));
  mnModelSyncStatus = _t("检测中...");
  if (statusEl) statusEl.textContent = mnModelSyncStatus;
  try {
    const res = await invoke()("sync_model_to_remote", {
      ip: n.ip,
      user: n.sshUser,
      port: n.sshPort || 22,
      key: null,
      remoteModelDir: n.modelDir.trim(),
    });
    if (statusEl) statusEl.textContent = String(res);
    mnModelSyncStatus = String(res);
    showToast(String(res));
    probeNode(1);
  } catch (e) {
    const failMsg = _t("同步失败: ") + e;
    if (statusEl) statusEl.textContent = failMsg;
    mnModelSyncStatus = failMsg;
    showToast(failMsg, true);
  } finally {
    mnModelSyncBusy = false;
    setMnBtnBusy(btn, false);
  }
}

// 同步进度事件（index.html 全局监听转发）→ 更新对应状态文本
const MN_SYNC_PHASE_TEXT = {
  save: null, // 阶段文案由事件 detail 决定
};
function syncPhaseText(phase) {
  const map = {
    save: _t("导出中..."),
    transfer: _t("传输中..."),
    load: _t("导入中..."),
    done: _t("完成"),
  };
  return map[phase] || _t("同步中...");
}

function handleTauriEvent(type, payload) {
  if (type === "dgx-zero-deploy") { handleZdEvent(payload); return; }
  if ((type !== "image-push-progress" && type !== "model-sync-progress") || !payload || payload.ip === undefined) return;
  const statusEl = type === "image-push-progress"
    ? document.getElementById("mn-push-img-status")
    : document.getElementById("mn-sync-model-status");
  if (!statusEl) return;
  let txt;
  if (type === "image-push-progress") {
    // 镜像同步：save 显示「正在同步 <镜像名>」（多镜像轮换自动更新）；
    // transfer 带「已传输 X MB」时附加；done 显示完成
    if (payload.phase === "done") {
      txt = syncPhaseText("done");
    } else {
      if (payload.phase === "save") {
        const img = String(payload.detail || "")
          .replace(/^同步镜像\s*/, "")
          .replace(/\s*\.\.\.?$/, "")
          .trim();
        if (img) mnImgSyncName = img;
      }
      const d = String(payload.detail || "");
      const extra = d.startsWith("已传输") ? " · " + d : "";
      txt = mnImgSyncName
        ? _t("正在同步") + " " + mnImgSyncName + extra
        : _t("同步中...");
    }
  } else if (payload.phase === "save") {
    // 模型同步：显示正在同步的模型名（detail 形如「同步模型 xxx ...」）
    const name = String(payload.detail || "")
      .replace(/^同步模型\s*/, "")
      .replace(/\s*\.\.\.?$/, "")
      .trim();
    txt = name ? _t("正在同步") + " " + name : syncPhaseText(payload.phase);
  } else {
    txt = syncPhaseText(payload.phase) + (payload.detail ? " · " + payload.detail : "");
  }
  statusEl.textContent = payload.percent != null && payload.percent > 0 && payload.percent < 100
    ? txt + " " + payload.percent + "%"
    : txt;
  // 保存最近状态，切页回来 mount 时恢复显示
  if (type === "image-push-progress") {
    mnImgSyncStatus = statusEl.textContent;
  } else {
    mnModelSyncStatus = statusEl.textContent;
  }
}

async function fillMultiNodeArgsForm(m) {
  const v = m || {};
  // 双机直连 v1：固定 2 个节点（rank0 本机 + rank1 远端），不足自动补默认行
  mnNodes = (v.nodes || []).slice(0, 2).map(function (x) {
    return { ip: x.ip || "", sshUser: x.ssh_user || "", sshPort: x.ssh_port || 22, isSelf: !!x.is_self, modelDir: x.model_dir || "" };
  });
  while (mnNodes.length < 2) {
    mnNodes.push(defaultMnNode(mnNodes.length === 0));
  }
  // 首行必须是本机
  mnNodes[0].isSelf = true;
  mnNodes[1].isSelf = false;
  // 本机模型目录：留空自动填软件数据目录（config.json / models 所在位置）
  if (!mnNodes[0].modelDir.trim()) {
    try {
      const dataDir = await invoke()("get_app_data_dir");
      mnNodes[0].modelDir = String(dataDir) + "/models";
    } catch (_) {}
  }
  // 本机（rank0）SSH 用户名：留空或仍为旧默认值时自动获取当前系统用户名（用户手改的值不会被覆盖）
  if (!mnNodes[0].sshUser.trim() || mnNodes[0].sshUser.trim() === "user") {
    try {
      const localUser = await invoke()("get_local_username");
      if (localUser) mnNodes[0].sshUser = String(localUser);
    } catch (_) {}
  }
  mnProbeState = {};
  const set = function (id, val) { const el = document.getElementById(id); if (el) el.value = val; };
  const setB = function (id, val) { const el = document.getElementById(id); if (el) el.checked = !!val; };
  setB("multi_enabled", v.enabled);
  set("multi_dist_port", v.dist_init_port || 6379);
  set("multi_extra_env", v.extra_env || "");
  renderMultiNodeTable();
}

function getVllmArgsFromForm() {
  const num = function (id) { return parseInt(document.getElementById(id).value) || 0; };
  const str = function (id) { const el = document.getElementById(id); return el ? el.value.trim() : ""; };
  const bool = function (id) { const el = document.getElementById(id); return el ? el.checked : false; };
  return {
    shm_size: str("vllm_shm"),
    tensor_parallel_size: num("sg_tp") || 1,
    gpu_memory_utilization: parseFloat(document.getElementById("sg_gpu_mem_util").value) || 0,
    quantization: str("sg_quant"),
    kv_cache_dtype: str("sg_kv_dtype"),
    load_format: str("sg_load_format"),
    block_size: num("sg_block_size"),
    tokenizer_mode: str("sg_tokenizer_mode"),
    tool_call_parser: str("sg_tool_parser"),
    reasoning_parser: str("sg_reasoning_parser"),
    enable_auto_tool_choice: bool("sg_auto_tool_choice"),
    trust_remote_code: bool("sg_trust_remote"),
    max_num_seqs: num("sg_max_seqs"),
    max_num_batched_tokens: parseInt(document.getElementById("sg_max_batched_tokens").value) || 0,
    extra_args: str("sg_extra_args").trim(),
    extra_env: str("sg_extra_env").trim(),
  };
}

function fillVllmArgsForm(a) {
  const v = a || {};
  const set = function (id, val) { const el = document.getElementById(id); if (el) el.value = val; };
  const setB = function (id, val) { const el = document.getElementById(id); if (el) el.checked = !!val; };
  set("vllm_shm", v.shm_size || "");
  set("sg_tp", v.tensor_parallel_size || 1);
  set("sg_gpu_mem_util", v.gpu_memory_utilization || 0);
  set("sg_quant", v.quantization || "");
  set("sg_kv_dtype", v.kv_cache_dtype || "");
  set("sg_load_format", v.load_format || "");
  set("sg_block_size", v.block_size || 256);
  set("sg_tokenizer_mode", v.tokenizer_mode || "");
  set("sg_tool_parser", v.tool_call_parser || "");
  set("sg_reasoning_parser", v.reasoning_parser || "");
  setB("sg_auto_tool_choice", v.enable_auto_tool_choice !== false);
  setB("sg_trust_remote", v.trust_remote_code !== false);
  set("sg_max_seqs", v.max_num_seqs || 0);
  set("sg_max_batched_tokens", v.max_num_batched_tokens == null ? 0 : v.max_num_batched_tokens);
  set("sg_extra_args", v.extra_args || "");
  set("sg_extra_env", v.extra_env || "");
}

function getParamsFromForm() {
  const ctxVal = parseInt(document.getElementById("ctx_size").value) || 0;
  const portEl = document.getElementById("port");
  return {
    ctx_size: ctxVal,
    port: portEl ? (parseInt(portEl.value) || 8000) : 8000,
    host: "127.0.0.1",
  };
}

function fillFormFromParams(params) {
  const p = params || {};
  document.getElementById("ctx_size").value = p.ctx_size ?? p.ctxSize ?? DEFAULT_CTX_SIZE;
  const portEl = document.getElementById("port");
  if (portEl) portEl.value = p.port ?? 8000;
}

async function saveParams() {
  const params = getParamsFromForm();
  console.log("[settings] 保存参数:", JSON.stringify(params));
  try {
    // 加载完整设置，仅替换 launch_params 与 vllm_args
    let s = await invoke()("load_settings");
    s.launch_params = params;
    s.vllm_args = getVllmArgsFromForm();
    s.multi_node_args = getMultiNodeArgsFromForm();
    var proxyEl = document.getElementById("proxy_url");
    s.proxy_url = proxyEl ? proxyEl.value.trim() : "";
    await invoke()("save_settings", { settings: s });
    console.log("[settings] 保存成功");
    showToast(_t("设置已保存，重启模型后生效"));
  } catch (e) {
    console.error("[settings] 保存失败:", e);
    showToast(_t("保存失败: ") + e, true);
  }
}

function resetParams() {
  fillFormFromParams({ ctx_size: DEFAULT_CTX_SIZE });
  fillVllmArgsForm(null);
  autoSave();
}

function autoSave() { saveParams(); }

function setupAutoSave() {
  ["ctx_size", "port", "vllm_shm", "sg_tp", "sg_gpu_mem_util", "sg_quant", "sg_kv_dtype", "sg_load_format", "sg_block_size", "sg_tokenizer_mode", "sg_tool_parser", "sg_reasoning_parser", "sg_auto_tool_choice", "sg_trust_remote", "sg_max_seqs", "sg_max_batched_tokens", "sg_extra_args", "sg_extra_env",
   "multi_enabled", "multi_dist_port", "multi_extra_env", "proxy_url"].forEach(function (id) {
    var el = document.getElementById(id);
    if (el) el.addEventListener("change", autoSave);
  });
}

async function loadVersionInfo() {
  try {
    const admVersion = await invoke()("get_app_version");
    document.getElementById("v-adm").innerHTML = admVersion + ' <span id="update-badge" style="display:none;color:#4caf50;font-size:12px;margin-left:6px;">' + _t("✓ 最新") + '</span>';
  } catch (e) {
    document.getElementById("v-adm").textContent = _t("未知");
  }
  const platform = navigator.platform || navigator.userAgent;
  let osName = _t("未知");
  if (platform.includes("Win")) osName = "Windows";
  else if (platform.includes("Mac")) osName = "macOS";
  else if (platform.includes("Linux")) osName = "Linux";
  document.getElementById("v-os").textContent = osName;
}

let _confirmResolve = null;

function showConfirmDialog(message) {
  const overlay = document.getElementById("confirm-overlay");
  document.getElementById("confirm-message").textContent = message;
  overlay.classList.add("show");
  return new Promise((resolve) => { _confirmResolve = resolve; });
}

function closeConfirmDialog(result) {
  document.getElementById("confirm-overlay").classList.remove("show");
  if (_confirmResolve) { _confirmResolve(result); _confirmResolve = null; }
}

async function checkUpdateNow() {
  const statusEl = document.getElementById("update-status");
  const badgeEl = document.getElementById("update-badge");
  statusEl.textContent = _t("检查中...");
  badgeEl.style.display = "none";
  try {
    const result = await invoke()("check_update");
    if (result.has_update) {
      statusEl.textContent = _t("发现新版本 v") + result.remote_version;
      statusEl.style.color = "#ff9800";
      const html = `
        <div class="update-icon" style="font-size:40px;text-align:center;margin-bottom:12px;">📥</div>
        <div class="update-title" style="font-size:20px;font-weight:600;color:#fff;text-align:center;margin-bottom:8px;">${_t("发现新版本")}</div>
        <div class="update-desc" style="font-size:14px;color:var(--c-text-2);text-align:center;margin-bottom:20px;line-height:1.6;">${_t("有新版本可用，是否前往下载？")}</div>
        <div class="update-info-row" style="display:flex;justify-content:center;gap:24px;margin-bottom:20px;font-size:13px;">
          <div class="info-item" style="text-align:center;"><div class="info-label" style="color:var(--c-text-3);font-size:11px;">${_t("当前版本")}</div><div class="info-value" style="color:var(--c-text);font-weight:500;margin-top:2px;">v${escHtml(result.current_version)}</div></div>
          <div class="info-item" style="text-align:center;"><div class="info-label" style="color:var(--c-text-3);font-size:11px;">${_t("最新版本")}</div><div class="info-value" style="color:var(--c-text);font-weight:500;margin-top:2px;">v${escHtml(result.remote_version)}</div></div>
        </div>
        <div class="update-buttons" style="display:flex;gap:12px;justify-content:center;">
          <button class="update-btn-primary" style="background:var(--c-accent);color:#fff;border:none;padding:10px 28px;border-radius:8px;font-size:14px;font-weight:500;cursor:pointer;" onclick="window.ADM.hideUpdateDialog();window.openUrl('${escHtml(result.download_url)}')">${_t("下载更新")}</button>
          <button class="update-btn-secondary" style="background:var(--c-overlay);color:var(--c-text);border:none;padding:10px 28px;border-radius:8px;font-size:14px;cursor:pointer;" onclick="window.ADM.hideUpdateDialog()">${_t("稍后再说")}</button>
        </div>`;
      window.ADM.showUpdateDialog(html);
    } else {
      statusEl.textContent = _t("已是最新版本");
      statusEl.style.color = "#4caf50";
      badgeEl.style.display = "inline";
    }
  } catch (e) {
    statusEl.textContent = _t("检查失败: ") + e;
    statusEl.style.color = "#f44336";
  }
  setTimeout(() => { statusEl.textContent = ""; statusEl.style.color = "var(--c-text-3)"; }, 5000);
}

function goBack() { location.hash = "#/list"; }

// ===== 运行日志 =====

// 实时日志轮询（仅当天日志自动刷新，其他日期静态展示）
const LOG_POLL_MS = 2000;
let logPollTimer = null;

function todayStr() {
  const d = new Date();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return d.getFullYear() + "-" + m + "-" + day;
}

function stopLogPolling() {
  if (logPollTimer) { clearInterval(logPollTimer); logPollTimer = null; }
}

// 选中日期为今天时开启轮询，否则停止；date 为空（读取默认今天）也开启
function syncLogPolling(date) {
  stopLogPolling();
  if (!date || date === todayStr()) {
    logPollTimer = setInterval(function () { loadLogContent(date, true); }, LOG_POLL_MS);
  }
}

async function loadLogDates() {
  try {
    const dates = await invoke()("list_log_dates");
    const select = document.getElementById("log-date-select");
    if (!select) return;
    select.innerHTML = "";
    if (dates.length === 0) {
      stopLogPolling();
      select.innerHTML = '<option value="">' + _t("暂无日志") + '</option>';
      document.getElementById("log-content").textContent = _t("暂无日志");
      return;
    }
    dates.forEach(function(d) {
      var opt = document.createElement("option");
      opt.value = d;
      opt.textContent = d;
      select.appendChild(opt);
    });
    loadLogContent(dates[0]);
    syncLogPolling(dates[0]);
  } catch (e) {
    console.error("[settings] 加载日志日期失败:", e);
    document.getElementById("log-content").textContent = _t("加载失败: ") + e;
  }
}

async function loadLogContent(date, isPoll) {
  const pre = document.getElementById("log-content");
  if (!pre) return;
  if (!isPoll) pre.textContent = _t("加载中...");
  try {
    const content = await invoke()("read_log", { date: date });
    const text = content || _t("暂无日志");
    // 轮询时内容未变不重绘（避免闪烁/打断滚动）；手动加载总是刷新
    if (isPoll && pre.textContent === text) return;
    // 跟踪是否接近底部：轮询仅在用户停留底部时自动跟随，上翻查看历史不被打断
    const nearBottom = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 40;
    pre.textContent = text;
    if (!isPoll || nearBottom) pre.scrollTop = pre.scrollHeight;
  } catch (e) {
    if (!isPoll) pre.textContent = _t("加载失败: ") + e;
  }
}

// ===== 外观主题 =====

function renderThemeGrid() {
  const grid = document.getElementById("theme-grid");
  if (!grid) return;
  const themes = window.__ADM_THEMES || [];
  const current = (typeof window.getTheme === "function" ? window.getTheme() : "default");
  grid.innerHTML = themes.map(function (t) {
    const active = t.id === current ? " active" : "";
    const sw = (t.colors || []).map(function (c) {
      return '<span style="background:' + escHtml(c) + ';"></span>';
    }).join("");
    return '<div class="theme-card' + active + '" data-theme-id="' + escHtml(t.id) + '">' +
             '<div class="theme-preview">' + sw + '</div>' +
             '<div class="theme-name"><span class="theme-check">\u2714</span>' + escHtml(_t(t.name)) + '</div>' +
           '</div>';
  }).join("");
  grid.querySelectorAll(".theme-card").forEach(function (card) {
    card.addEventListener("click", function () {
      const id = card.dataset.themeId;
      if (typeof window.applyTheme === "function") window.applyTheme(id);
      grid.querySelectorAll(".theme-card").forEach(function (c) { c.classList.remove("active"); });
      card.classList.add("active");
      showToast(_t("已切换主题"));
    });
  });
}

// ===== Docker 镜像配置（daemon.json registry-mirrors）=====

// 解析后端错误消息：FALLBACK_MANUAL|xxx 携带手动执行命令；PKEXEC_CANCELLED / UAC_CANCELLED 为用户取消
function parseMirrorError(e) {
  const s = String(e);
  const idx = s.indexOf("FALLBACK_MANUAL|");
  if (idx !== -1) return { kind: "manual", message: s.slice(idx + "FALLBACK_MANUAL|".length) };
  if (s.indexOf("PKEXEC_CANCELLED") !== -1 || s.indexOf("UAC_CANCELLED") !== -1) return { kind: "cancelled", message: "" };
  return { kind: "error", message: s };
}

async function loadDockerMirrorConfig() {
  const statusEl = document.getElementById("mirror-status");
  try {
    const cfg = await invoke()("get_docker_mirror_config");
    const pathEl = document.getElementById("mirror-daemon-path");
    if (pathEl) pathEl.textContent = cfg.daemonPath + (cfg.exists ? "" : _t("（不存在，保存时将新建）"));
    const ta = document.getElementById("mirror-list");
    // 仅「从未配置过」时填入默认值；用户显式保存空列表（直连 Docker Hub）后保持为空，不再回填
    if (ta) ta.value = (cfg.configured ? (cfg.mirrors || []) : DEFAULT_DOCKER_MIRRORS).join("\n");
    if (statusEl) statusEl.textContent = "";
  } catch (e) {
    if (statusEl) statusEl.textContent = _t("读取失败: ") + e;
  }
}

async function saveDockerMirrorConfig() {
  const ta = document.getElementById("mirror-list");
  const statusEl = document.getElementById("mirror-status");
  const btn = document.getElementById("mirror-save-btn");
  if (!ta || btn.disabled) return;
  const mirrors = (ta.value || "").split("\n").map(function (l) { return l.trim(); }).filter(Boolean);
  btn.disabled = true;
  if (statusEl) statusEl.textContent = _t("正在写入配置并重启 Docker，请稍候...");
  try {
    const res = await invoke()("save_docker_mirror_config", { mirrors: mirrors });
    if (res === "DOCKER_RESTARTED") {
      if (statusEl) statusEl.textContent = _t("配置已写入，Docker 已重启");
      showToast(_t("镜像加速配置已生效"));
      loadDockerMirrorConfig();
    } else {
      if (statusEl) statusEl.textContent = String(res);
    }
  } catch (e) {
    const parsed = parseMirrorError(e);
    if (parsed.kind === "manual") {
      if (statusEl) statusEl.textContent = _t("需要权限，请手动执行");
      await showConfirmDialog(_t("需要管理员权限手动执行以下命令：\n\n") + parsed.message);
    } else if (parsed.kind === "cancelled") {
      if (statusEl) statusEl.textContent = _t("已取消");
    } else {
      if (statusEl) statusEl.textContent = _t("保存失败: ") + parsed.message;
    }
  } finally {
    btn.disabled = false;
  }
}

// ===== 代理配置（daemon.json proxies，镜像拉取走代理）=====

async function loadDockerProxyConfig() {
  const statusEl = document.getElementById("proxy-status");
  try {
    const cfg = await invoke()("get_docker_proxy_config");
    const pathEl = document.getElementById("proxy-daemon-path");
    if (pathEl) pathEl.textContent = cfg.daemonPath + (cfg.exists ? "" : _t("（不存在，保存时将新建）"));
    if (statusEl) statusEl.textContent = cfg.proxy ? _t("当前 daemon 代理: ") + cfg.proxy : "";
  } catch (e) {
    if (statusEl) statusEl.textContent = _t("读取失败: ") + e;
  }
}

async function saveDockerProxyConfig() {
  const input = document.getElementById("proxy_url");
  const statusEl = document.getElementById("proxy-status");
  const btn = document.getElementById("proxy-save-btn");
  if (!input || btn.disabled) return;
  const proxy = (input.value || "").trim();
  btn.disabled = true;
  if (statusEl) statusEl.textContent = _t("正在写入配置并重启 Docker，请稍候...");
  try {
    const res = await invoke()("save_docker_proxy_config", { proxy: proxy });
    if (res === "DOCKER_RESTARTED") {
      if (statusEl) statusEl.textContent = _t("配置已写入，Docker 已重启");
      showToast(proxy ? _t("代理配置已生效，镜像拉取将走代理") : _t("已清除 Docker 代理配置"));
      loadDockerProxyConfig();
    } else {
      if (statusEl) statusEl.textContent = String(res);
    }
  } catch (e) {
    const parsed = parseMirrorError(e);
    if (parsed.kind === "manual") {
      if (statusEl) statusEl.textContent = _t("需要权限，请手动执行");
      await showConfirmDialog(_t("需要管理员权限手动执行以下命令：\n\n") + parsed.message);
    } else if (parsed.kind === "cancelled") {
      if (statusEl) statusEl.textContent = _t("已取消");
    } else {
      if (statusEl) statusEl.textContent = _t("保存失败: ") + parsed.message;
    }
  } finally {
    btn.disabled = false;
  }
}

export default {
  template,
  mount(root) {
    console.log("[settings] mount()");
    root.innerHTML = template;

    // 禁用页面右键（屏蔽浏览器默认菜单；#confirm-overlay 是 #settings-app 的兄弟节点，需各自绑定）
    ["settings-app", "confirm-overlay"].forEach(function(id) {
      var el = document.getElementById(id);
      if (el) el.addEventListener("contextmenu", function(e) { e.preventDefault(); });
    });

    document.getElementById("back-btn").addEventListener("click", goBack);
    document.querySelectorAll(".nav-item").forEach(function(el) {
      el.addEventListener("click", function() { switchPanel(el.dataset.panel); });
    });
    document.getElementById("reset-btn").addEventListener("click", resetParams);
    document.getElementById("check-update-btn").addEventListener("click", checkUpdateNow);
    document.getElementById("confirm-cancel-btn").addEventListener("click", function() { closeConfirmDialog(false); });
    document.getElementById("confirm-ok-btn").addEventListener("click", function() { closeConfirmDialog(true); });

    // 运行日志
    var navLogs = document.getElementById("nav-logs");
    if (navLogs) {
      navLogs.addEventListener("click", function() { loadLogDates(); });
    }
    // Docker 镜像配置：进入面板时加载当前 daemon.json 配置
    var navMirror = document.getElementById("nav-docker-mirror");
    if (navMirror) {
      navMirror.addEventListener("click", loadDockerMirrorConfig);
    }
    var mirrorSaveBtn = document.getElementById("mirror-save-btn");
    if (mirrorSaveBtn) {
      mirrorSaveBtn.addEventListener("click", saveDockerMirrorConfig);
    }
    // 代理配置：进入面板时加载当前 daemon.json 代理
    var navProxy = document.getElementById("nav-proxy");
    if (navProxy) {
      navProxy.addEventListener("click", loadDockerProxyConfig);
    }
    var proxySaveBtn = document.getElementById("proxy-save-btn");
    if (proxySaveBtn) {
      proxySaveBtn.addEventListener("click", saveDockerProxyConfig);
    }
    var logRefresh = document.getElementById("log-refresh-btn");
    if (logRefresh) logRefresh.addEventListener("click", function() {
      var sel = document.getElementById("log-date-select");
      loadLogContent(sel ? sel.value : null);
    });
    var logOpenDir = document.getElementById("log-open-dir-btn");
    if (logOpenDir) logOpenDir.addEventListener("click", async function() {
      try { await invoke()("open_log_dir"); } catch (e) { showToast(_t("打开失败: ") + e, true); }
    });
    var logClearBtn = document.getElementById("log-clear-btn");
    if (logClearBtn) logClearBtn.addEventListener("click", async function() {
      if (!confirm(_t("确定清空所有日志吗？"))) return;
      try {
        await invoke()("clear_all_logs");
        document.getElementById("log-content").textContent = _t("暂无日志");
        var sel = document.getElementById("log-date-select");
        if (sel) sel.innerHTML = '<option value="">' + _t("暂无日志") + '</option>';
        stopLogPolling();
        showToast(_t("日志已清空"));
      } catch (e) { showToast(_t("清空失败: ") + e, true); }
    });
    var logDateSel = document.getElementById("log-date-select");
    if (logDateSel) logDateSel.addEventListener("change", function() {
      syncLogPolling(this.value);
      loadLogContent(this.value);
    });

    // 语言切换：保存到 Settings.language + localStorage，并立即重建当前视图生效
    const langSelect = document.getElementById("ui-lang-select");
    if (langSelect) {
      langSelect.value = getLanguage();
      langSelect.addEventListener("change", async function () {
        const lang = this.value;
        try {
          let current = {};
          try { current = await invoke()("load_settings"); } catch (_) {}
          await invoke()("save_settings", Object.assign({}, current, { language: lang }));
        } catch (_) {}
        setLanguage(lang);
        location.reload();
      });
    }

    setupAutoSave();
    renderThemeGrid();

    // ===== 多机互联：节点表交互（tbody 事件委托，change 写回并自动保存）=====
    var mnTbody = document.getElementById("mn-tbody");
    if (mnTbody) {
      mnTbody.addEventListener("change", function (e) {
        var el = e.target;
        if (!el.dataset || el.dataset.rank === undefined || !el.dataset.field) return;
        var i = parseInt(el.dataset.rank);
        if (!mnNodes[i]) return;
        var field = el.dataset.field;
        if (field === "sshPort") {
          mnNodes[i].sshPort = parseInt(el.value) || 22;
        } else {
          mnNodes[i][field] = el.value;
        }
        autoSave();
      });
      mnTbody.addEventListener("click", function (e) {
        var btn = e.target.closest ? e.target.closest("button") : null;
        if (!btn || !btn.dataset || btn.dataset.rank === undefined) return;
        var i = parseInt(btn.dataset.rank);
        if (btn.classList.contains("mn-probe-btn")) {
          probeNode(i);
        }
      });
    }
    var mnAddBtn = null;
    var mnProbeAllBtn = document.getElementById("mn-probe-all-btn");
    if (mnProbeAllBtn) {
      mnProbeAllBtn.addEventListener("click", probeAllNodes);
    }
    // 「互连网卡」已移除：后端自动从节点 IP 反查网卡名注入 NCCL_SOCKET_IFNAME
    // 同步入口：镜像（整体按钮）+ 模型（下拉选择本地模型）
    var mnPushImgBtn = document.getElementById("mn-push-img-btn");
    if (mnPushImgBtn) {
      mnPushImgBtn.addEventListener("click", pushImageToRemote);
    }
    var mnSyncModelBtn = document.getElementById("mn-sync-model-btn");
    if (mnSyncModelBtn) {
      mnSyncModelBtn.addEventListener("click", syncModelToRemote);
    }
    // ===== DGX 双机直连一键部署：按钮 =====
    var zdStartBtn = document.getElementById("zd-start-btn");
    if (zdStartBtn) zdStartBtn.addEventListener("click", startZdDeploy);
    zdInitForm();
    // 同步进行中切页回来：恢复按钮禁用与忙态文案
    if (mnImgSyncBusy) setMnBtnBusy(mnPushImgBtn, true, _t("镜像同步中..."));
    if (mnModelSyncBusy) setMnBtnBusy(mnSyncModelBtn, true, _t("模型同步中..."));
    // 恢复最近同步状态文本（同步中显示当前进度，已完成显示结果）
    var mnImgStatusEl = document.getElementById("mn-push-img-status");
    if (mnImgStatusEl && mnImgSyncStatus) mnImgStatusEl.textContent = mnImgSyncStatus;
    var mnModelStatusEl = document.getElementById("mn-sync-model-status");
    if (mnModelStatusEl && mnModelSyncStatus) mnModelStatusEl.textContent = mnModelSyncStatus;

    (async function() {
      try {
        const settings = await invoke()("load_settings");
        console.log("[settings] 加载设置成功, keys:", Object.keys(settings));
        const params = settings.launch_params || settings.launchParams;
        if (settings && params) fillFormFromParams(params);
        // vLLM 详细参数回填
        if (settings && settings.vllm_args) fillVllmArgsForm(settings.vllm_args);
        // 多机互联配置回填
        if (settings) await fillMultiNodeArgsForm(settings.multi_node_args);
        // 代理地址回填
        var proxyEl = document.getElementById("proxy_url");
        if (proxyEl) proxyEl.value = (settings && settings.proxy_url) || "";
      } catch (e) {
        console.error("加载设置失败:", e);
      }
      loadVersionInfo();
    })();
  },
  unmount() {
    console.log("[settings] unmount()");
    stopLogPolling();
  },
  handleTauriEvent
};
