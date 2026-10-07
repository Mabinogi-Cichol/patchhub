//! Job Object creation, limits, accounting and reclamation.

use core::ffi::c_void;
use core::mem::size_of;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    CreateJobObjectW, JobObjectBasicAccountingInformation, JobObjectCpuRateControlInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_CPU_RATE_CONTROL_INFORMATION, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_CPU_RATE_CONTROL_ENABLE,
    JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_JOB_TIME, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY,
};

use super::util::{close_handle, hresult_to_win32};
use crate::model::{IsolationError, JobSnapshot, SandboxLimits};

/// RAII wrapper around a Job Object handle. The handle is created non-inheritable and
/// `KILL_ON_JOB_CLOSE` is always set.
pub struct Job {
    handle: HANDLE,
    closed: bool,
}

impl Job {
    pub fn create() -> Result<Self, IsolationError> {
        match unsafe { CreateJobObjectW(None, PCWSTR::null()) } {
            Ok(handle) => Ok(Job {
                handle,
                closed: false,
            }),
            Err(err) => Err(IsolationError::new(
                "job",
                Some(hresult_to_win32(&err)),
                format!("CreateJobObjectW failed: {err}"),
            )),
        }
    }

    pub fn handle(&self) -> HANDLE {
        self.handle
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Configure memory, process-count, user-time and CPU-rate limits.
    pub fn set_limits(&self, limits: &SandboxLimits) -> Result<(), IsolationError> {
        let mut ext = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        let mut flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        if let Some(bytes) = limits.process_memory_bytes {
            flags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            ext.ProcessMemoryLimit = bytes as usize;
        }
        if let Some(bytes) = limits.job_memory_bytes {
            flags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
            ext.JobMemoryLimit = bytes as usize;
        }
        if let Some(n) = limits.active_process_limit {
            flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            ext.BasicLimitInformation.ActiveProcessLimit = n;
        }
        if let Some(t) = limits.per_job_user_time_100ns {
            flags |= JOB_OBJECT_LIMIT_JOB_TIME;
            ext.BasicLimitInformation.PerJobUserTimeLimit = t;
        }
        ext.BasicLimitInformation.LimitFlags = flags;

        unsafe {
            SetInformationJobObject(
                self.handle,
                JobObjectExtendedLimitInformation,
                &ext as *const _ as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|err| {
            IsolationError::new(
                "job",
                Some(hresult_to_win32(&err)),
                format!("SetInformationJobObject(ExtendedLimit) failed: {err}"),
            )
        })?;

        if let Some(percent) = limits.cpu_rate_percent {
            let cpu = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
                ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE
                    | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
                Anonymous: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0 {
                    CpuRate: percent.clamp(1, 100) * 100,
                },
            };
            unsafe {
                SetInformationJobObject(
                    self.handle,
                    JobObjectCpuRateControlInformation,
                    &cpu as *const _ as *const c_void,
                    size_of::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>() as u32,
                )
            }
            .map_err(|err| {
                IsolationError::new(
                    "job",
                    Some(hresult_to_win32(&err)),
                    format!("SetInformationJobObject(CpuRateControl) failed: {err}"),
                )
            })?;
        }

        Ok(())
    }

    /// Read back the configured CPU rate control (for probe evidence).
    pub fn cpu_rate_control(&self) -> Result<(u32, u32), IsolationError> {
        let mut info = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                Some(self.handle),
                JobObjectCpuRateControlInformation,
                &mut info as *mut _ as *mut c_void,
                size_of::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>() as u32,
                None,
            )
        }
        .map_err(|err| {
            IsolationError::new(
                "job",
                Some(hresult_to_win32(&err)),
                format!("QueryInformationJobObject(CpuRateControl) failed: {err}"),
            )
        })?;
        let rate = unsafe { info.Anonymous.CpuRate };
        Ok((info.ControlFlags.0, rate))
    }

    /// Read accounting + peak memory into a snapshot.
    pub fn snapshot(&self) -> Result<JobSnapshot, IsolationError> {
        let mut acct = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                Some(self.handle),
                JobObjectBasicAccountingInformation,
                &mut acct as *mut _ as *mut c_void,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            )
        }
        .map_err(|err| {
            IsolationError::new(
                "job",
                Some(hresult_to_win32(&err)),
                format!("QueryInformationJobObject(accounting) failed: {err}"),
            )
        })?;

        let mut ext = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                Some(self.handle),
                JobObjectExtendedLimitInformation,
                &mut ext as *mut _ as *mut c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                None,
            )
        }
        .map_err(|err| {
            IsolationError::new(
                "job",
                Some(hresult_to_win32(&err)),
                format!("QueryInformationJobObject(extended) failed: {err}"),
            )
        })?;

        Ok(JobSnapshot {
            active_processes: acct.ActiveProcesses,
            total_processes: acct.TotalProcesses,
            total_user_time_100ns: acct.TotalUserTime.max(0) as u64,
            total_kernel_time_100ns: acct.TotalKernelTime.max(0) as u64,
            peak_process_memory_bytes: ext.PeakProcessMemoryUsed as u64,
            peak_job_memory_bytes: ext.PeakJobMemoryUsed as u64,
        })
    }

    /// Terminate every process in the job.
    pub fn terminate(&self, exit_code: u32) -> Result<(), IsolationError> {
        if self.closed {
            return Err(IsolationError::new(
                "job",
                None,
                "cannot terminate a closed job handle",
            ));
        }
        unsafe { TerminateJobObject(self.handle, exit_code) }.map_err(|err| {
            IsolationError::new(
                "job",
                Some(hresult_to_win32(&err)),
                format!("TerminateJobObject failed: {err}"),
            )
        })
    }

    /// Close the job handle early. With `KILL_ON_JOB_CLOSE` this terminates members.
    pub fn close(&mut self) {
        if !self.closed {
            unsafe { close_handle(self.handle) };
            self.closed = true;
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.close();
    }
}
