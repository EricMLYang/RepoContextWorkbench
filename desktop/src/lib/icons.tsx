const PATHS = {
  stack: "M3 7l9-4 9 4-9 4-9-4m0 5 9 4 9-4M3 17l9 4 9-4",
  check: "M5 12l4 4L19 6",
  alert: "M12 8v5m0 3v.1M10 3 2 19h20L14 3z",
  arrow: "M5 12h14m-5-5 5 5-5 5",
  clock: "M12 7v5l3 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0",
  file: "M14 2H5v20h14V7zm0 0v5h5M8 12h8m-8 4h8",
  chat: "M4 4h16v12H9l-5 4z",
  chevron: "m9 5 7 7-7 7",
  edit: "M4 20h4L19 9l-4-4L4 16zM13 7l4 4",
} as const;

export type IconName = keyof typeof PATHS;

/** 16px 線條圖示（沿用舊版 path）。 */
export function Icon({ name }: { name: IconName }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={PATHS[name] || PATHS.file} />
    </svg>
  );
}
