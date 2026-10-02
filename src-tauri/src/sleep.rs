use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SleepStatus {
    pub enabled: bool,
    pub seconds_remaining: Option<u64>,
    pub supported: bool,
}

// Runtime only: neither an armed run nor its countdown survives a restart.
#[derive(Clone, Default)]
pub struct SleepPlan {
    enabled: bool,
    pending: HashSet<String>,
    deadline: Option<Instant>,
}
impl SleepPlan {
    pub fn arm(&mut self, queue: &[String]) -> Result<(), String> {
        if queue.is_empty() {
            return Err("Add a campaign to the queue first.".into());
        }
        self.enabled = true;
        self.pending = queue.iter().cloned().collect();
        self.deadline = None;
        Ok(())
    }
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
    pub fn queue_changed(&mut self, before: &[String], after: &[String]) {
        if before.iter().collect::<HashSet<_>>() != after.iter().collect::<HashSet<_>>() {
            self.cancel();
        }
    }
    pub fn confirmed(&mut self, id: &str) {
        self.pending.remove(id);
    }
    pub fn finish(&mut self, now: Instant) -> bool {
        if !self.enabled || !self.pending.is_empty() || self.deadline.is_some() {
            return false;
        }
        self.deadline = Some(now + Duration::from_secs(60));
        true
    }
    pub fn status(&self, now: Instant) -> SleepStatus {
        SleepStatus {
            enabled: self.enabled,
            seconds_remaining: self
                .deadline
                .map(|d| d.saturating_duration_since(now).as_millis().div_ceil(1000) as u64),
            supported: cfg!(windows),
        }
    }
    pub fn take_due(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|d| now >= d) {
            // A suspended or stalled app must not issue a stale sleep request on resume.
            let on_time = self
                .deadline
                .is_some_and(|d| now.duration_since(d) < Duration::from_secs(5));
            self.cancel();
            on_time
        } else {
            false
        }
    }
}

pub fn take_request(s: &mut crate::model::Snapshot, now: Instant) -> bool {
    if s.running
        || !s.queue.is_empty()
        || s.account.is_none()
        || s.needs_reconnect
        || s.error.is_some()
    {
        s.sleep_plan.cancel();
        return false;
    }
    s.sleep_plan.take_due(now)
}

#[cfg(windows)]
pub fn suspend() -> Result<(), String> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_SUCCESS, HANDLE},
        Security::{
            AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, SE_SHUTDOWN_NAME,
            TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
        },
        System::{
            Power::SetSuspendState,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    };
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        ) == 0
        {
            return Err(
                "Windows could not grant permission to sleep. The PC will stay awake.".into(),
            );
        }
        let result = (|| {
            let mut requested = TOKEN_PRIVILEGES::default();
            requested.PrivilegeCount = 1;
            if LookupPrivilegeValueW(null(), SE_SHUTDOWN_NAME, &mut requested.Privileges[0].Luid)
                == 0
            {
                return Err("Windows sleep permission is unavailable.".into());
            }
            requested.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
            let mut previous = TOKEN_PRIVILEGES::default();
            let mut length = 0;
            let adjusted = AdjustTokenPrivileges(
                token,
                0,
                &requested,
                std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                &mut previous,
                &mut length,
            );
            let error = GetLastError();
            if adjusted == 0 || error != ERROR_SUCCESS {
                return Err("Windows did not allow sleep. The PC will stay awake.".into());
            }
            // Request sleep, keep wake events enabled, and restore the token afterwards.
            // https://learn.microsoft.com/windows/win32/api/powrprof/nf-powrprof-setsuspendstate
            let slept = SetSuspendState(false, false, false);
            AdjustTokenPrivileges(token, 0, &previous, 0, null_mut(), null_mut());
            if slept {
                Ok(())
            } else {
                Err("Windows could not put the PC to sleep. Check your power settings.".into())
            }
        })();
        CloseHandle(token);
        result
    }
}
#[cfg(not(windows))]
pub fn suspend() -> Result<(), String> {
    Err("Sleep when finished is currently available on Windows.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan() -> SleepPlan {
        let mut p = SleepPlan::default();
        p.arm(&["a".into(), "b".into()]).unwrap();
        p
    }
    #[test]
    fn requires_every_confirmation_and_waits_a_full_minute() {
        let now = Instant::now();
        let mut p = plan();
        assert!(!p.finish(now)); // Offline, blocked, or omitted campaigns remain pending.
        p.confirmed("a");
        p.confirmed("unknown");
        assert!(!p.finish(now));
        p.confirmed("b");
        assert!(p.finish(now));
        assert!(!p.finish(now + Duration::from_secs(20))); // Cannot extend the countdown.
        assert_eq!(p.status(now).seconds_remaining, Some(60));
        assert!(!p.take_due(now + Duration::from_millis(59999)));
        assert!(p.take_due(now + Duration::from_secs(60)));
        assert!(!p.take_due(now + Duration::from_secs(61)));
        assert!(!p.status(now).enabled);
    }
    #[test]
    fn edits_cancel_but_reordering_preserves_the_run() {
        let mut p = plan();
        p.queue_changed(&["a".into(), "b".into()], &["b".into(), "a".into()]);
        assert!(p.status(Instant::now()).enabled);
        p.queue_changed(&["a".into(), "b".into()], &["a".into()]);
        assert!(!p.status(Instant::now()).enabled);
        let mut p = plan();
        p.queue_changed(&["a".into(), "b".into()], &[]);
        assert!(!p.finish(Instant::now()));
        let mut p = plan();
        p.queue_changed(
            &["a".into(), "b".into()],
            &["a".into(), "b".into(), "c".into()],
        );
        assert!(!p.status(Instant::now()).enabled);
    }
    #[test]
    fn cancellation_and_rearming_discard_old_deadlines() {
        let now = Instant::now();
        let mut p = plan();
        p.confirmed("a");
        p.confirmed("b");
        p.finish(now);
        p.cancel();
        assert!(!p.take_due(now + Duration::from_secs(60)));
        p.arm(&["c".into()]).unwrap();
        assert!(!p.take_due(now + Duration::from_secs(120)));
        assert!(!p.finish(now));
        assert!(SleepPlan::default().arm(&[]).is_err());
    }
    #[test]
    fn late_countdown_after_resume_cannot_sleep_again() {
        let now = Instant::now();
        let mut p = plan();
        p.confirmed("a");
        p.confirmed("b");
        p.finish(now);
        assert!(!p.take_due(now + Duration::from_secs(3600)));
        assert!(!p.status(now).enabled);
    }
    #[test]
    fn final_check_cancels_for_new_work_errors_or_lost_login() {
        use crate::model::{Account, Snapshot};
        let now = Instant::now();
        let mut s = Snapshot::default();
        s.account = Some(Account {
            id: "a".into(),
            login: "user".into(),
        });
        s.sleep_plan = plan();
        s.sleep_plan.confirmed("a");
        s.sleep_plan.confirmed("b");
        s.sleep_plan.finish(now);
        let due = now + Duration::from_secs(60);
        for change in 0..5 {
            let mut changed = s.clone();
            match change {
                0 => changed.running = true,
                1 => changed.queue.push("new".into()),
                2 => changed.account = None,
                3 => changed.needs_reconnect = true,
                _ => changed.error = Some("save failed".into()),
            }
            assert!(!take_request(&mut changed, due));
            assert!(!changed.sleep_plan.status(due).enabled);
        }
        // Simulated power callback: final completion fires exactly once.
        let mut sleep_calls = 0;
        if take_request(&mut s, due) {
            sleep_calls += 1;
        }
        if take_request(&mut s, due) {
            sleep_calls += 1;
        }
        assert_eq!(sleep_calls, 1);
    }
    #[test]
    fn serialized_state_cannot_restore_an_armed_run() {
        let mut s = crate::model::Snapshot::default();
        s.sleep_plan = plan();
        s.sleep_after_queue = s.sleep_plan.status(Instant::now());
        let restored: crate::model::Snapshot =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(!restored.sleep_plan.status(Instant::now()).enabled);
    }
}
