import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";
import { detectBackend } from "./api";
import { createSpineApi, isTauri } from "./api/tauri";
import { App } from "./App";
import { SpineSetup } from "./components/SpineSetup";
import { WorkbenchStore } from "./store/store";

const root = createRoot(document.getElementById("root")!);
const store = new WorkbenchStore(detectBackend());

function render(needsSpine: boolean) {
  root.render(<StrictMode>{needsSpine ? <Setup /> : <App store={store} />}</StrictMode>);
}

/** 桌面殼：spine 設好之前先顯示首次設定；設好就切到工作台。 */
function Setup() {
  const [api] = useState(() => createSpineApi());
  return <SpineSetup setSpine={async (p) => render(!(await api.setSpine(p)).spine)} />;
}

if (isTauri()) {
  createSpineApi()
    .getSpine()
    .then((info) => render(!info.spine))
    .catch(() => render(false)); // 取不到就照常開工作台，錯誤由 get_state 顯示
} else {
  render(false);
}
