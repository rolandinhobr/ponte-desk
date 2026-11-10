//! One authority (Mac) selects a stable target; Windows follows its decision.
use super::native::State;
use crate::preferences::Preferences;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub target: String,
    pub revision: u64,
    pub reason: String,
    pub waiting_release: bool,
    #[serde(default)]
    pub local_control: bool,
}
#[derive(Default)]
pub(super) struct Selector {
    decision: Option<Decision>,
    claims: [u64; 2],
    activity: [u64; 2],
    locked: [Option<bool>; 2],
    unlock_pending: [Option<u64>; 2],
    last_claim_at: u64,
    unlock_gate: bool,
    monitor: Option<u32>,
    monitor_candidate: Option<(u32, u64)>,
    pending: Option<(String, String, u64)>,
    last_human: u64,
    last_switch: u64,
    entered: bool,
}
impl Selector {
    pub fn update(&mut self, now: u64, options: &Preferences, mac: &State, pc: &State) -> Decision {
        if !self.entered {
            self.decision = Some(Decision {
                target: options.auto_preferred.clone(),
                revision: 1,
                reason: "Computador preferido na chegada".into(),
                waiting_release: false,
                local_control: false,
            });
            self.entered = true;
            self.last_human = now;
        }
        let mut request: Option<(String, String)> = None;
        let idle =
            now.saturating_sub(self.last_human) >= u64::from(options.auto_idle_seconds) * 1000;
        for (i, (side, state)) in [("mac", mac), ("pc", pc)].into_iter().enumerate() {
            if let Some(a) = &state.activity {
                if a.serial != self.activity[i] && a.age_ms.is_some_and(|age| age < 2000) {
                    self.activity[i] = a.serial;
                    if !state.agent {
                        self.last_human = now;
                    }
                }
                let unlocked = self.locked[i] == Some(true) && state.platform.locked == Some(false);
                if unlocked {
                    self.unlock_gate = true;
                    self.unlock_pending[i] = Some(now);
                }
                if state.agent {
                    self.unlock_pending[i] = None;
                }
                if let Some(at) = self.unlock_pending[i] {
                    if now.saturating_sub(at) > 5000 {
                        self.unlock_pending[i] = None;
                    } else if options.auto_unlock
                        && a.age_ms
                            .is_some_and(|age| now.saturating_sub(age) >= at.saturating_sub(500))
                    {
                        request = Some((side.into(), "Desbloqueio com atividade humana".into()));
                        self.unlock_pending[i] = None;
                    }
                }
                if a.claim_serial != self.claims[i] {
                    self.claims[i] = a.claim_serial;
                    if !state.agent
                        && options.auto_local_activity
                        && a.claim_age_ms.is_some_and(|age| age < 1500)
                    {
                        // Mac input is an explicit local takeover. A donor's
                        // forwarded input never claims the donor again.
                        self.last_claim_at = now;
                        request = Some((
                            side.into(),
                            if idle {
                                "Atividade humana na chegada"
                            } else {
                                "Atividade local assumiu o controle"
                            }
                            .into(),
                        ));
                    }
                }
            }
            self.locked[i] = state.platform.locked;
        }
        let inhibited = mac.agent || pc.agent || mac.manual_pause || pc.manual_pause;
        if !inhibited
            && options.auto_monitor
            && options.shared_monitor
            && pc.platform.input_age_ms.is_some_and(|age| age < 12000)
        {
            if let Some(input) = pc
                .platform
                .input
                .filter(|v| *v == options.mac_input || *v == options.pc_input)
            {
                match self.monitor_candidate {
                    Some((v, since)) if v == input && now.saturating_sub(since) >= 1200 => {
                        if self.monitor != Some(input) {
                            let initial = self.monitor.is_none();
                            self.monitor = Some(input);
                            if request.is_none() && (!initial || self.last_claim_at == 0) {
                                request = Some((
                                    if input == options.mac_input {
                                        "mac"
                                    } else {
                                        "pc"
                                    }
                                    .into(),
                                    "Entrada do monitor compartilhado mudou".into(),
                                ));
                            }
                        }
                    }
                    Some((v, _)) if v == input => {}
                    _ => self.monitor_candidate = Some((input, now)),
                }
            } else {
                self.monitor_candidate = None;
            }
        } else {
            self.monitor_candidate = None;
        }
        // Do not decide while either desktop is under agent control or paused.
        if inhibited {
            request = None;
            self.pending = None;
        }
        if let Some((target, reason)) = request {
            if target != self.decision.as_ref().unwrap().target {
                self.pending = Some((target, reason, now));
            } else {
                self.pending = None;
            }
        }
        let held = [mac, pc]
            .iter()
            .any(|s| s.activity.as_ref().is_some_and(|a| a.held));
        if let Some((target, reason, at)) = self.pending.clone() {
            if now.saturating_sub(at) > 15000 {
                self.pending = None;
            } else if !held && now.saturating_sub(self.last_switch) >= 1200 {
                let d = self.decision.as_mut().unwrap();
                d.target = target;
                d.reason = reason;
                d.revision += 1;
                self.last_switch = now;
                self.pending = None;
            }
        }
        if self.unlock_pending.iter().all(Option::is_none) && self.pending.is_none() && !held {
            self.unlock_gate = false;
        }
        let d = self.decision.as_mut().unwrap();
        d.local_control = options.auto_unlock
            && (self.unlock_gate
                || mac.platform.locked == Some(true)
                || pc.platform.locked == Some(true));
        d.waiting_release = self.pending.is_some() && held;
        d.clone()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use input_event::activity::Sample;
    fn pair() -> (Preferences, State, State) {
        let p = Preferences {
            input_mode: "auto".into(),
            auto_monitor: false,
            ..Default::default()
        };
        let s = State {
            activity: Some(Sample {
                ready: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        (p, s.clone(), s)
    }
    fn claim(s: &mut State, n: u64) {
        let a = s.activity.as_mut().unwrap();
        a.claim_serial = n;
        a.serial = n;
        a.age_ms = Some(0);
        a.claim_age_ms = Some(0);
    }
    #[test]
    fn unlock_waits_for_the_first_physical_event_instead_of_a_single_poll() {
        let (p, m, mut pc) = pair();
        pc.platform.locked = Some(true);
        let mut x = Selector::default();
        x.update(2000, &p, &m, &pc);
        pc.platform.locked = Some(false);
        assert_eq!(x.update(4000, &p, &m, &pc).target, "mac");
        let a = pc.activity.as_mut().unwrap();
        a.serial = 1;
        a.age_ms = Some(0);
        assert_eq!(x.update(4500, &p, &m, &pc).target, "pc");
    }
    #[test]
    fn first_monitor_sample_does_not_steal_a_recent_local_claim() {
        let (mut p, mut m, mut pc) = pair();
        p.shared_monitor = true;
        p.auto_monitor = true;
        p.mac_input = 19;
        p.pc_input = 15;
        pc.platform.input = Some(15);
        pc.platform.input_age_ms = Some(0);
        let mut x = Selector::default();
        claim(&mut m, 1);
        x.update(2000, &p, &m, &pc);
        assert_eq!(x.update(4000, &p, &m, &pc).target, "mac");
    }
    #[test]
    fn expired_pending_change_never_forces_a_drag_release() {
        let (p, mut m, mut pc) = pair();
        let mut x = Selector::default();
        x.update(2000, &p, &m, &pc);
        m.activity.as_mut().unwrap().held = true;
        claim(&mut pc, 1);
        x.update(3000, &p, &m, &pc);
        assert_eq!(x.update(20000, &p, &m, &pc).target, "mac");
        m.activity.as_mut().unwrap().held = false;
        assert_eq!(x.update(21000, &p, &m, &pc).target, "mac");
    }
    #[test]
    fn monitor_change_while_paused_is_reconsidered_after_resume() {
        let (mut p, m, mut pc) = pair();
        p.shared_monitor = true;
        p.auto_monitor = true;
        p.mac_input = 19;
        p.pc_input = 15;
        pc.platform.input = Some(15);
        pc.platform.input_age_ms = Some(0);
        let mut x = Selector::default();
        x.update(2000, &p, &m, &pc);
        assert_eq!(x.update(3500, &p, &m, &pc).target, "pc");
        pc.manual_pause = true;
        pc.platform.input = Some(19);
        x.update(5000, &p, &m, &pc);
        assert_eq!(x.update(7000, &p, &m, &pc).target, "pc");
        pc.manual_pause = false;
        x.update(8000, &p, &m, &pc);
        assert_eq!(x.update(9500, &p, &m, &pc).target, "mac");
    }
    #[test]
    fn first_click_after_unlock_remains_local_through_its_release() {
        let (p, m, mut pc) = pair();
        pc.platform.locked = Some(true);
        let mut x = Selector::default();
        assert!(x.update(2000, &p, &m, &pc).local_control);
        pc.platform.locked = Some(false);
        assert!(x.update(4000, &p, &m, &pc).local_control);
        let a = pc.activity.as_mut().unwrap();
        a.serial = 1;
        a.age_ms = Some(0);
        a.held = true;
        let d = x.update(4500, &p, &m, &pc);
        assert!(d.local_control && d.waiting_release);
        assert_eq!(d.target, "mac");
        pc.activity.as_mut().unwrap().held = false;
        let d = x.update(5000, &p, &m, &pc);
        assert!(!d.local_control);
        assert_eq!(d.target, "pc");
    }
    #[test]
    fn donor_activity_does_not_bounce_back() {
        let (p, m, mut pc) = pair();
        let mut x = Selector::default();
        assert_eq!(x.update(2000, &p, &m, &pc).target, "mac");
        pc.activity.as_mut().unwrap().serial = 50;
        pc.activity.as_mut().unwrap().age_ms = Some(0);
        assert_eq!(x.update(4000, &p, &m, &pc).target, "mac");
    }
    #[test]
    fn claims_wait_for_release_and_agents_cannot_select() {
        let (p, mut m, mut pc) = pair();
        let mut x = Selector::default();
        x.update(2000, &p, &m, &pc);
        claim(&mut pc, 1);
        m.activity.as_mut().unwrap().held = true;
        assert!(x.update(4000, &p, &m, &pc).waiting_release);
        m.activity.as_mut().unwrap().held = false;
        assert_eq!(x.update(5000, &p, &m, &pc).target, "pc");
        claim(&mut m, 1);
        m.agent = true;
        assert_eq!(x.update(7000, &p, &m, &pc).target, "pc");
        m.agent = false;
        assert_eq!(x.update(8000, &p, &m, &pc).target, "pc");
    }
    #[test]
    fn monitor_requires_stability_and_local_claim_wins() {
        let (mut p, mut m, mut pc) = pair();
        p.shared_monitor = true;
        p.auto_monitor = true;
        p.mac_input = 19;
        p.pc_input = 15;
        pc.platform.input = Some(15);
        pc.platform.input_age_ms = Some(0);
        let mut x = Selector::default();
        x.update(2000, &p, &m, &pc);
        assert_eq!(x.update(2500, &p, &m, &pc).target, "mac");
        claim(&mut m, 1);
        assert_eq!(x.update(3400, &p, &m, &pc).target, "mac");
        assert_eq!(x.update(5500, &p, &m, &pc).target, "mac");
        pc.platform.input = Some(19);
        x.update(6000, &p, &m, &pc);
        x.update(7500, &p, &m, &pc);
        pc.platform.input = Some(15);
        x.update(8000, &p, &m, &pc);
        assert_eq!(x.update(9500, &p, &m, &pc).target, "pc");
    }
    #[test]
    fn stale_monitor_and_synthetic_unlock_never_claim() {
        let (mut p, m, mut pc) = pair();
        p.shared_monitor = true;
        p.auto_monitor = true;
        p.mac_input = 19;
        p.pc_input = 15;
        pc.platform.input = Some(15);
        pc.platform.input_age_ms = Some(20000);
        pc.platform.locked = Some(true);
        let mut x = Selector::default();
        x.update(2000, &p, &m, &pc);
        pc.platform.locked = Some(false);
        assert_eq!(x.update(4000, &p, &m, &pc).target, "mac");
    }
}
