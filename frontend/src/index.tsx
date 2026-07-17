import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";

import "@uiw/react-md-editor/markdown-editor.css";
import "./index.css";

const root = document.getElementById("root");

if (!root) {
  throw new Error("No root element found");
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
