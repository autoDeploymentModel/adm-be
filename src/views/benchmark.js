// @ts-nocheck
import { t as _t } from "../i18n.js";
const template = `
<style>
  #benchmark-root {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 20px;
  }

  .page-title {
    font-size: 18px;
    font-weight: 600;
    color: var(--c-text-hi);
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 20px;
  }

  .page-title::before {
    content: "";
    display: inline-block;
    width: 4px;
    height: 18px;
    background: var(--c-accent);
    border-radius: 2px;
  }

  .bench-form {
    display: flex;
    gap: 16px;
    align-items: flex-end;
    margin-bottom: 20px;
    flex-wrap: wrap;
  }

  .form-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .form-item label {
    font-size: 13px;
    color: var(--c-text-2);
  }

  .form-item input {
    width: 120px;
    padding: 7px 12px;
    background: var(--c-panel-2);
    border: 1px solid var(--c-border);
    border-radius: 6px;
    color: var(--c-text);
    font-size: 13px;
    outline: none;
  }

  .form-item input:focus {
    border-color: var(--c-accent);
  }

  .bench-btn {
    background: var(--c-accent);
    color: #fff;
    border: none;
    padding: 8px 24px;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .bench-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .bench-btn:hover:not(:disabled) {
    background: var(--c-accent-2);
  }

  #bench-output {
    flex: 1;
    background: var(--c-bg-deep);
    color: var(--c-text-2);
    font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', Menlo, monospace;
    font-size: 12px;
    line-height: 1.6;
    padding: 16px;
    border-radius: 8px;
    border: 1px solid var(--c-border);
    overflow-y: auto;
    white-space: pre-wrap;
    word-break: break-all;
    margin: 0;
    min-height: 200px;
  }

  .bench-empty {
    color: var(--c-text-4);
    text-align: center;
    padding: 40px;
  }
</style>
<div id="benchmark-root">
  <div class="page-title">${_t("性能测试")}</div>
  <div class="bench-form">
    <div class="form-item">
      <label>${_t("输入长度 (tokens)")}</label>
      <input type="number" id="bench-input-len" value="1024" min="1">
    </div>
    <div class="form-item">
      <label>${_t("输出长度 (tokens)")}</label>
      <input type="number" id="bench-output-len" value="256" min="1">
    </div>
    <div class="form-item">
      <label>${_t("请求数")}</label>
      <input type="number" id="bench-num-prompts" value="5" min="1">
    </div>
    <button class="bench-btn" id="bench-start-btn">${_t("开始测试")}</button>
  </div>
  <pre id="bench-output"><span class="bench-empty">${_t("点击开始测试，测试结果将显示在此处")}</span></pre>
</div>
`;

const invoke = () => window.__adm_invoke;
const S = () => window.__adm_state;

export default {
  template,
  mount(root) {
    root.innerHTML = template;

    var btn = document.getElementById("bench-start-btn");
    var output = document.getElementById("bench-output");

    btn.addEventListener("click", async function() {
      var inputLen = parseInt(document.getElementById("bench-input-len").value) || 1024;
      var outputLen = parseInt(document.getElementById("bench-output-len").value) || 256;
      var numPrompts = parseInt(document.getElementById("bench-num-prompts").value) || 5;

      btn.disabled = true;
      btn.textContent = _t("测试中...");
      output.innerHTML = '<span class="bench-empty">' + _t("测试启动中...") + '</span>';

      try {
        await invoke()("start_benchmark", {
          params: { inputLen: inputLen, outputLen: outputLen, numPrompts: numPrompts }
        });
      } catch (e) {
        output.textContent = _t("测试启动失败: ") + e;
        btn.disabled = false;
        btn.textContent = _t("开始测试");
      }
    });

    // 监听测试日志
    window.__adm_listen("benchmark-log", function(event) {
      var line = event.payload.line || "";
      if (output.querySelector(".bench-empty")) {
        output.innerHTML = "";
      }
      output.textContent += line + "\n";
      output.scrollTop = output.scrollHeight;
    });

    // 监听测试完成
    window.__adm_listen("benchmark-complete", function(event) {
      btn.disabled = false;
      btn.textContent = _t("开始测试");
      if (!event.payload.success) {
        output.textContent += "\n" + _t("测试未成功完成") + "\n";
      }
    });
  },
  unmount() {
    console.log("[benchmark] unmount()");
  }
};
