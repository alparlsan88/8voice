import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import Widget from "./Widget";
import "../index.css";

// Show a native context menu on right-click with a "Quit" option.
window.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  invoke("cmd_widget_context_menu", {
    x: e.clientX,
    y: e.clientY,
  }).catch(console.error);
});

ReactDOM.createRoot(
  document.getElementById("widget-root") as HTMLElement,
).render(
  <React.StrictMode>
    <Widget />
  </React.StrictMode>,
);
