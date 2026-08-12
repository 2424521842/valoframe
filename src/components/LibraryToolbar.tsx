import {
  ListBullets,
  MagnifyingGlass,
  SquaresFour,
  X,
} from "@phosphor-icons/react";
import { useId, useMemo, useState } from "react";
import { buildLibrarySearchSuggestionGroups } from "../lib/libraryFlow";
import type {
  AccountSummary,
  ClipSort,
  HighlightFilter,
  LibraryDatePreset,
  LibraryViewMode,
  Tag,
} from "../types";
import { videoTypeLabel, type VideoTypeFilter } from "../lib/videoTypes";
import {
  UiCommand,
  UiCommandEmpty,
  UiCommandGroup,
  UiCommandInput,
  UiCommandItem,
  UiCommandList,
} from "./ui/command";
import { UiPopover, UiPopoverAnchor, UiPopoverContent } from "./ui/popover";
import {
  UiSelect,
  UiSelectContent,
  UiSelectItem,
  UiSelectTrigger,
  UiSelectValue,
} from "./ui/select";
import {
  UiTooltip,
  UiTooltipContent,
  UiTooltipProvider,
  UiTooltipTrigger,
} from "./ui/tooltip";

type LibraryToolbarProps = {
  query: string;
  accounts: AccountSummary[];
  accountId: string;
  agentNames: string[];
  agentName: string;
  mapNames: string[];
  mapName: string;
  gameModes: string[];
  gameMode: string;
  tags: Tag[];
  tagId: string;
  datePreset: LibraryDatePreset;
  highlightFilter: HighlightFilter;
  videoTypes: readonly VideoTypeFilter[];
  sortBy: ClipSort;
  viewMode: LibraryViewMode;
  hasActiveFilters: boolean;
  onQueryChange: (query: string) => void;
  onAccountChange: (value: string) => void;
  onAgentChange: (value: string) => void;
  onMapChange: (value: string) => void;
  onGameModeChange: (value: string) => void;
  onTagChange: (value: string) => void;
  onDatePresetChange: (value: LibraryDatePreset) => void;
  onHighlightFilterChange: (value: HighlightFilter) => void;
  onSortChange: (value: ClipSort) => void;
  onViewModeChange: (value: LibraryViewMode) => void;
  onClearAll: () => void;
};

type FilterOption = {
  label: string;
  value: string;
};

export function LibraryToolbar({
  query,
  accounts,
  accountId,
  agentNames,
  agentName,
  mapNames,
  mapName,
  gameModes,
  gameMode,
  tags,
  tagId,
  datePreset,
  highlightFilter,
  videoTypes,
  sortBy,
  viewMode,
  hasActiveFilters,
  onQueryChange,
  onAccountChange,
  onAgentChange,
  onMapChange,
  onGameModeChange,
  onTagChange,
  onDatePresetChange,
  onHighlightFilterChange,
  onSortChange,
  onViewModeChange,
  onClearAll,
}: LibraryToolbarProps) {
  const [searchOpen, setSearchOpen] = useState(false);
  const suggestionGroups = useMemo(() => buildLibrarySearchSuggestionGroups({
    accounts: accounts.map((account) => account.displayName),
    agents: agentNames,
    maps: mapNames,
    tags: tags.map((tag) => tag.label),
  }), [accounts, agentNames, mapNames, tags]);

  const chooseSuggestion = (value: string) => {
    onQueryChange(value);
    setSearchOpen(false);
  };

  return (
    <section className="library-command-bar" aria-label="素材检索与筛选">
      <div className="library-search-row">
        <UiCommand className="library-search-command" shouldFilter>
          <UiPopover open={searchOpen} onOpenChange={setSearchOpen}>
            <UiPopoverAnchor asChild>
              <label className="library-global-search">
                <MagnifyingGlass aria-hidden="true" weight="bold" />
                <UiCommandInput
                  aria-label="全局搜索素材"
                  placeholder="搜索账号、英雄、地图、标签、文件名…"
                  value={query}
                  onFocus={() => setSearchOpen(true)}
                  onValueChange={(value) => {
                    onQueryChange(value);
                    setSearchOpen(true);
                  }}
                />
              </label>
            </UiPopoverAnchor>
            <UiPopoverContent
              align="start"
              className="library-search-popover"
              onCloseAutoFocus={(event) => event.preventDefault()}
              onOpenAutoFocus={(event) => event.preventDefault()}
            >
              <UiCommandList>
                <UiCommandEmpty>没有匹配的账号、英雄、地图或标签</UiCommandEmpty>
                {suggestionGroups.map((group) => (
                  <UiCommandGroup heading={group.label} key={group.label}>
                    {group.values.map((value) => (
                      <UiCommandItem
                        key={`${group.label}:${value}`}
                        value={`${group.label} ${value}`}
                        onSelect={() => chooseSuggestion(value)}
                      >
                        <span>{value}</span>
                        <small>{group.label}</small>
                      </UiCommandItem>
                    ))}
                  </UiCommandGroup>
                ))}
              </UiCommandList>
            </UiPopoverContent>
          </UiPopover>
        </UiCommand>

        <UiTooltipProvider delayDuration={300}>
          <div className="library-search-actions">
            {hasActiveFilters ? (
              <UiTooltip>
                <UiTooltipTrigger asChild>
                  <button
                    aria-label="清空搜索与所有筛选"
                    className="library-clear-button"
                    type="button"
                    onClick={onClearAll}
                  >
                    <X weight="bold" />
                  </button>
                </UiTooltipTrigger>
                <UiTooltipContent side="bottom">清空搜索与所有筛选</UiTooltipContent>
              </UiTooltip>
            ) : null}

            <div className="library-view-switch" aria-label="素材视图">
              <ViewToggle
                active={viewMode === "grid"}
                label="网格视图"
                onClick={() => onViewModeChange("grid")}
              >
                <SquaresFour weight={viewMode === "grid" ? "fill" : "regular"} />
              </ViewToggle>
              <ViewToggle
                active={viewMode === "list"}
                label="列表视图"
                onClick={() => onViewModeChange("list")}
              >
                <ListBullets weight={viewMode === "list" ? "bold" : "regular"} />
              </ViewToggle>
            </div>
          </div>
        </UiTooltipProvider>
      </div>

      <div className="library-filter-row">
        <div className="library-filter-controls">
          <FilterSelect
            label="账号"
            options={[
              { label: "全部账号", value: "all" },
              ...accounts.map((account) => ({ label: account.displayName, value: account.id })),
            ]}
            value={accountId}
            onChange={onAccountChange}
          />
          <OptionFilter label="英雄" value={agentName} options={agentNames} onChange={onAgentChange} />
          <OptionFilter label="地图" value={mapName} options={mapNames} onChange={onMapChange} />
          <OptionFilter label="模式" value={gameMode} options={gameModes} onChange={onGameModeChange} />
          <FilterSelect
            label="日期"
            options={[
              { label: "全部日期", value: "all" },
              { label: "今天", value: "today" },
              { label: "近 7 天", value: "week" },
              { label: "近 30 天", value: "month" },
            ]}
            value={datePreset}
            onChange={(value) => onDatePresetChange(value as LibraryDatePreset)}
          />
          <FilterSelect
            label="视频类型"
            options={[
              { label: "全部类型", value: "all" },
              ...videoTypes.map((value) => ({
                label: videoTypeLabel(value),
                value,
              })),
            ]}
            value={highlightFilter}
            onChange={(value) => onHighlightFilterChange(value as HighlightFilter)}
          />
          <FilterSelect
            label="自定义标签"
            options={[
              { label: "全部自定义标签", value: "all" },
              ...tags.map((tag) => ({ label: tag.label, value: tag.id })),
            ]}
            value={tagId}
            onChange={onTagChange}
          />
        </div>

        <div className="library-sort-control">
          <FilterSelect
            label="排序"
            options={[
              { label: "最新优先", value: "modified-desc" },
              { label: "最早优先", value: "modified-asc" },
              { label: "体积最大", value: "size-desc" },
              { label: "体积最小", value: "size-asc" },
              { label: "文件名", value: "name-asc" },
            ]}
            value={sortBy}
            onChange={(value) => onSortChange(value as ClipSort)}
          />
        </div>
      </div>
    </section>
  );
}

function FilterSelect({ label, value, options, onChange }: {
  label: string;
  value: string;
  options: FilterOption[];
  onChange: (value: string) => void;
}) {
  const labelId = useId();

  return (
    <div className="library-filter-select">
      <span id={labelId}>{label}</span>
      <UiSelect value={value} onValueChange={onChange}>
        <UiSelectTrigger aria-labelledby={labelId}>
          <UiSelectValue />
        </UiSelectTrigger>
        <UiSelectContent className="library-filter-select-content">
          {options.map((option) => (
            <UiSelectItem key={option.value} value={option.value}>{option.label}</UiSelectItem>
          ))}
        </UiSelectContent>
      </UiSelect>
    </div>
  );
}

function OptionFilter({ label, value, options, onChange }: {
  label: string;
  value: string;
  options: string[];
  onChange: (value: string) => void;
}) {
  return (
    <FilterSelect
      label={label}
      options={[
        { label: `全部${label}`, value: "all" },
        ...options.map((option) => ({ label: option, value: option })),
      ]}
      value={value}
      onChange={onChange}
    />
  );
}

function ViewToggle({ active, label, children, onClick }: {
  active: boolean;
  label: string;
  children: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <UiTooltip>
      <UiTooltipTrigger asChild>
        <button aria-label={label} aria-pressed={active} type="button" onClick={onClick}>
          {children}
        </button>
      </UiTooltipTrigger>
      <UiTooltipContent side="top">{label}</UiTooltipContent>
    </UiTooltip>
  );
}
