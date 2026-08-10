import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import { ThemeProvider } from "next-themes"

import "./index.css"
import App from "./App.tsx"

const requestedTheme = import.meta.env.DEV
  ? new URLSearchParams(window.location.search).get("theme")
  : null
const previewTheme =
  requestedTheme === "light" || requestedTheme === "dark"
    ? requestedTheme
    : undefined

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ThemeProvider
      attribute="class"
      defaultTheme="system"
      enableSystem
      forcedTheme={previewTheme}
      disableTransitionOnChange
      storageKey="codex-tools-theme"
    >
      <App />
    </ThemeProvider>
  </StrictMode>
)
