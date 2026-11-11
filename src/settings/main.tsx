import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../shared/plates.css";
import "./settings.css";
import { installDebugUi, watchSystemTheme } from "../shared/api";
import { App } from "./App";

installDebugUi();
watchSystemTheme();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
