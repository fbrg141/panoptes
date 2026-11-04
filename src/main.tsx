import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

// Outside the Tauri app (a plain browser tab), use fixture data instead.
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  const { installMockBackend } = await import("./dev/mockBackend");
  installMockBackend();
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
