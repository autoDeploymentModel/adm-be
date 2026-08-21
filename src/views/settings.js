// @ts-nocheck -- 历史视图暂未类型化（jsconfig checkJs 全局开启，新代码请勿加此标记）
import { t as _t, setLanguage, getLanguage } from "../i18n.js";
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
            <div class="param-label">${_t("上下文大小")}<div class="param-key">--context-length</div></div>
            <div class="param-input"><input type="number" id="ctx_size" value="25600" min="0"><div class="param-desc">${_t("模型最大上下文长度，留 0 表示使用模型自带默认值")}</div></div>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("服务参数")}</div>
          <div class="param-row">
            <div class="param-label">${_t("监听端口")}<div class="param-key">--port</div></div>
            <div class="param-input"><input type="number" id="port" value="5678" min="1" max="65535"><div class="param-desc">${_t("Docker 端口映射（容器内始终监听 0.0.0.0）")}</div></div>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("Docker 部署")}</div>
          <div class="param-row">
            <div class="param-label">${_t("镜像")}<div class="param-key">sglang_image</div></div>
            <div class="param-input"><input type="text" id="sglang_image" placeholder="lmsysorg/sglang:v0.5.17" style="max-width:400px;"><div class="param-desc">${_t("固定版本 tag，DGX Spark 推荐 lmsysorg/sglang:v0.5.17（多架构自动含 arm64）")}</div></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("共享内存")}<div class="param-key">--shm-size</div></div>
            <div class="param-input"><input type="text" id="sglang_shm" placeholder="64g" style="max-width:200px;"><div class="param-desc">${_t("DGX Spark 建议 64g，其他机型 32g")}</div></div>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("推理参数")}</div>
          <div class="param-row">
            <div class="param-label">${_t("张量并行")}<div class="param-key">--tensor-parallel-size</div></div>
            <div class="param-input"><input type="number" id="sg_tp" value="1" min="1"><div class="param-desc">${_t("单机多卡时拆分模型权重到多张 GPU（如 2 张卡填 2）；单卡填 1。2 台 Spark 跨机部署需要多节点模式，当前版本暂不支持")}</div></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("静态内存占比")}<div class="param-key">--mem-fraction-static</div></div>
            <div class="param-input"><input type="number" id="sg_mem_frac" value="0" min="0" max="1" step="0.01"><div class="param-desc">${_t("0 = 自动；KV 缓存池/权重内存占比，OOM 时调小")}</div></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("数据类型")}<div class="param-key">--dtype</div></div>
            <div class="param-input">
              <select id="sg_dtype">
                <option value="">${_t("auto（默认）")}</option>
                <option value="half">half (FP16)</option>
                <option value="float16">float16</option>
                <option value="bfloat16">bfloat16</option>
                <option value="float">float (FP32)</option>
                <option value="float32">float32</option>
              </select>
            </div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("量化方法")}<div class="param-key">--quantization</div></div>
            <div class="param-input">
              <select id="sg_quant">
                <option value="">${_t("不指定（NVFP4/FP8 模型自动从 config 解析）")}</option>
                <option value="fp8">fp8</option>
                <option value="mxfp8">mxfp8</option>
                <option value="modelopt_fp8">modelopt_fp8</option>
                <option value="modelopt_fp4">modelopt_fp4（NVFP4）</option>
                <option value="nvfp4_online">nvfp4_online（在线量化）</option>
                <option value="modelopt">modelopt</option>
                <option value="modelopt_mixed">modelopt_mixed</option>
                <option value="petit_nvfp4">petit_nvfp4</option>
                <option value="awq">awq</option>
                <option value="gptq">gptq</option>
                <option value="marlin">marlin</option>
                <option value="gptq_marlin">gptq_marlin</option>
                <option value="awq_marlin">awq_marlin</option>
                <option value="w8a8_fp8">w8a8_fp8</option>
                <option value="w8a8_int8">w8a8_int8</option>
                <option value="w4afp8">w4afp8</option>
                <option value="moe_wna16">moe_wna16</option>
                <option value="mxfp4">mxfp4</option>
                <option value="bitsandbytes">bitsandbytes</option>
                <option value="gguf">gguf</option>
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
                <option value="fp8_e5m2">fp8_e5m2</option>
                <option value="fp8_e4m3">fp8_e4m3</option>
                <option value="bf16">bf16</option>
                <option value="bfloat16">bfloat16</option>
                <option value="nvfp4">nvfp4（需 CUDA 12.8+）</option>
                <option value="fp4_mx_block16">fp4_mx_block16（需 CUDA 12.8+）</option>
              </select>
            </div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("调度策略")}<div class="param-key">--schedule-policy</div></div>
            <div class="param-input">
              <select id="sg_sched">
                <option value="">${_t("fcfs（默认）")}</option>
                <option value="lpm">lpm</option>
                <option value="random">random</option>
                <option value="dfs-weight">dfs-weight</option>
                <option value="lof">lof</option>
                <option value="priority">priority</option>
                <option value="routing-key">routing-key</option>
              </select>
            </div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("最大运行请求数")}<div class="param-key">--max-running-requests</div></div>
            <div class="param-input"><input type="number" id="sg_max_run" value="0" min="0"><div class="param-desc">${_t("0 = 自动")}</div></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("最大排队请求数")}<div class="param-key">--max-queued-requests</div></div>
            <div class="param-input"><input type="number" id="sg_max_queue" value="0" min="0"><div class="param-desc">${_t("0 = 自动")}</div></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("Chunked Prefill")}<div class="param-key">--chunked-prefill-size</div></div>
            <div class="param-input"><input type="number" id="sg_chunk" value="0" min="-1"><div class="param-desc">${_t("0 = 自动，-1 = 禁用；长提示词 OOM 时调小（如 4096）")}</div></div>
          </div>
        </div>

        <div class="param-group">
          <div class="param-group-title">${_t("模型解析与日志")}</div>
          <div class="param-row">
            <div class="param-label">${_t("推理解析器")}<div class="param-key">--reasoning-parser</div></div>
            <div class="param-input">
              <select id="sg_reasoning">
                <option value="">${_t("无")}</option>
                <option value="deepseek-r1">deepseek-r1</option>
                <option value="deepseek-v3">deepseek-v3</option>
                <option value="glm45">glm45</option>
                <option value="gpt-oss">gpt-oss</option>
                <option value="kimi">kimi</option>
                <option value="qwen3">qwen3</option>
                <option value="qwen3-thinking">qwen3-thinking</option>
                <option value="step3">step3</option>
              </select>
              <div class="param-desc">${_t("推理模型（DeepSeek/Qwen3 等）专用，分离思考内容")}</div>
            </div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("工具调用解析器")}<div class="param-key">--tool-call-parser</div></div>
            <div class="param-input">
              <select id="sg_tool">
                <option value="">${_t("无")}</option>
                <option value="qwen">qwen</option>
                <option value="qwen25">qwen25</option>
                <option value="qwen3_coder">qwen3_coder</option>
                <option value="deepseekv3">deepseekv3</option>
                <option value="deepseekv31">deepseekv31</option>
                <option value="glm">glm</option>
                <option value="glm45">glm45</option>
                <option value="glm47">glm47</option>
                <option value="gpt-oss">gpt-oss</option>
                <option value="kimi_k2">kimi_k2</option>
                <option value="llama3">llama3</option>
                <option value="mistral">mistral</option>
                <option value="pythonic">pythonic</option>
                <option value="step3">step3</option>
                <option value="gigachat3">gigachat3</option>
              </select>
            </div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("日志级别")}<div class="param-key">--log-level</div></div>
            <div class="param-input">
              <select id="sg_log_level">
                <option value="">${_t("info（默认）")}</option>
                <option value="debug">debug</option>
                <option value="warning">warning</option>
                <option value="error">error</option>
                <option value="critical">critical</option>
              </select>
            </div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("请求日志")}<div class="param-key">--log-requests</div></div>
            <div class="param-input"><div class="checkbox-wrap"><input type="checkbox" id="sg_log_requests"><span>${_t("记录所有请求的元数据/输入/输出")}</span></div></div>
          </div>
          <div class="param-row">
            <div class="param-label">${_t("监控指标")}<div class="param-key">--enable-metrics</div></div>
            <div class="param-input"><div class="checkbox-wrap"><input type="checkbox" id="sg_metrics"><span>${_t("启动 Prometheus metrics")}</span></div></div>
          </div>
        </div>

        <button class="btn-reset" id="reset-btn">${_t("恢复默认")}</button>
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

const DEFAULT_CTX_SIZE = 25600;

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

function getSglangArgsFromForm() {
  const num = function (id) { return parseInt(document.getElementById(id).value) || 0; };
  const str = function (id) { const el = document.getElementById(id); return el ? el.value.trim() : ""; };
  const bool = function (id) { const el = document.getElementById(id); return el ? el.checked : false; };
  return {
    image: str("sglang_image"),
    shm_size: str("sglang_shm"),
    context_length: 0,
    tensor_parallel_size: num("sg_tp") || 1,
    mem_fraction_static: parseFloat(document.getElementById("sg_mem_frac").value) || 0,
    dtype: str("sg_dtype"),
    quantization: str("sg_quant"),
    kv_cache_dtype: str("sg_kv_dtype"),
    schedule_policy: str("sg_sched"),
    max_running_requests: num("sg_max_run"),
    max_queued_requests: num("sg_max_queue"),
    chunked_prefill_size: parseInt(document.getElementById("sg_chunk").value) || 0,
    log_level: str("sg_log_level"),
    log_requests: bool("sg_log_requests"),
    enable_metrics: bool("sg_metrics"),
    reasoning_parser: str("sg_reasoning"),
    tool_call_parser: str("sg_tool"),
    extra_args: "",
  };
}

function fillSglangArgsForm(a) {
  const v = a || {};
  const set = function (id, val) { const el = document.getElementById(id); if (el) el.value = val; };
  const setB = function (id, val) { const el = document.getElementById(id); if (el) el.checked = !!val; };
  set("sglang_image", v.image || "");
  set("sglang_shm", v.shm_size || "");
  set("sg_tp", v.tensor_parallel_size || 1);
  set("sg_mem_frac", v.mem_fraction_static || 0);
  set("sg_dtype", v.dtype || "");
  set("sg_quant", v.quantization || "");
  set("sg_kv_dtype", v.kv_cache_dtype || "");
  set("sg_sched", v.schedule_policy || "");
  set("sg_max_run", v.max_running_requests || 0);
  set("sg_max_queue", v.max_queued_requests || 0);
  set("sg_chunk", v.chunked_prefill_size || 0);
  set("sg_log_level", v.log_level || "");
  setB("sg_log_requests", v.log_requests);
  setB("sg_metrics", v.enable_metrics);
  set("sg_reasoning", v.reasoning_parser || "");
  set("sg_tool", v.tool_call_parser || "");
}

function getParamsFromForm() {
  const ctxVal = parseInt(document.getElementById("ctx_size").value) || 0;
  const portEl = document.getElementById("port");
  return {
    ctx_size: ctxVal,
    port: portEl ? (parseInt(portEl.value) || 5678) : 5678,
    host: "127.0.0.1",
  };
}

function fillFormFromParams(params) {
  const p = params || {};
  document.getElementById("ctx_size").value = p.ctx_size ?? p.ctxSize ?? DEFAULT_CTX_SIZE;
  const portEl = document.getElementById("port");
  if (portEl) portEl.value = p.port ?? 5678;
}

async function saveParams() {
  const params = getParamsFromForm();
  console.log("[settings] 保存参数:", JSON.stringify(params));
  try {
    // 加载完整设置，仅替换 launch_params 与 sglang_args
    let s = await invoke()("load_settings");
    s.launch_params = params;
    s.sglang_args = getSglangArgsFromForm();
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
  fillSglangArgsForm(null);
  autoSave();
}

function autoSave() { saveParams(); }

function setupAutoSave() {
  ["ctx_size", "port", "host", "sglang_image", "sglang_shm", "sg_tp", "sg_mem_frac", "sg_dtype", "sg_quant", "sg_kv_dtype", "sg_sched", "sg_max_run", "sg_max_queue", "sg_chunk", "sg_log_level", "sg_reasoning", "sg_tool", "sg_log_requests", "sg_metrics"].forEach(function (id) {
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

async function loadLogDates() {
  try {
    const dates = await invoke()("list_log_dates");
    const select = document.getElementById("log-date-select");
    if (!select) return;
    select.innerHTML = "";
    if (dates.length === 0) {
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
  } catch (e) {
    console.error("[settings] 加载日志日期失败:", e);
    document.getElementById("log-content").textContent = _t("加载失败: ") + e;
  }
}

async function loadLogContent(date) {
  const pre = document.getElementById("log-content");
  if (!pre) return;
  pre.textContent = _t("加载中...");
  try {
    const content = await invoke()("read_log", { date: date });
    pre.textContent = content || _t("暂无日志");
    pre.scrollTop = pre.scrollHeight;
  } catch (e) {
    pre.textContent = _t("加载失败: ") + e;
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
        showToast(_t("日志已清空"));
      } catch (e) { showToast(_t("清空失败: ") + e, true); }
    });
    var logDateSel = document.getElementById("log-date-select");
    if (logDateSel) logDateSel.addEventListener("change", function() { loadLogContent(this.value); });

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

    (async function() {
      try {
        const settings = await invoke()("load_settings");
        console.log("[settings] 加载设置成功, keys:", Object.keys(settings));
        const params = settings.launch_params || settings.launchParams;
        if (settings && params) fillFormFromParams(params);
        // SGLang 详细参数回填
        if (settings && settings.sglang_args) fillSglangArgsForm(settings.sglang_args);
      } catch (e) {
        console.error("加载设置失败:", e);
      }
      loadVersionInfo();
    })();
  },
  unmount() {
    console.log("[settings] unmount()");
  }
};
