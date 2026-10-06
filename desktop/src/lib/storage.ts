/** 本機偏好（範圍、展開的牌組、草稿、上次選的 agent）。鍵名與 Python 版前端相同（workbench:<key>）。 */
const PREFIX = "workbench:";

export function save(key: string, value: unknown): void {
  try {
    localStorage.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    /* 私密模式／配額滿：只是不記住 */
  }
}

export function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(PREFIX + key);
    if (raw == null) return fallback;
    return (JSON.parse(raw) as T) ?? fallback;
  } catch {
    return fallback;
  }
}
