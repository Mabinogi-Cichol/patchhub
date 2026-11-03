import { callable, toaster } from '@decky/api';
import { DialogButton, Field, PanelSection, PanelSectionRow, Navigation } from '@decky/ui';
import { useEffect, useRef, useState } from 'react';
import { settingsRoutes } from './route';
import { updateProgressLabel, updateExecuting, type UpdateExecution } from './app-update-view';

type Update = { state: string; current?: string; tag?: string; notes?: string; published_at?: string; needs_recovery?: boolean;
  error?: string; skipped?: boolean; execution?: UpdateExecution | null };
const status = callable<[], Update>('get_app_update_status');
const check = callable<[], Update>('check_app_update');
const start = callable<[string, number], Update>('start_app_update');
const restart = callable<[number], Update>('restart_app_update_client');
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
  let timer: ReturnType<typeof setTimeout>;
  const inspect = async (startup = false) => {
    try {
      // Startup always checks now; the backend still respects GitHub's
      // retry_at and joins any check already in progress.
      if (startup) await check();
      if (!active) return;
      const value = await claimNotice();
      if (!active || !value) return;
      toaster.toast({ title: `发现新版 ${value.tag}`, body: 'PatchHub · 点击查看更新', onClick: () => {
        Navigation.Navigate(settingsRoutes.cache); Navigation.CloseSideMenus();
      } });
    } catch (error) { console.warn('app_update_notice_failed', error); }
    finally { if (active) timer = setTimeout(() => void inspect(), 60000); }
  };
  void inspect(true);
  return () => { active = false; clearTimeout(timer); };
}

export function AppUpdatePanel() {
  const { value, setValue, error: statusError } = useUpdateStatus({ state: 'idle' });
  const [busy, setBusy] = useState(false);
  const [starting, setStarting] = useState(false);
  const inFlight = useRef(false);
  const work = async (action: () => Promise<Update>, installing = false) => {
    if (inFlight.current) return;
    inFlight.current = true; setBusy(true); setStarting(installing);
    try {
      const next = await action(); setValue(next);
      if (next.error) throw new Error(next.error);
      if (next.state === 'up_to_date' && !installing) toaster.toast({ title: '应用更新', body: '当前已是最新正式版。' });
    } catch (e) { toaster.toast({ title: '应用更新', body: String(e) }); }
    finally { inFlight.current = false; setBusy(false); setStarting(false); }
  };
  const execution = value.execution;
  const executing = starting || updateExecuting(execution);
  // A new Steam JS context means the previous request must not drive this UI.
  const restarting = execution?.state === 'completed' && execution.restart === 'requested'
    && execution.restart_context === performance.timeOrigin;
  const restartFailed = execution?.state === 'completed' && execution.restart === 'failed';
  const label = executing ? updateProgressLabel(starting ? { state: 'preparing', percent: 0 } : execution!)
    : restarting ? '100% · 正在重启客户端'
    : restartFailed ? '重启客户端'
    : value.needs_recovery ? '恢复更新'
    : busy ? '检查中…'
    : execution?.state === 'failed' ? '更新失败 · 重试'
    : statusError ? '读取失败 · 重试'
    : value.error ? '检查失败 · 重试'
    : value.state === 'available' ? `更新至 ${value.tag}` : '检查更新';
  const detail = () => {
    const error = restartFailed ? `更新已安装，客户端重启失败：${execution?.restart_error}`
      : execution?.state === 'failed' ? `更新未完成：${execution.error}` : statusError;
    if (error) toaster.toast({ title: '应用更新', body: error });
  };
  const act = () => {
    if (executing || restarting || busy) return;
    detail();
    if (restartFailed) void work(() => restart(performance.timeOrigin), true);
    else if (value.needs_recovery) void work(recover, true);
    else if (value.state === 'available') void work(() => start(value.tag!, performance.timeOrigin), true);
    else void work(check);
  };
  return <PanelSection title="应用更新"><PanelSectionRow>
    <Field label={`当前版本 ${value.current ?? '…'}`} childrenLayout="inline" spacingBetweenLabelAndChild="none">
      <DialogButton style={{ minWidth: 240, minHeight: 44, width: 280, maxWidth: '100%', whiteSpace: 'nowrap' }}
        aria-label={executing || restarting ? label : `${label}，更新完成后自动重启 Steam 客户端`}
        aria-disabled={busy || executing || restarting} onClick={act}>
        <span role="status" aria-live="polite">{label}</span>
      </DialogButton>
    </Field>
  </PanelSectionRow></PanelSection>;
}
