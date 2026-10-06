import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";
import { detectBackend } from "./api";
import { App } from "./App";
import { WorkbenchStore } from "./store/store";

const store = new WorkbenchStore(detectBackend());

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App store={store} />
  </StrictMode>,
);
