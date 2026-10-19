// Mounts the real application on top of the IPC mock (development only).
import "./tauri-mock";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "../src/App";
import "../src/styles.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
