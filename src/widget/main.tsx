import React from "react";
import ReactDOM from "react-dom/client";
import Widget from "./Widget";
import "../index.css";

// Disable the default browser context menu in the floating widget.
window.addEventListener("contextmenu", (e) => e.preventDefault());

ReactDOM.createRoot(
  document.getElementById("widget-root") as HTMLElement,
).render(
  <React.StrictMode>
    <Widget />
  </React.StrictMode>,
);
