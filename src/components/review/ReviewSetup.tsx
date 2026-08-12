import {
  ArrowRight,
  CalendarDots,
  CheckSquare,
  FunnelSimple,
  ListChecks,
  MagnifyingGlass,
  Sparkle,
  X,
} from "@phosphor-icons/react";
import { useId, useState } from "react";
import { reviewSessionCounts } from "../../lib/reviewSessions";
import { videoTypeLabel, type VideoTypeFilter } from "../../lib/videoTypes";
import type {
  AccountSummary,
  HighlightFilter,
  LibraryDatePreset,
  ReviewCandidateScope,
  ReviewSession,
  ReviewSessionSort,
  Tag,
} from "../../types";
import {
  UiSelect,
  UiSelectContent,
  UiSelectItem,
  UiSelectTrigger,
  UiSelectValue,
} from "../ui/select";

export type ReviewScopeEditor = {
  query: string;
  accounts: readonly AccountSummary[];
  accountId: string;
  agentNames: readonly string[];
  agentName: string;
  mapNames: readonly string[];
  mapName: string;
  gameModes: readonly string[];
  gameMode: string;
  tags: readonly Tag[];
  tagId: string;
  datePreset: LibraryDatePreset;
  highlightFilter: HighlightFilter;
  videoTypes: readonly VideoTypeFilter[];
  onQueryChange: (value: string) => void;
  onAccountChange: (value: string) => void;
  onAgentChange: (value: string) => void;
  onMapChange: (value: string) => void;
  onGameModeChange: (value: string) => void;
  onTagChange: (value: string) => void;
  onDatePresetChange: (value: LibraryDatePreset) => void;
  onHighlightFilterChange: (value: HighlightFilter) => void;
  onClearFilters: () => void;
};

type ReviewSetupProps = {
  filterLabels: readonly string[];
  scopeEditor: ReviewScopeEditor;
  candidateCount: number;
  sort: ReviewSessionSort;
  candidateScope: ReviewCandidateScope;
  resumableSession: ReviewSession | null;
  isPreparing: boolean;
  error: string | null;
  onSortChange: (sort: ReviewSessionSort) => void;
  onCandidateScopeChange: (scope: ReviewCandidateScope) => void;
  onStart: () => void;
  onResume: () => void;
};

const SORT_OPTIONS: Array<{ value: ReviewSessionSort; label: string; detail: string }> = [
  { value: "latest", label: "最新优先", detail: "从最近录制的素材开始" },
  { value: "oldest", label: "最早优先", detail: "从最早录制的素材开始" },
  { value: "kills", label: "击杀数优先", detail: "优先浏览击杀数更多的对局" },
  { value: "score", label: "战绩评分优先", detail: "优先浏览战斗评分更高的对局" },
  { value: "library", label: "继承素材库排序", detail: "保留素材库当前的排列顺序" },
];

const SCOPE_OPTIONS: Array<{
  value: ReviewCandidateScope;
  label: string;
  detail: string;
  icon: typeof ListChecks;
}> = [
  { value: "all", label: "全部素材", detail: "在当前素材库范围内逐条挑片", icon: ListChecks },
  { value: "not-selected", label: "仅尚未挑选", detail: "排除过去会话中已入选的素材", icon: CheckSquare },
  { value: "recent", label: "最近新增", detail: "按录制时间筛选最近 7 天的素材", icon: CalendarDots },
];

export function ReviewSetup({
  filterLabels,
  scopeEditor,
  candidateCount,
  sort,
  candidateScope,
  resumableSession,
  isPreparing,
  error,
  onSortChange,
  onCandidateScopeChange,
  onStart,
  onResume,
}: ReviewSetupProps) {
  const resumeCounts = resumableSession ? reviewSessionCounts(resumableSession) : null;
  const hasCandidates = candidateCount > 0;
  const shouldResume = resumableSession !== null;
  const [isScopeEditorOpen, setIsScopeEditorOpen] = useState(false);

  return (
    <section aria-labelledby="review-setup-title" className="review-workspace review-setup">
      <header className="review-setup-heading">
        <div>
          <h1 id="review-setup-title">快速挑片</h1>
          <p>继承素材库当前范围，专注决定这一轮真正要使用的素材。</p>
        </div>
        <button
          aria-controls="review-scope-editor"
          aria-expanded={isScopeEditorOpen}
          className="cinematic-button cinematic-button--secondary"
          type="button"
          onClick={() => setIsScopeEditorOpen((open) => !open)}
        >
          <FunnelSimple weight="bold" />{isScopeEditorOpen ? "收起条件" : "修改范围"}
        </button>
      </header>

      {resumableSession && resumeCounts ? (
        <section className="review-resume-strip" aria-label="可继续的快速挑片会话">
          <div>
            <strong>有一轮未完成的挑片</strong>
            <span>已浏览 {resumeCounts.reviewed} / {resumeCounts.total} · 已入选 {resumeCounts.selected} · 待定 {resumeCounts.pending}</span>
          </div>
          <button className="cinematic-button cinematic-button--secondary" disabled={isPreparing} type="button" onClick={onResume}>
            继续上次挑片 <ArrowRight weight="bold" />
          </button>
        </section>
      ) : null}

      {isScopeEditorOpen ? <ReviewScopeEditor controls={scopeEditor} /> : null}

      <div className="review-setup-grid">
        <section className="review-setup-section review-setup-section--scope" aria-labelledby="review-scope-title">
          <div className="review-section-heading">
            <div>
              <span className="review-section-icon"><FunnelSimple weight="duotone" /></span>
              <h2 id="review-scope-title">当前范围</h2>
            </div>
            <strong aria-live="polite">{isPreparing ? "正在计算…" : `${candidateCount} 条素材`}</strong>
          </div>
          <div className="review-scope-chips">
            {filterLabels.length > 0 ? filterLabels.map((label) => (
              <span key={label}>{label}</span>
            )) : <span>素材库当前的全部可用素材</span>}
          </div>
          <p>可在此页修改账号、英雄、地图、模式、日期、视频类型、标签或搜索条件。</p>
        </section>

        <fieldset className="review-setup-section review-option-fieldset">
          <legend>排序</legend>
          <div className="review-option-list review-option-list--compact">
            {SORT_OPTIONS.map((option) => (
              <label className={sort === option.value ? "review-option review-option--selected" : "review-option"} key={option.value}>
                <input
                  checked={sort === option.value}
                  name="review-sort"
                  type="radio"
                  value={option.value}
                  onChange={() => onSortChange(option.value)}
                />
                <span><strong>{option.label}</strong><small>{option.detail}</small></span>
              </label>
            ))}
          </div>
        </fieldset>

        <fieldset className="review-setup-section review-option-fieldset">
          <legend>候选范围</legend>
          <div className="review-option-list">
            {SCOPE_OPTIONS.map((option) => {
              const Icon = option.icon;
              return (
                <label className={candidateScope === option.value ? "review-option review-option--selected" : "review-option"} key={option.value}>
                  <input
                    checked={candidateScope === option.value}
                    name="review-candidate-scope"
                    type="radio"
                    value={option.value}
                    onChange={() => onCandidateScopeChange(option.value)}
                  />
                  <Icon weight="duotone" />
                  <span><strong>{option.label}</strong><small>{option.detail}</small></span>
                </label>
              );
            })}
          </div>
        </fieldset>
      </div>

      <footer className="review-setup-footer">
        <div aria-live="polite">
          <span className="review-section-icon"><Sparkle weight="duotone" /></span>
          <p>{hasCandidates ? "入选只会写入本轮会话；收藏、标签和回收站保持原样。" : "当前范围没有可挑片素材，请修改范围后重试。"}</p>
          {error ? <small role="alert">{error}</small> : null}
        </div>
        <button
          className="cinematic-button cinematic-button--primary"
          disabled={isPreparing || (!shouldResume && !hasCandidates)}
          type="button"
          onClick={shouldResume ? onResume : onStart}
        >
          {shouldResume ? "继续挑片" : "开始挑片"} <ArrowRight weight="bold" />
        </button>
      </footer>
    </section>
  );
}

type SelectOption = {
  label: string;
  value: string;
};

function ReviewScopeEditor({ controls }: { controls: ReviewScopeEditor }) {
  return (
    <section className="review-scope-editor" id="review-scope-editor" aria-labelledby="review-scope-editor-title">
      <div className="review-scope-editor-heading">
        <div>
          <h2 id="review-scope-editor-title">调整筛选范围</h2>
          <p>条件会直接更新本轮候选素材，无需返回素材库。</p>
        </div>
        <button className="review-scope-clear" type="button" onClick={controls.onClearFilters}>
          <X weight="bold" />重置条件
        </button>
      </div>

      <div className="review-scope-controls">
        <label className="review-scope-search">
          <span>搜索素材</span>
          <span className="review-scope-search-control">
            <MagnifyingGlass aria-hidden="true" weight="bold" />
            <input
              aria-label="搜索素材"
              placeholder="搜索账号、英雄、地图、标签、文件名…"
              value={controls.query}
              onChange={(event) => controls.onQueryChange(event.target.value)}
            />
          </span>
        </label>
        <ReviewScopeSelect
          label="账号"
          options={[
            { label: "全部账号", value: "all" },
            ...controls.accounts.map((account) => ({ label: account.displayName, value: account.id })),
          ]}
          value={controls.accountId}
          onChange={controls.onAccountChange}
        />
        <ReviewScopeSelect
          label="英雄"
          options={allOption("英雄", controls.agentNames)}
          value={controls.agentName}
          onChange={controls.onAgentChange}
        />
        <ReviewScopeSelect
          label="地图"
          options={allOption("地图", controls.mapNames)}
          value={controls.mapName}
          onChange={controls.onMapChange}
        />
        <ReviewScopeSelect
          label="模式"
          options={allOption("模式", controls.gameModes)}
          value={controls.gameMode}
          onChange={controls.onGameModeChange}
        />
        <ReviewScopeSelect
          label="日期"
          options={[
            { label: "全部日期", value: "all" },
            { label: "今天", value: "today" },
            { label: "近 7 天", value: "week" },
            { label: "近 30 天", value: "month" },
          ]}
          value={controls.datePreset}
          onChange={(value) => controls.onDatePresetChange(value as LibraryDatePreset)}
        />
        <ReviewScopeSelect
          label="视频类型"
          options={[
            { label: "全部类型", value: "all" },
            ...controls.videoTypes.map((value) => ({ label: videoTypeLabel(value), value })),
          ]}
          value={controls.highlightFilter}
          onChange={(value) => controls.onHighlightFilterChange(value as HighlightFilter)}
        />
        <ReviewScopeSelect
          label="自定义标签"
          options={[
            { label: "全部自定义标签", value: "all" },
            ...controls.tags.map((tag) => ({ label: tag.label, value: tag.id })),
          ]}
          value={controls.tagId}
          onChange={controls.onTagChange}
        />
      </div>
    </section>
  );
}

function ReviewScopeSelect({ label, options, value, onChange }: {
  label: string;
  options: readonly SelectOption[];
  value: string;
  onChange: (value: string) => void;
}) {
  const labelId = useId();

  return (
    <div className="review-scope-filter">
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

function allOption(label: string, options: readonly string[]): SelectOption[] {
  return [
    { label: `全部${label}`, value: "all" },
    ...options.map((option) => ({ label: option, value: option })),
  ];
}
