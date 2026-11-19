import { ButtonItem, Focusable, PanelSection, PanelSectionRow } from "@decky/ui";
import { useRef, useState } from "react";
import type { ScriptSourceDocument } from "../../../packages/contracts/generated/protocol";
import { scriptSourceError } from "./script-source-error";

function CodeInput({ label, value, disabled, placeholder, rows = 11, onChange }: {
  label: string; value: string; disabled: boolean; placeholder?: string; rows?: number; onChange: (value: string) => void;
}) {
  const input = useRef<HTMLTextAreaElement>(null);
  return <Focusable onActivate={() => input.current?.focus()}>
    <label style={{ display: "block", marginBlock: 12 }}>{label}
      <textarea ref={input} aria-label={label} rows={rows} value={value} disabled={disabled}
        placeholder={placeholder} spellCheck={false} onChange={e => onChange(e.target.value)}
        style={{ display: "block", boxSizing: "border-box", width: "100%", marginTop: 8, padding: 12,
          background: "#1b2838", color: "#fff", border: "1px solid #8b929a", borderRadius: 4,
          fontFamily: "monospace", fontSize: 16, resize: "vertical" }} />
    </label>
  </Focusable>;
}

function hostList(hosts?: string[]): string {
  return hosts && hosts.length ? hosts.join(" · ") : "无";
}

export function ScriptSourceEditor({ document, readOnly = false, onLoadPrompt, onSave, onClose }: {
  document?: ScriptSourceDocument;
  /** 内置来源随版本更新，只展示只读摘要与源码。 */
  readOnly?: boolean;
  onLoadPrompt?: () => Promise<string>;
  onSave: (code: string, sourceId: string | null, apiVersion: number) => Promise<void>;
  onClose: () => void;
}) {
  const original = document?.rule;
  const target = document?.source_id ?? original?.id ?? null;
  const isCreate = target === null && !readOnly;
  const displayName = document?.name ?? target ?? "";
  const roleLabel = original?.role === "fallback_indexer" ? "补查源" : "索引源";

  const [code, setCode] = useState(document?.code ?? "");
  const [saving, setSaving] = useState(false);
  const [loadingPrompt, setLoadingPrompt] = useState(false);
  const [error, setError] = useState("");
  const [note, setNote] = useState("");
  const disabled = saving || readOnly;

  async function copyPrompt() {
    if (!onLoadPrompt) return;
    setLoadingPrompt(true);
    setError("");
    setNote("");
    try {
      const prompt = await onLoadPrompt();
      await navigator.clipboard.writeText(prompt);
      setNote("AI 提示词已复制，请粘贴发送给你的 AI 工具。");
    } catch (failure) {
      setError(`复制 AI 提示词失败：${scriptSourceError(failure)}`);
    } finally {
      setLoadingPrompt(false);
    }
  }

  async function save() {
    setSaving(true);
    setError("");
    setNote("");
    try {
      await onSave(code, target, document?.api_version ?? 1);
      onClose();
    } catch (failure) {
      setError(scriptSourceError(failure));
    } finally {
      setSaving(false);
    }
  }

  async function copyCode() {
    try {
      await navigator.clipboard.writeText(code);
      setNote("Lua 源码已复制");
    } catch (failure) {
      setError(`复制失败：${String(failure)}`);
    }
  }

  return <PanelSection title={readOnly ? `查看脚本：${target}` : target ? `编辑脚本：${target}` : "导入脚本来源"}>
    {readOnly && <PanelSectionRow><p>内置来源随版本更新，只读查看。</p></PanelSectionRow>}
    {isCreate && <PanelSectionRow><p>仅导入你信任的 Lua 脚本。可复制 AI 提示词发给 AI 工具生成脚本后直接粘贴导入；按 A 聚焦文本框，可使用 Steam + X 输入或粘贴。</p></PanelSectionRow>}
    {!readOnly && !isCreate && <PanelSectionRow><p>仅导入你信任的 Lua 脚本。替换代码会保留当前来源 ID、名称及订阅设置；按 A 聚焦文本框。</p></PanelSectionRow>}
    {isCreate && onLoadPrompt && <PanelSectionRow>
      <ButtonItem layout="below" disabled={saving || loadingPrompt} onClick={copyPrompt}>
        {loadingPrompt ? "正在读取提示词…" : "复制 AI 提示词"}
      </ButtonItem>
    </PanelSectionRow>}
    {target && <PanelSectionRow>
      <div style={{ padding: 12, background: "rgba(255,255,255,0.04)", borderRadius: 4, fontSize: 14, lineHeight: 1.6 }}>
        <div>来源 ID：{target}</div>
        <div>显示名称：{displayName}</div>
        <div>来源类型：{roleLabel}</div>
        <div>允许访问域名：{hostList(original?.page_hosts)}</div>
        <div>允许下载域名：{hostList(original?.download_hosts)}</div>
      </div>
    </PanelSectionRow>}
    <PanelSectionRow>
      <CodeInput label="Lua 源码" value={code} disabled={disabled}
        placeholder={isCreate ? "粘贴包含 meta 表的完整 Lua 脚本…" : undefined} onChange={setCode} />
    </PanelSectionRow>
    {error && <PanelSectionRow><p role="alert" className="patch-warning" style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{error}</p></PanelSectionRow>}
    {readOnly && <PanelSectionRow><ButtonItem layout="below" onClick={copyCode}>复制 Lua 源码</ButtonItem></PanelSectionRow>}
    {note && <PanelSectionRow><p role="status">{note}</p></PanelSectionRow>}
    {!readOnly && <PanelSectionRow>
      <ButtonItem layout="below" disabled={saving || loadingPrompt} onClick={save}>
        {saving ? "正在保存…" : target ? `保存并替换 ${target}` : "保存并导入"}
      </ButtonItem>
    </PanelSectionRow>}
    <PanelSectionRow><ButtonItem layout="below" disabled={saving} onClick={onClose}>{readOnly ? "关闭" : "取消"}</ButtonItem></PanelSectionRow>
  </PanelSection>;
}
