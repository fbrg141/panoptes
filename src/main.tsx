import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

// Outside the Tauri app (a plain browser tab), use fixture data instead.
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  const { installMockBackend } = await import("./dev/mockBackend");
  installMockBackend();
}

// The design system is dark first; follow the system when it asks for light.
const light = window.matchMedia("(prefers-color-scheme: light)");
const applyTheme = () => {
  document.documentElement.dataset.theme = light.matches ? "light" : "dark";
};
applyTheme();
light.addEventListener("change", applyTheme);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
