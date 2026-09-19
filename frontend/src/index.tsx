import React from "react";
import ReactDOM from "react-dom/client";
import { RouterProvider } from "react-router-dom";
import { createRouter } from "./App";
import { initAuth } from "./auth/session";

import "@uiw/react-md-editor/markdown-editor.css";
import "./index.css";

const root = document.getElementById("root");

if (!root) {
  throw new Error("No root element found");
}

const app = ReactDOM.createRoot(root);

// The auth mode decides which routes exist, so it has to be known before the
// router is built.
initAuth().then((mode) => {
  app.render(
    <React.StrictMode>
      <RouterProvider router={createRouter(mode)} />
    </React.StrictMode>,
  );
});
