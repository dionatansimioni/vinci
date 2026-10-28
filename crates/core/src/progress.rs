//! Time-weighted progress for the install, so the bar keeps moving during long steps
//! instead of sitting still for minutes (users take a still bar for a hang).

use crate::container::StepKind;
use std::time::Duration;

/// Typical duration of each step, measured on the real runs in docs/PROVA.md. Only the
/// proportions matter for the bar; the absolute values drive the "time left" estimate.
pub fn expected(kind: StepKind) -> Duration {
    let secs = match kind {
        StepKind::PullImage => 10,
        StepKind::Snapshot => 420,
        StepKind::RemoveExisting => 3,
        StepKind::Create => 2,
        StepKind::Initialize => 190,
        StepKind::AddRepository => 15,
        StepKind::InstallPackages => 70,
        StepKind::ExtractInstaller => 55,
        StepKind::InstallResolve => 165,
        StepKind::Verify => 1,
        StepKind::RemoveConflictingLibs => 1,
    };
    Duration::from_secs(secs)
}

/// Share of a step done after `elapsed`: linear up to 90% at the expected time, then it
/// keeps creeping towards (never reaching) 100% so an overdue step still visibly moves.
fn within(elapsed: Duration, expected: Duration) -> f64 {
    let t = elapsed.as_secs_f64();
    let e = expected.as_secs_f64().max(1.0);
    if t <= e {
        0.9 * t / e
    } else {
        0.9 + 0.1 * (1.0 - (-(t - e) / e).exp())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    /// 0..1 for the whole plan.
    pub fraction: f64,
    /// Expected time left; `None` once the current step runs past its estimate (we only
    /// know it's "almost done", not how long).
    pub remaining: Option<Duration>,
}

/// Progress of the whole plan, given the steps, the one running and its elapsed time.
pub fn estimate(steps: &[StepKind], current: usize, elapsed_in_step: Duration) -> Estimate {
    let weights: Vec<f64> = steps.iter().map(|k| expected(*k).as_secs_f64()).collect();
    let total: f64 = weights.iter().sum::<f64>().max(1.0);
    let current = current.min(steps.len());
    let done: f64 = weights[..current].iter().sum();
    let (now, left_here) = match steps.get(current) {
        Some(k) => {
            let e = expected(*k);
            (
                weights[current] * within(elapsed_in_step, e),
                e.checked_sub(elapsed_in_step),
            )
        }
        None => (0.0, Some(Duration::ZERO)),
    };
    let later: Duration = steps.iter().skip(current + 1).map(|k| expected(*k)).sum();
    Estimate {
        fraction: ((done + now) / total).clamp(0.0, 1.0),
        remaining: left_here.map(|l| l + later),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use StepKind::*;

    const PLAN: [StepKind; 4] = [PullImage, Initialize, InstallResolve, Verify];

    fn s(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    #[test]
    fn starts_at_zero_and_ends_at_one() {
        let e = estimate(&PLAN, 0, s(0));
        assert_eq!(e.fraction, 0.0);
        assert_eq!(e.remaining, Some(s(10 + 190 + 165 + 1)));
        let done = estimate(&PLAN, PLAN.len(), s(0));
        assert_eq!(done.fraction, 1.0);
        assert_eq!(done.remaining, Some(Duration::ZERO));
    }

    #[test]
    fn long_steps_weigh_more_and_move_with_time() {
        // Halfway through the 190 s init the bar has clearly moved since it started.
        let start = estimate(&PLAN, 1, s(0)).fraction;
        let mid = estimate(&PLAN, 1, s(95)).fraction;
        assert!(mid - start > 0.2, "{start} → {mid}");
        assert_eq!(estimate(&PLAN, 1, s(95)).remaining, Some(s(95 + 165 + 1)));
    }

    #[test]
    fn overdue_step_keeps_moving_but_never_reaches_the_next() {
        let at_estimate = estimate(&PLAN, 1, s(190)).fraction;
        let late = estimate(&PLAN, 1, s(600)).fraction;
        let next = estimate(&PLAN, 2, s(0)).fraction;
        assert!(
            late > at_estimate && late < next,
            "{at_estimate} < {late} < {next}"
        );
        assert_eq!(estimate(&PLAN, 1, s(600)).remaining, None);
    }

    #[test]
    fn monotonic_over_a_whole_run() {
        let mut last = 0.0;
        for (i, k) in PLAN.iter().enumerate() {
            for t in (0..=expected(*k).as_secs() * 2).step_by(5) {
                let f = estimate(&PLAN, i, s(t)).fraction;
                assert!(f >= last, "went backwards at step {i}, t={t}");
                last = f;
            }
        }
    }
}
