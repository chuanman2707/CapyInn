import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { installGlobalCrashHandlers } from "@/lib/crashReporting/globalHandlers";
import "./index.css";

installGlobalCrashHandlers();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
