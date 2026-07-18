export const STANDARD_MAP_NAMES = [
  "天枢云阙",
  "盐海矿镇",
  "幽邃地窟",
  "日落之城",
  "莲华古城",
  "深海明珠",
  "裂变峡谷",
  "微风岛屿",
  "森寒冬港",
  "亚海悬城",
  "霓虹町",
  "隐世修所",
  "源工重镇",
] as const;

const OBSOLETE_INCORRECT_MAP_NAMES = new Set(["幽邃迷境", "迷邃幽境"]);

export function deriveMapOptions(observedNames: Iterable<string>): string[] {
  const standardNames = new Set<string>(STANDARD_MAP_NAMES);
  const observedExtras = new Set<string>();

  for (const observedName of observedNames) {
    const mapName = observedName.trim();
    if (
      mapName &&
      !standardNames.has(mapName) &&
      !OBSOLETE_INCORRECT_MAP_NAMES.has(mapName)
    ) {
      observedExtras.add(mapName);
    }
  }

  return [
    ...STANDARD_MAP_NAMES,
    ...[...observedExtras].sort((left, right) =>
      left.localeCompare(right, "zh-CN"),
    ),
  ];
}
