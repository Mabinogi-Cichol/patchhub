import { ButtonItem, DropdownItem, Focusable, PanelSection, PanelSectionRow, TextField, ToggleField } from "@decky/ui";
import { useRef, useState } from "react";
import type { Columns, RawSourceRule, ScriptSourceDocument } from "../../../packages/contracts/generated/protocol";
import { scriptSourceError } from "./script-source-error";

function CodeInput({ label, value, disabled, rows = 9, onChange }: {
  label: string; value: string; disabled: boolean; rows?: number; onChange: (value: string) => void;
}) {
  const input = useRef<HTMLTextAreaElement>(null);
  return <Focusable onActivate={() => input.current?.focus()}>
    <label style={{ display: "block", marginBlock: 12 }}>{label}
      <textarea ref={input} aria-label={label} rows={rows} value={value} disabled={disabled}
        spellCheck={false} onChange={e => onChange(e.target.value)}
        style={{ display: "block", boxSizing: "border-box", width: "100%", marginTop: 8, padding: 12,
          background: "#1b2838", color: "#fff", border: "1px solid #8b929a", borderRadius: 4,
          fontFamily: "monospace", fontSize: 16, resize: "vertical" }} />
    </label>
  </Focusable>;
}

const columnFields = [
  ["title", "标题列"], ["download", "下载列"], ["date", "日期列"],
  ["language", "语言列"], ["appid", "Steam AppID 列"],
] as const;
const optionalStrings = [
  ["url", "起始 URL"], ["language", "语言"], ["table_id", "表格 ID"],
] as const;
const lists = [
  ["pages", "起始页面（必填，每行一项）"],
  ["page_hosts", "页面域名（必填，每行一项）"],
  ["download_hosts", "下载域名（每行一项）"],
] as const;

function integer(value: string, label: string): number {
  const parsed = Number(value);
  if (!value.trim() || !Number.isSafeInteger(parsed)) throw Error(`${label}必须是整数`);
  return parsed;
}

export function ScriptSourceEditor({ document, readOnly = false, onSave, onClose }: {
  document?: ScriptSourceDocument;
  /** 内置来源随版本更新，只展示规则与源码。 */
  readOnly?: boolean;
  onSave: (rule: string, apiVersion: number, code: string, replace: boolean) => Promise<void>;
  onClose: () => void;
}) {
  const original = document?.rule;
  const [rule, setRule] = useState<RawSourceRule>(() => original ?? {
    id: "", name: "", adapter: "lua", enabled: true, pages: [], page_hosts: [],
    root_layout: "game_root",
  });
  const [priority, setPriority] = useState(String(original?.priority ?? ""));
  const [listValues, setListValues] = useState(() => ({
    pages: original?.pages.join("\n") ?? "",
    page_hosts: original?.page_hosts.join("\n") ?? "",
    download_hosts: original?.download_hosts?.join("\n") ?? "",
  }));
  const [useColumns, setUseColumns] = useState(!!original?.columns);
  const [columns, setColumns] = useState(() => Object.fromEntries(
    columnFields.map(([key]) => [key, String(original?.columns?.[key] ?? "")]),
  ));
  const [code, setCode] = useState(document?.code ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [note, setNote] = useState("");
  const target = original?.id;
  const disabled = saving || readOnly;
  const discoveryOnly = !!rule.discovery_only || rule.role === "fallback_indexer";

  function update<K extends keyof RawSourceRule>(key: K, value: RawSourceRule[K]) {
    setRule(previous => ({ ...previous, [key]: value }));
  }

  async function save() {
    setSaving(true);
    setError("");
    try {
      const lines = (value: string) => value.split(/\r?\n/).map(line => line.trim()).filter(Boolean);
      const downloads = lines(listValues.download_hosts);
      let parsedColumns: Columns | undefined;
      if (useColumns) {
        parsedColumns = {
          title: integer(columns.title, "标题列"),
          download: integer(columns.download, "下载列"),
          date: columns.date.trim() ? integer(columns.date, "日期列") : null,
          language: columns.language.trim() ? integer(columns.language, "语言列") : null,
          appid: columns.appid.trim() ? integer(columns.appid, "Steam AppID 列") : null,
        };
      }
      const parsed: RawSourceRule = {
        ...rule,
        discovery_only: discoveryOnly,
        root_layout: discoveryOnly ? undefined : "game_root",
        priority: priority.trim() ? integer(priority, "优先级") : undefined,
        pages: lines(listValues.pages),
        page_hosts: lines(listValues.page_hosts),
        download_hosts: downloads.length || original?.download_hosts != null ? downloads : undefined,
        columns: parsedColumns,
      };
      await onSave(JSON.stringify(parsed), document?.api_version ?? 1, code, !!target);
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
    {!readOnly && <PanelSectionRow><p>仅导入你信任的 Lua 脚本。按 A 聚焦文本框，可使用 Steam + X 输入或粘贴。</p></PanelSectionRow>}
    <PanelSectionRow><TextField label="来源 ID（必填）" value={rule.id} disabled={disabled || !!target}
      onChange={e => update("id", e.target.value)} /></PanelSectionRow>
    <PanelSectionRow><TextField label="规则名称（必填）" value={rule.name} disabled={disabled}
      onChange={e => update("name", e.target.value)} /></PanelSectionRow>
    <PanelSectionRow><DropdownItem label="角色" layout="below" selectedOption={rule.role ?? "publisher"}
      disabled={disabled} rgOptions={[{ data: "publisher", label: "发行商" }, { data: "fallback_indexer", label: "补查来源" }]}
      onChange={option => update("role", option.data)} /></PanelSectionRow>
    <PanelSectionRow><ToggleField label="仅发现" checked={discoveryOnly}
      disabled={disabled || rule.role === "fallback_indexer"} onChange={value => update("discovery_only", value)} /></PanelSectionRow>
    <PanelSectionRow><TextField label="安装布局" disabled value={discoveryOnly ? "无" : "游戏根目录（game_root）"} /></PanelSectionRow>
    <PanelSectionRow><TextField label="优先级（默认 100）" value={priority} disabled={disabled}
      onChange={e => setPriority(e.target.value)} /></PanelSectionRow>
    <PanelSectionRow><ToggleField label="启用" checked={rule.enabled} disabled={disabled || !!target}
      onChange={value => update("enabled", value)} /></PanelSectionRow>
    {lists.map(([key, label]) => <PanelSectionRow key={key}>
      <CodeInput label={label} rows={3} value={listValues[key]} disabled={disabled}
        onChange={value => setListValues(previous => ({ ...previous, [key]: value }))} />
    </PanelSectionRow>)}
    {optionalStrings.map(([key, label]) => <PanelSectionRow key={key}>
      <ToggleField label={`设置${label}`} checked={rule[key] != null} disabled={disabled}
        onChange={checked => update(key, checked ? "" : undefined)} />
      {rule[key] != null && <TextField label={label} value={rule[key]} disabled={disabled}
        onChange={e => update(key, e.target.value)} />}
    </PanelSectionRow>)}
    <PanelSectionRow><ToggleField label="设置表格列（从 0 开始）" checked={useColumns} disabled={disabled}
      onChange={setUseColumns} /></PanelSectionRow>
    {useColumns && columnFields.map(([key, label]) => <PanelSectionRow key={key}>
      <TextField label={label} value={columns[key]} disabled={disabled}
        onChange={e => {
          const value = e.target.value;
          setColumns(previous => ({ ...previous, [key]: value }));
        }} />
    </PanelSectionRow>)}
    <PanelSectionRow><CodeInput label="Lua 源码" value={code} disabled={disabled} onChange={setCode} /></PanelSectionRow>
    {error && <PanelSectionRow><p role="alert" className="patch-warning" style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{error}</p></PanelSectionRow>}
    {readOnly && <PanelSectionRow><ButtonItem layout="below" onClick={() => void copyCode()}>复制 Lua 源码</ButtonItem></PanelSectionRow>}
    {note && <PanelSectionRow><p role="status">{note}</p></PanelSectionRow>}
    {!readOnly && <PanelSectionRow><ButtonItem layout="below" disabled={saving} onClick={save}>{saving ? "正在保存…" : target ? `保存并替换 ${target}` : "保存并导入"}</ButtonItem></PanelSectionRow>}
    <PanelSectionRow><ButtonItem layout="below" disabled={saving} onClick={onClose}>{readOnly ? "关闭" : "取消"}</ButtonItem></PanelSectionRow>
  </PanelSection>;
}
