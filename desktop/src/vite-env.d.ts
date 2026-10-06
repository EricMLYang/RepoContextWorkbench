/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** HTTP 後端的 token（網址沒有 ?token= 時用）。 */
  readonly VITE_WORKBENCH_TOKEN?: string;
  /** HTTP 後端位址（不經 Vite 代理直連時用，例：http://127.0.0.1:8765）。 */
  readonly VITE_WORKBENCH_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
