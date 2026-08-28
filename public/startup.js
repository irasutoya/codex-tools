;(function () {
  const root = document.getElementById("root")
  document.title = "Codex Tools · 正在加载"

  function reportStartupFailure() {
    if (root?.dataset.startupPending !== "true") return
    document.title = "Codex Tools · 启动失败"
    root.replaceChildren()
    const panel = document.createElement("main")
    panel.setAttribute("role", "alert")
    panel.style.cssText =
      "display:grid;min-height:100vh;place-items:center;padding:24px;font-family:system-ui,sans-serif;text-align:center"
    const content = document.createElement("div")
    const title = document.createElement("h1")
    title.textContent = "Codex Tools 未能启动"
    const message = document.createElement("p")
    message.textContent =
      "入口模块没有加载。请检查开发终端中的 Vite 或 WebView 错误，然后按 Ctrl+R 重试。"
    message.style.opacity = "0.7"
    content.append(title, message)
    panel.append(content)
    root.append(panel)
  }

  document.addEventListener(
    "contextmenu",
    (event) => {
      event.preventDefault()
    },
    true
  )

  window.addEventListener("error", reportStartupFailure, true)
})()
