export type UiIconName =
  | "menu"
  | "search"
  | "scan"
  | "filter"
  | "sort"
  | "close"
  | "star"
  | "folder"
  | "play";

type UiIconProps = {
  name: UiIconName;
  size?: number;
};

const ICON_PATHS: Record<UiIconName, string> = {
  menu: "M4 7h16M4 12h16M4 17h16",
  search:
    "m20 20-4.4-4.4M10.8 18a7.2 7.2 0 1 1 0-14.4 7.2 7.2 0 0 1 0 14.4Z",
  scan: "M8 3H5a2 2 0 0 0-2 2v3m13-5h3a2 2 0 0 1 2 2v3M8 21H5a2 2 0 0 1-2-2v-3m13 5h3a2 2 0 0 0 2-2v-3M7 12h10",
  filter: "M4 6h16M7 12h10M10 18h4",
  sort: "M8 6h12M8 12h8M8 18h4M4 5v14",
  close: "M6 6l12 12M18 6 6 18",
  star:
    "m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9L12 3Z",
  folder: "M3 7.5h7l2 2h9v8.5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7.5Z",
  play: "m9 7 8 5-8 5V7Z",
};

export function UiIcon({ name, size = 18 }: UiIconProps) {
  return (
    <svg
      aria-hidden="true"
      className="ui-icon"
      fill="none"
      height={size}
      viewBox="0 0 24 24"
      width={size}
    >
      <path
        d={ICON_PATHS[name]}
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.7"
      />
    </svg>
  );
}
