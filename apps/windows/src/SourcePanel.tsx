import { useState } from "react";
import {
  gameMatchStatusLabel,
  sourceChipLabel,
  sourceCoverageText,
  sourceLastAttemptText,
  sourceLastSuccessText,
  sourceNextDueText,
  sourceRoleOf,
  sourceStatusLabel,
  sourceStatusTone,
  SOURCE_INTERVAL_PRESETS,
  taskForSource,
  taskProgressText,
  taskKindLabel,
  type GameSourceSubscription,
  type SourceSummary,
  type SourceTaskStatus,
} from "./sources";

const FALLBACK_SOURCE_HINT = "补查来源：仅在重新匹配时按游戏查询，不参与自动更新。";

export type SourcePanelProps = {
  open: boolean;
  onClose: () => void;
  globalSources: SourceSummary[] | null;
  gameSources: GameSourceSubscription[] | null;
  gameName: string | null;
  gameInstanceId: string | null;
  preferredLanguage: string | null;
  settingsRevision: number | null;
  loading: boolean;
  busy: string | null;
  error: string | null;
  notice: string | null;
  tasks: SourceTaskStatus[];
  onReload: () => void;
  onMatchAllGames: () => void;
  onUpdateSource: (sourceId: string) => void;
  onSetInterval: (sourceId: string, intervalHours: number) => void;
  onCancelTask: (taskId: string) => void;
  onRematchGame: () => void;
  onLanguageChange: (language: string | null) => void;
  onToggleSource: (sourceId: string, enabled: boolean) => void;
  onToggleGameSource: (sourceId: string, subscribed: boolean) => void;
  onAddSource: (rule: string) => void;
  onOpenUrl: (url: string) => void;
};

function Toggle({ checked, disabled, label, onChange }: { checked: boolean; disabled?: boolean; label: string; onChange: (value: boolean) => void }) {
  return <button type="button" role="switch" aria-checked={checked} aria-label={label} className={`toggle${checked ? " on" : ""}`} disabled={disabled} onClick={() => onChange(!checked)}>
    <span className="dot" aria-hidden="true" />{checked ? "已启用" : "已停用"}
  </button>;
}

function ProgressBar({ task }: { task: SourceTaskStatus }) {
  const progress = task.progress;
  const width = progress && progress.total > 0 ? `${Math.min(100, progress.completed / progress.total * 100)}%` : "35%";
  return <div className={`progress-bar${progress && progress.total > 0 ? "" : " indeterminate"}`}>
    <div className="progress-fill" style={{ width }} />
  </div>;
}

function SourceCard({ source, busy, task, onToggle, onUpdate, onSetInterval, onCancel, onOpenUrl }: {
  source: SourceSummary;
  busy: boolean;
  task: SourceTaskStatus | undefined;
  onToggle: (enabled: boolean) => void;
  onUpdate: () => void;
  onSetInterval: (hours: number) => void;
  onCancel: () => void;
  onOpenUrl: (url: string) => void;
}) {
  const running = task?.running === true;
  return <div className="source-item">
    <div className="source-item-head">
      <strong>{source.name}</strong>
      <span className={`chip ${running ? "busy" : sourceStatusTone(source.status)}`}>{sourceChipLabel(source.status, running)}</span>
    </div>
    <dl className="source-meta">
      <dt>站点</dt><dd>{source.pages?.length ? source.pages.join(" · ") : "—"}</dd>
      <dt>索引条目</dt><dd>{source.entry_count != null ? `${source.entry_count} 条` : "—"}</dd>
      <dt>查询状态</dt><dd>{sourceStatusLabel(source.status, running)}</dd>
      <dt>覆盖</dt><dd>{sourceCoverageText(source)}</dd>
      <dt>最近成功</dt><dd>{sourceLastSuccessText(source)}</dd>
      <dt>最近尝试</dt><dd>{sourceLastAttemptText(source)}</dd>
      <dt>下次到期</dt><dd>{sourceNextDueText(source)}</dd>
    </dl>
    {task?.running && <div className="source-progress-detail">
      <ProgressBar task={task} />
      <p className="hint">{taskProgressText(task)}</p>
    </div>}
    <div className="source-actions">
      <Toggle checked={source.enabled} disabled={busy} label={`启用 ${source.name}`} onChange={onToggle} />
      <button type="button" disabled={!!busy || running} onClick={onUpdate}>{running ? "更新中…" : "手动更新"}</button>
      {running && <button type="button" className="cancel" onClick={onCancel} disabled={task?.cancelled}>{task?.cancelled ? "正在取消…" : "取消"}</button>}
      <label className="field-label">自动更新
        <select value={source.interval_hours ?? 0} disabled={!!busy} onChange={event => onSetInterval(Number(event.target.value))}>
          {SOURCE_INTERVAL_PRESETS.map(preset => <option key={preset.hours} value={preset.hours}>{preset.label}</option>)}
        </select>
      </label>
      {!!source.pages?.length && <button type="button" onClick={() => onOpenUrl(source.pages[0])}>打开来源网页 ↗</button>}
    </div>
  </div>;
}

function FallbackCard({ source, busy, onToggle }: { source: SourceSummary; busy: boolean; onToggle: (enabled: boolean) => void }) {
  return <div className="source-item">
    <div className="source-item-head">
      <strong>{source.name}</strong>
      <span className={`chip ${source.enabled ? "warn" : "idle"}`}>{source.enabled ? "补查来源" : "已关闭"}</span>
    </div>
    <p className="hint">{FALLBACK_SOURCE_HINT}</p>
    <dl className="source-meta">
      <dt>覆盖</dt><dd>{sourceCoverageText(source)}</dd>
      <dt>最近成功</dt><dd>{sourceLastSuccessText(source)}</dd>
      <dt>最近尝试</dt><dd>{sourceLastAttemptText(source)}</dd>
      <dt>下次到期</dt><dd>{sourceNextDueText(source)}</dd>
    </dl>
    {!source.enabled && <p className="hint">{gameMatchStatusLabel("no_match")}（普通来源无结果时的展示文案）。</p>}
    <div className="source-actions">
      <Toggle checked={source.enabled} disabled={busy} label={`启用 ${source.name}`} onChange={onToggle} />
    </div>
  </div>;
}

export function SourcePanel(props: SourcePanelProps) {
  const [rule, setRule] = useState("");
  if (!props.open) return null;
  const { busy, globalSources, gameSources, tasks } = props;
  const running = tasks.filter(task => task.running);
  const rematchTask = props.gameInstanceId
    ? tasks.find(task => task.kind === "rematch_game" && task.key === `instance:${props.gameInstanceId}`)
    : undefined;
  const groups: { role: "publisher" | "fallback_indexer"; title: string }[] = [
    { role: "publisher", title: "发行商补丁源" },
    { role: "fallback_indexer", title: "补丁索引（补查来源）" },
  ];
  return <div className="drawer-backdrop" role="presentation" onMouseDown={event => { if (event.target === event.currentTarget) props.onClose(); }}>
    <section className="drawer" role="dialog" aria-modal="true" aria-label="补丁来源管理">
      <header className="drawer-head">
        <div>
          <h2>补丁来源</h2>
          <p className="rev">全局设置对所有游戏生效{props.settingsRevision != null ? ` · 设置版本 ${props.settingsRevision}` : ""}</p>
        </div>
        <button type="button" onClick={props.onClose}>关闭</button>
      </header>
      <div className="drawer-body">
        {busy && <div role="status" className="status inline">{busy}</div>}
        {props.error && <div role="alert" className="error inline">{props.error}</div>}
        {props.notice && <div role="status" className="notice inline">{props.notice}</div>}

        {!!running.length && <section className="drawer-section">
          <h3>进行中的来源任务</h3>
          {running.map(task => <div className="source-item" key={task.task_id}>
            <div className="source-item-head">
              <strong>{taskKindLabel(task.kind)}{task.kind === "update_source" ? ` · ${task.key}` : task.kind === "rematch_game" ? " · 此游戏" : " · 全部游戏"}</strong>
              <span className="chip busy">{task.cancelled ? "正在取消" : "进行中"}</span>
            </div>
            <ProgressBar task={task} />
            <p className="hint">{taskProgressText(task)}</p>
            <div className="source-actions">
              <button type="button" className="cancel" onClick={() => props.onCancelTask(task.task_id)} disabled={task.cancelled}>{task.cancelled ? "正在取消…" : "取消任务"}</button>
            </div>
          </div>)}
        </section>}

        <section className="drawer-section">
          <h3>全局来源</h3>
          <div className="source-actions">
            <button type="button" disabled={!!busy} onClick={props.onReload}>刷新列表</button>
            <button type="button" disabled={!!busy || tasks.some(task => task.kind === "match_games" && task.running)} onClick={props.onMatchAllGames}>匹配全部游戏</button>
          </div>
          <p className="hint">自动更新仅在应用运行期间按来源间隔执行；失败时保留上次成功数据。</p>
          {props.loading && <p className="hint">正在读取来源列表…</p>}
          {!props.loading && globalSources && !globalSources.length && <p className="hint">尚未配置来源。</p>}
          {globalSources && groups.map(group => {
            const items = globalSources.filter(source => sourceRoleOf(source) === group.role);
            if (!items.length) return null;
            return <div className="source-group" key={group.role}>
              <h4>{group.title}</h4>
              {items.map(source => group.role === "publisher"
                ? <SourceCard key={source.id} source={source} busy={!!busy} task={taskForSource(tasks, source.id)}
                  onToggle={enabled => props.onToggleSource(source.id, enabled)}
                  onUpdate={() => props.onUpdateSource(source.id)}
                  onSetInterval={hours => props.onSetInterval(source.id, hours)}
                  onCancel={() => { const task = taskForSource(tasks, source.id); if (task) props.onCancelTask(task.task_id); }}
                  onOpenUrl={props.onOpenUrl} />
                : <FallbackCard key={source.id} source={source} busy={!!busy}
                  onToggle={enabled => props.onToggleSource(source.id, enabled)} />)}
            </div>;
          })}
        </section>

        <section className="drawer-section">
          <h3>此游戏的订阅与匹配</h3>
          {props.gameName
            ? <>
              <p className="hint">选择用于“{props.gameName}”的订阅源。关闭后不再从该源匹配补丁；选择按游戏编号保存。</p>
              {gameSources?.length
                ? gameSources.map(source => <div className="source-item" key={source.id}>
                  <div className="source-item-head">
                    <strong>{source.name}</strong>
                    <span className={`chip ${source.matched ? "ok" : "idle"}`}>{source.matched ? "已找到补丁" : "暂未匹配"}</span>
                  </div>
                  <dl className="source-meta">
                    <dt>查询状态</dt><dd>{sourceStatusLabel(source.status)}</dd>
                  </dl>
                  <div className="source-actions">
                    <Toggle checked={source.subscribed} disabled={!!busy} label={`${source.name} 用于此游戏`}
                      onChange={subscribed => props.onToggleGameSource(source.id, subscribed)} />
                  </div>
                  {!source.enabled && <p className="warning">此源已在全局设置中停用，当前不会参与匹配。</p>}
                </div>)
                : <p className="hint">尚未配置订阅源，请先在上方启用全局来源。</p>}
              <div className="source-actions">
                <label className="field-label">偏好的补丁语言
                  <select value={props.preferredLanguage ?? ""} onChange={event => props.onLanguageChange(event.target.value || null)} disabled={!!busy}>
                    <option value="">未指定</option>
                    <option value="zh-CN">简体中文</option>
                    <option value="zh-TW">繁体中文</option>
                    <option value="en">英文</option>
                    <option value="ja">日文</option>
                  </select>
                </label>
                <button type="button" disabled={!!busy || rematchTask?.running} onClick={props.onRematchGame}>
                  {rematchTask?.running ? "正在重新匹配…" : "重新匹配此游戏"}
                </button>
              </div>
              <p className="hint">重新匹配只使用已缓存的普通来源索引，不会刷新来源；缺索引时请先手动更新来源。</p>
              {rematchTask?.running && <div className="source-progress-detail">
                <ProgressBar task={rematchTask} />
                <p className="hint">{taskProgressText(rematchTask)}</p>
              </div>}
            </>
            : <p className="hint">选择一款游戏后，可在此调整它单独使用的订阅源。</p>}
        </section>

        <section className="drawer-section">
          <h3>添加来源（高级）</h3>
          <p className="hint">粘贴受支持的声明式规则 JSON；规则不允许嵌入代码，解析适配器由核心内置提供。</p>
          <textarea aria-label="来源规则 JSON" placeholder='例如 {"id":"custom","name":"我的来源","adapter":"table","enabled":true,"pages":["https://example.com/patch"],"page_hosts":["example.com"]}' value={rule} onChange={event => setRule(event.target.value)} />
          <div className="source-actions">
            <button type="button" disabled={!!busy || !rule.trim()} onClick={() => props.onAddSource(rule)}>添加来源</button>
          </div>
        </section>
      </div>
    </section>
  </div>;
}
