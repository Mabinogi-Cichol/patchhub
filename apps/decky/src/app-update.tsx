import { callable, toaster } from '@decky/api';
import { ButtonItem, Field, PanelSection, PanelSectionRow, Navigation, showModal } from '@decky/ui';
import { useEffect, useState } from 'react';
import { settingsRoutes } from './route';
import { AppUpdateDialogView, AppUpdateProgress, updateExecuting, type UpdateExecution } from './app-update-view';

type Update = { state: string; current?: string; tag?: string; notes?: string; published_at?: string; needs_recovery?: boolean;
  error?: string; skipped?: boolean; execution?: UpdateExecution | null };
const status = callable<[], Update>('get_app_update_status');
const check = callable<[], Update>('check_app_update');
const skip = callable<[string], Update>('skip_app_update');
const start = callable<[string], Update>('start_app_update');
const cancel = callable<[], Update>('cancel_app_update');
const recover = callable<[], Update>('recover_app_update');
const claimNotice = callable<[], Update | null>('claim_app_update_notice');

function useUpdateStatus(initial: Update) {
  const [value, setValue] = useState(initial);
  const [error, setError] = useState<string>();
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      try { const next = await status(); if (active) { setValue(next); setError(undefined); } }
      catch (e) { if (active) setError(String(e)); }
      finally { if (active) timer = setTimeout(() => void refresh(), 1000); }
    };
    void refresh();
    return () => { active = false; clearTimeout(timer); };
  }, []);
  return { value, setValue, error };
}

let progressModal: ReturnType<typeof showModal> | undefined;
function closeProgress() { progressModal?.Close(); progressModal = undefined; }
function openProgress(initial: Update) {
  closeProgress();
  progressModal = showModal(<AppUpdateDialog initial={initial} />);
}
function AppUpdateDialog({ initial }: { initial: Update }) {
  const { value, setValue, error } = useUpdateStatus(initial);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string>();
  const cancelDownload = async () => {
    setBusy(true); setActionError(undefined);
    try { setValue(await cancel()); } catch (e) { setActionError(String(e)); } finally { setBusy(false); }
  };
  return <AppUpdateDialogView execution={value.execution} error={error} actionError={actionError} busy={busy}
    onCancelDownload={() => void cancelDownload()} onClose={closeProgress} />;
}

export async function waitForAppUpdate() {
  while (true) {
    const value = await status();
    if (!['preparing', 'applying'].includes(value.execution?.state ?? '')) {
      if (value.needs_recovery) throw new Error('应用更新尚需恢复，请在全局设置中恢复更新');
      return;
    }
    await new Promise(resolve => setTimeout(resolve, 500));
  }
}

export function startAppUpdateNotifications() {
  let active = true;
  void status().then(value => { if (active && updateExecuting(value.execution)) openProgress(value); })
    .catch(error => console.warn('app_update_resume_failed', error));
  const inspect = () => { void claimNotice().then(value => {
    if (!active || !value) return;
    toaster.toast({ title: `发现新版 ${value.tag}`, body: 'PatchHub · 点击查看更新', onClick: () => {
      Navigation.Navigate(settingsRoutes.cache); Navigation.CloseSideMenus();
    } });
  }).catch(error => console.warn('app_update_notice_failed', error)); };
  inspect();
  const timer = setInterval(inspect, 60000);
  return () => { active = false; clearInterval(timer); closeProgress(); };
}

export function AppUpdatePanel() {
  const { value, setValue, error: statusError } = useUpdateStatus({ state: 'idle' });
  const [manual, setManual] = useState(false);
  const [dismissed, setDismissed] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const work = async (action: () => Promise<Update>) => {
    setBusy(true); setError(undefined);
    try { setValue(await action()); } catch (e) { setError(String(e)); } finally { setBusy(false); }
  };
  const visible = manual || (!value.skipped && dismissed !== value.tag && value.state === 'available');
  const executing = updateExecuting(value.execution);
  const begin = (action: () => Promise<Update>) => work(async () => { const next = await action(); openProgress(next); return next; });
  return <PanelSection title="应用更新">
    <PanelSectionRow><Field label="当前版本" childrenLayout="inline" spacingBetweenLabelAndChild="none"><span className="patch-kv-value">{value.current ?? '…'}</span></Field></PanelSectionRow>
    <PanelSectionRow><ButtonItem layout="below" bottomSeparator="none" disabled={busy || executing} onClick={() => { setManual(true); void work(check); }}>检查更新</ButtonItem></PanelSectionRow>
    {error && <PanelSectionRow><p className="patch-warning">{error}</p></PanelSectionRow>}
    {statusError && <PanelSectionRow><p>{executing ? '正在等待插件重载或重新连接。' : '读取更新状态失败。'} {statusError}</p></PanelSectionRow>}
    {executing && <PanelSectionRow><AppUpdateProgress execution={value.execution!} /></PanelSectionRow>}
    {executing && <PanelSectionRow><ButtonItem layout="below" bottomSeparator="none" onClick={() => openProgress(value)}>查看更新进度</ButtonItem></PanelSectionRow>}
    {value.execution?.state === 'preparing' && <PanelSectionRow><ButtonItem layout="below" bottomSeparator="none" disabled={busy} onClick={() => void work(cancel)}>取消下载</ButtonItem></PanelSectionRow>}
    {value.execution?.state === 'failed' && <PanelSectionRow><p className="patch-warning">更新未完成：{value.execution.error}</p></PanelSectionRow>}
    {value.needs_recovery && !executing && <PanelSectionRow><ButtonItem layout="below" bottomSeparator="none" disabled={busy} onClick={() => void begin(recover)}>恢复未完成的更新</ButtonItem></PanelSectionRow>}
    {value.execution?.state === 'completed' && <PanelSectionRow><p>更新已完成。请退出当前插件页面，再从 Decky 重新打开 PatchHub，以加载新版界面。</p></PanelSectionRow>}
    {visible && <>
      {value.state === 'up_to_date' && <PanelSectionRow><p>当前已是最新正式版。</p></PanelSectionRow>}
      {value.error && <PanelSectionRow><p className="patch-warning">{value.error}</p></PanelSectionRow>}
      {value.state === 'available' && <>
        <PanelSectionRow><Field label="可用版本" childrenLayout="inline" spacingBetweenLabelAndChild="none"><span className="patch-kv-value">{value.tag}</span></Field></PanelSectionRow>
        {value.published_at && <PanelSectionRow><Field label="发布时间" childrenLayout="inline" spacingBetweenLabelAndChild="none"><span className="patch-kv-value">{new Date(value.published_at).toLocaleDateString('zh-CN')}</span></Field></PanelSectionRow>}
        {value.notes && <PanelSectionRow><p style={{ whiteSpace: 'pre-wrap' }}>{value.notes}</p></PanelSectionRow>}
        <PanelSectionRow><ButtonItem layout="below" bottomSeparator="none" disabled={busy || executing} onClick={() => void begin(() => start(value.tag!))}>更新插件</ButtonItem></PanelSectionRow>
        <PanelSectionRow><ButtonItem layout="below" bottomSeparator="none" disabled={executing} onClick={() => { setDismissed(value.tag); setManual(false); }}>稍后</ButtonItem></PanelSectionRow>
        <PanelSectionRow><ButtonItem layout="below" bottomSeparator="none" disabled={busy || executing} onClick={() => void work(async () => { const next = await skip(value.tag!); setManual(false); return next; })}>跳过此版本</ButtonItem></PanelSectionRow>
      </>}
    </>}
  </PanelSection>;
}
