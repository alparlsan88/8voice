import React from "react";
import ReactDOM from "react-dom/client";
import Widget from "./Widget";
import "../index.css";

ReactDOM.createRoot(
  document.getElementById("widget-root") as HTMLElement,
).render(
  <React.StrictMode>
    <Widget />
  </React.StrictMode>,
);
