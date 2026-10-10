import { useState } from "react";
import type { GameSourceSubscription } from "@steam-patch/ui";
import { chipLabel, formatTime, sourceRoleOf, statusLabel, statusTone, type SourceSummary } from "./sources";

export type SourcePanelProps = {
  open: boolean;
  onClose: () => void;
  globalSources: SourceSummary[] | null;
  gameSources: GameSourceSubscription[] | null;
  gameName: string | null;
  settingsRevision: number | null;
  loading: boolean;
  busy: string | null;
  error: string | null;
  notice: string | null;
  scan: { running: boolean; cancelled: boolean; completed: number; total: number } | null;
  onReload: () => void;
  onRefreshSources: (force: boolean) => void;
  onToggleSource: (sourceId: string, enabled: boolean) => void;
  onToggleGameSource: (sourceId: string, subscribed: boolean) => void;
  onAddSource: (rule: string) => void;
  onCancelScan: () => void;
  onOpenUrl: (url: string) => void;
};

function Toggle({ checked, disabled, label, onChange }: { checked: boolean; disabled?: boolean; label: string; onChange: (value: boolean) => void }) {
  return <button type="button" role="switch" aria-checked={checked} aria-label={label} className={`toggle${checked ? " on" : ""}`} disabled={disabled} onClick={() => onChange(!checked)}>
    <span className="dot" aria-hidden="true" />{checked ? "已启用" : "已停用"}
  </button>;
}

function SourceCard({ source, busy, onToggle, onOpenUrl }: { source: SourceSummary; busy: boolean; onToggle: (enabled: boolean) => void; onOpenUrl: (url: string) => void }) {
  const tone = statusTone(source.status);
  return <div className="source-item">
    <div className="source-item-head">
      <strong>{source.name}</strong>
      <span className={`chip ${tone}`}>{chipLabel(source.status)}</span>
    </div>
    <dl className="source-meta">
      <dt>站点</dt><dd>{source.pages?.length ? source.pages.join(" · ") : "—"}</dd>
      <dt>查询结果</dt><dd>{source.entry_count != null ? `${source.entry_count} 条` : "—"}</dd>
      <dt>查询状态</dt><dd>{statusLabel(source.status)}</dd>
      <dt>缓存时间</dt><dd>{formatTime(source.cached_at)}</dd>
      <dt>最近查询</dt><dd>{source.checked_at ? formatTime(source.checked_at) : "尚未查询"}</dd>
    </dl>
    <div className="source-actions">
      <Toggle checked={source.enabled} disabled={busy} label={`启用 ${source.name}`} onChange={onToggle} />
      {!!source.pages?.length && <button type="button" onClick={() => onOpenUrl(source.pages[0])}>打开来源网页 ↗</button>}
    </div>
  </div>;
}

export function SourcePanel(props: SourcePanelProps) {
  const [rule, setRule] = useState("");
  if (!props.open) return null;
  const { busy, globalSources, gameSources } = props;
  const groups: { role: "publisher" | "fallback_indexer"; title: string }[] = [
    { role: "publisher", title: "发行商补丁源" },
    { role: "fallback_indexer", title: "补丁索引" },
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

        {props.scan?.running && <section className="drawer-section">
          <h3>查询进度</h3>
          <div className="progress-bar"><div className="progress-fill" style={{ width: props.scan.total ? `${Math.min(100, props.scan.completed / props.scan.total * 100)}%` : "35%" }} /></div>
          <p className="hint">{props.scan.total ? `已查询 ${props.scan.completed} / ${props.scan.total} 款游戏` : "正在准备查询已安装游戏"}</p>
          <button type="button" className="cancel" onClick={props.onCancelScan} disabled={props.scan.cancelled}>{props.scan.cancelled ? "正在取消…" : "取消查找"}</button>
        </section>}
        {props.scan && !props.scan.running && props.scan.cancelled && <div role="status" className="status inline">已取消查找，保留已取得的结果；可稍后重新查找。</div>}

        <section className="drawer-section">
          <h3>全局来源</h3>
          <div className="source-actions">
            <button type="button" disabled={!!busy} onClick={props.onReload}>刷新列表</button>
            <button type="button" disabled={!!busy} onClick={() => props.onRefreshSources(false)}>刷新来源索引</button>
            <button type="button" disabled={!!busy} onClick={() => props.onRefreshSources(true)}>强制刷新索引</button>
          </div>
          <p className="hint">刷新会按需请求各来源网站（受缓存与限速约束），可能需要数十秒；失败时保留上次成功数据。</p>
          {props.loading && <p className="hint">正在读取来源列表…</p>}
          {!props.loading && globalSources && !globalSources.length && <p className="hint">尚未配置来源。</p>}
          {globalSources && groups.map(group => {
            const items = globalSources.filter(source => sourceRoleOf(source) === group.role);
            if (!items.length) return null;
            return <div className="source-group" key={group.role}>
              <h4>{group.title}</h4>
              {items.map(source => <SourceCard key={source.id} source={source} busy={!!busy}
                onToggle={enabled => props.onToggleSource(source.id, enabled)} onOpenUrl={props.onOpenUrl} />)}
            </div>;
          })}
        </section>

        <section className="drawer-section">
          <h3>此游戏的订阅</h3>
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
                    <dt>查询状态</dt><dd>{statusLabel(source.status)}</dd>
                  </dl>
                  <div className="source-actions">
                    <Toggle checked={source.subscribed} disabled={!!busy} label={`${source.name} 用于此游戏`}
                      onChange={subscribed => props.onToggleGameSource(source.id, subscribed)} />
                  </div>
                  {!source.enabled && <p className="warning">此源已在全局设置中停用，当前不会参与匹配。</p>}
                </div>)
                : <p className="hint">尚未配置订阅源，请先在上方启用全局来源。</p>}
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
