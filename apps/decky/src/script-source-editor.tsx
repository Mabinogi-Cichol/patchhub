import { ButtonItem, Focusable, PanelSection, PanelSectionRow, TextField } from "@decky/ui";
import { useRef, useState } from "react";
import type { ScriptSourceDocument } from "../../../packages/contracts/generated/protocol";
import { scriptCodeExample, scriptRuleExample } from "./script-source-examples";

function CodeInput({ label, value, disabled, onChange }: {
  label: string; value: string; disabled: boolean; onChange: (value: string) => void;
}) {
  const input = useRef<HTMLTextAreaElement>(null);
  return <Focusable onActivate={() => input.current?.focus()}>
    <label style={{ display: "block", marginBlock: 12 }}>{label}
      <textarea ref={input} aria-label={label} rows={9} value={value} disabled={disabled}
        spellCheck={false} onChange={e => onChange(e.target.value)}
        style={{ display: "block", boxSizing: "border-box", width: "100%", marginTop: 8, padding: 12,
          background: "#1b2838", color: "#fff", border: "1px solid #8b929a", borderRadius: 4,
          fontFamily: "monospace", fontSize: 16, resize: "vertical" }} />
    </label>
  </Focusable>;
}

export function ScriptSourceEditor({ document, onSave, onClose }: {
  document?: ScriptSourceDocument;
  onSave: (rule: string, apiVersion: number, code: string, replace: boolean) => Promise<void>;
  onClose: () => void;
}) {
  const [rule, setRule] = useState(document ? JSON.stringify(document.rule, null, 2) : "");
  const [code, setCode] = useState(document?.code ?? "");
  const [api, setApi] = useState(String(document?.api_version ?? 1));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [example, setExample] = useState(false);
  const [note, setNote] = useState("");
  const target = document?.rule.id;
  async function save() {
    setSaving(true); setError("");
    try {
      const parsed = JSON.parse(rule);
      if (!parsed || Array.isArray(parsed) || typeof parsed !== "object") throw Error("规则必须是 JSON 对象");
      if (target && parsed.id !== target) throw Error(`替换时来源 ID 必须为 ${target}`);
      if (!/^\d+$/.test(api)) throw Error("API 版本必须是整数");
      await onSave(rule, Number(api), code, !!target);
      onClose();
    } catch (failure) { setError(String(failure)); }
    finally { setSaving(false); }
  }
  async function copy(text: string) {
    try { await navigator.clipboard.writeText(text); setNote("示例已复制，请修改后导入"); }
    catch (failure) { setError(`复制失败：${String(failure)}`); }
  }
  return <PanelSection title={target ? `编辑脚本：${target}` : "导入脚本来源"}>
    <PanelSectionRow><p>仅导入你信任的 Lua 脚本。网络请求限于规则声明的域名。保存仅检查语法，不执行脚本；保存后需更新来源或重新匹配游戏。</p></PanelSectionRow>
    {document && <PanelSectionRow><p style={{ overflowWrap: "anywhere" }}>当前 API {document.api_version} · SHA-256 {document.sha256}<br />替换同 ID 来源，保留名称覆盖、启用、订阅及更新间隔。</p></PanelSectionRow>}
    <PanelSectionRow><p>按 A 聚焦文本框；可使用 Steam + X 输入或粘贴。</p></PanelSectionRow>
    <PanelSectionRow><CodeInput label="脚本规则 JSON" value={rule} disabled={saving} onChange={setRule} /></PanelSectionRow>
    <PanelSectionRow><TextField label="Lua API 版本（当前支持 1）" value={api} disabled={saving} onChange={e => setApi(e.target.value)} /></PanelSectionRow>
    <PanelSectionRow><CodeInput label="Lua 源码" value={code} disabled={saving} onChange={setCode} /></PanelSectionRow>
    {error && <PanelSectionRow><p role="alert" className="patch-warning" style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{error}</p></PanelSectionRow>}
    <PanelSectionRow><ButtonItem layout="below" disabled={saving} onClick={save}>{saving ? "正在保存…" : target ? `保存并替换 ${target}` : "保存并导入"}</ButtonItem></PanelSectionRow>
    <PanelSectionRow><ButtonItem layout="below" disabled={saving} onClick={onClose}>取消</ButtonItem></PanelSectionRow>
    <PanelSectionRow><ButtonItem layout="below" aria-expanded={example} onClick={() => setExample(!example)}>{example ? "收起脚本示例" : "查看脚本示例"}</ButtonItem></PanelSectionRow>
    {example && <>
      <PanelSectionRow><p>示例只用于展示和复制，不会更改当前输入。请替换 ID、名称、域名及选择器。</p></PanelSectionRow>
      <PanelSectionRow><pre style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{scriptRuleExample}</pre></PanelSectionRow>
      <PanelSectionRow><ButtonItem layout="below" onClick={() => void copy(scriptRuleExample)}>复制规则示例</ButtonItem></PanelSectionRow>
      <PanelSectionRow><pre style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{scriptCodeExample}</pre></PanelSectionRow>
      <PanelSectionRow><ButtonItem layout="below" onClick={() => void copy(scriptCodeExample)}>复制源码示例</ButtonItem></PanelSectionRow>
      {note && <PanelSectionRow><p role="status">{note}</p></PanelSectionRow>}
    </>}
  </PanelSection>;
}
