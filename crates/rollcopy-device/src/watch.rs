//! Connection guide state machine (PLAN §5), headless.
//!
//! [`watch`] polls usbmuxd once a second and reports a [`Phase`] whenever the
//! "one clear next step" for the user changes. It follows the first phone
//! usbmuxd lists; v1 handles one phone at a time.

use std::path::Path;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;

use crate::error::DeviceError;
use crate::session::{DeviceInfo, DeviceRef, Session, list_devices, pair};

const POLL: Duration = Duration::from_secs(1);
/// Retry interval while the phone is locked or a request failed.
const RETRY: Duration = Duration::from_secs(2);
/// Pair requests that may fail (other than Don't Trust or a locked phone)
/// before Rollcopy stops asking. Each new request is a new Trust dialog on
/// the phone, so retrying forever showed one every few seconds (review C6).
const MAX_PAIR_FAILURES: u32 = 3;
/// A phone that vanishes this soon after appearing points at a bad cable or
/// port (seen 2026-11-12: usbmuxd attach, then kernel error -71).
const FLAKY_WITHIN: Duration = Duration::from_secs(5);
/// usbmuxd 1.1+ pairs by itself when a phone without a record is plugged in
/// (it shows the Trust dialog and saves the record). Give it this long before
/// sending our own request, so the user never gets two requests at once
/// (seen 2026-11-12: both ran, ours lost the race to save).
const USBMUXD_PAIRING_WAIT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoDeviceHint {
    /// The kernel sees an Apple USB device but usbmuxd doesn't list it.
    AppleUsbNotListed,
    /// A phone attached and dropped again within a few seconds.
    Flaky,
}

/// What the connection guide should show.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// The usbmuxd program is not installed.
    UsbmuxdMissing,
    /// "Plug in your iPhone with a cable", plus an optional hint.
    NoDevice { hint: Option<NoDeviceHint> },
    /// A phone is listed; handshake in progress.
    Connecting { udid: String },
    /// Pair request sent: "Unlock your iPhone and tap Trust, then enter your passcode".
    WaitingForTrust { udid: String },
    /// The user tapped Don't Trust: "Unplug and re-plug to ask again".
    TrustDenied { udid: String },
    /// Pairing failed several times; Rollcopy stopped asking so the phone
    /// doesn't keep showing Trust dialogs. "Unplug and re-plug to try again".
    PairingFailed { udid: String, message: String },
    /// "Unlock your iPhone".
    Locked { udid: String },
    /// Connected and trusted.
    Ready { info: Box<DeviceInfo> },
    /// Something unexpected; retried automatically.
    Failed { udid: String, message: String },
}

impl Phase {
    pub fn udid(&self) -> Option<&str> {
        match self {
            Self::Connecting { udid }
            | Self::WaitingForTrust { udid }
            | Self::TrustDenied { udid }
            | Self::PairingFailed { udid, .. }
            | Self::Locked { udid }
            | Self::Failed { udid, .. } => Some(udid),
            Self::Ready { info } => Some(&info.udid),
            Self::UsbmuxdMissing | Self::NoDevice { .. } => None,
        }
    }
}

/// Whether the usbmuxd daemon is installed. Its socket only exists while an
/// Apple device is plugged in, so the socket alone can't tell us this.
pub fn usbmuxd_installed() -> bool {
    ["/usr/sbin/usbmuxd", "/usr/bin/usbmuxd", "/sbin/usbmuxd"]
        .iter()
        .any(|p| Path::new(p).exists())
}

/// Whether the kernel currently has an Apple (vendor 05ac) USB device.
pub fn apple_usb_present() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return false;
    };
    entries.flatten().any(|e| {
        std::fs::read_to_string(e.path().join("idVendor")).is_ok_and(|v| v.trim() == "05ac")
    })
}

/// The phone currently being followed.
struct Tracked {
    dev: DeviceRef,
    since: Instant,
    /// Set once it reached Ready; a quick drop after that isn't "flaky".
    was_ready: bool,
    /// Don't Trust was tapped; iOS won't ask again until it's re-plugged.
    denied: bool,
    pairing: Option<JoinHandle<Result<(), DeviceError>>>,
    next_try: Instant,
    /// First time this phone was seen without a pair record.
    unpaired_since: Option<Instant>,
    /// Pair requests that failed so far (see [`after_pair`]).
    pair_failures: u32,
    /// Stopped asking after too many failures, until it's re-plugged.
    gave_up: bool,
}

/// What to do after a pair request ended.
#[derive(Debug, PartialEq)]
enum PairOutcome {
    /// Paired: connect again now.
    Paired,
    /// Show this and try again after the delay.
    Retry(Phase, Duration),
    /// Show this and don't ask again until the phone is re-plugged.
    Stop(Phase),
}

/// Decides what follows a pair request. "Still waiting for Trust" just
/// waits; Don't Trust stops; a locked phone retries; anything else backs
/// off (2 s, 4 s, …) and stops after [`MAX_PAIR_FAILURES`], because every
/// request makes the phone show the Trust dialog again (review C6).
/// `failures` counts failures before this one; it's updated.
fn after_pair(udid: &str, result: Result<(), DeviceError>, failures: &mut u32) -> PairOutcome {
    let udid = udid.to_owned();
    match result {
        Ok(()) => PairOutcome::Paired,
        Err(DeviceError::PairingPending) => {
            PairOutcome::Retry(Phase::WaitingForTrust { udid }, RETRY)
        }
        Err(DeviceError::PairingDenied) => PairOutcome::Stop(Phase::TrustDenied { udid }),
        Err(DeviceError::Locked) => PairOutcome::Retry(Phase::Locked { udid }, RETRY),
        Err(e) => {
            *failures += 1;
            let message = e.to_string();
            if *failures >= MAX_PAIR_FAILURES {
                PairOutcome::Stop(Phase::PairingFailed { udid, message })
            } else {
                let wait = RETRY * 2u32.pow(*failures - 1);
                PairOutcome::Retry(Phase::Failed { udid, message }, wait)
            }
        }
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        if let Some(t) = self.pairing.take() {
            t.abort();
        }
    }
}

/// Forwards a phase only when it differs from the previous one.
struct Emitter<F> {
    last: Option<Phase>,
    emit: F,
}

impl<F: FnMut(Phase)> Emitter<F> {
    fn send(&mut self, p: Phase) {
        if self.last.as_ref() != Some(&p) {
            self.last = Some(p.clone());
            (self.emit)(p);
        }
    }
}

/// Runs forever, calling `emit` each time the phase changes.
pub async fn watch(emit: impl FnMut(Phase)) {
    let mut out = Emitter { last: None, emit };

    let mut tracked: Option<Tracked> = None;
    let mut hint: Option<NoDeviceHint> = None;
    let mut ready = false;

    loop {
        let listed = list_devices().await;
        let first = match &listed {
            Ok(list) => list.first().cloned(),
            Err(_) => None,
        };

        // Same phone still there? usbmuxd gives a new mux id on every re-plug.
        let same = matches!((&tracked, &first), (Some(t), Some(d)) if t.dev == *d);
        if !same {
            if let Some(t) = tracked.take()
                && !t.was_ready
                && t.since.elapsed() < FLAKY_WITHIN
            {
                hint = Some(NoDeviceHint::Flaky);
            }
            ready = false;
            tracked = first.map(|dev| Tracked {
                dev,
                since: Instant::now(),
                was_ready: false,
                denied: false,
                pairing: None,
                next_try: Instant::now(),
                unpaired_since: None,
                pair_failures: 0,
                gave_up: false,
            });
        }

        let Some(t) = tracked.as_mut() else {
            match listed {
                Err(_) if !usbmuxd_installed() => out.send(Phase::UsbmuxdMissing),
                _ => {
                    if hint.is_none() && apple_usb_present() {
                        hint = Some(NoDeviceHint::AppleUsbNotListed);
                    }
                    out.send(Phase::NoDevice { hint: hint.clone() });
                }
            }
            tokio::time::sleep(POLL).await;
            continue;
        };
        let udid = t.dev.udid.clone();

        if ready || t.denied || t.gave_up {
            tokio::time::sleep(POLL).await;
            continue;
        }

        // A pair request is running: wait for its result.
        if t.pairing.is_some() {
            if let Some(task) = t.pairing.take_if(|task| task.is_finished()) {
                let result = task
                    .await
                    .unwrap_or_else(|join| Err(DeviceError::Other(join.to_string())));
                match after_pair(&udid, result, &mut t.pair_failures) {
                    PairOutcome::Paired => t.next_try = Instant::now(),
                    PairOutcome::Retry(phase, wait) => {
                        t.next_try = Instant::now() + wait;
                        out.send(phase);
                    }
                    PairOutcome::Stop(phase) => {
                        t.denied = matches!(phase, Phase::TrustDenied { .. });
                        t.gave_up = !t.denied;
                        out.send(phase);
                    }
                }
            }
            tokio::time::sleep(POLL).await;
            continue;
        }

        if Instant::now() < t.next_try {
            tokio::time::sleep(POLL).await;
            continue;
        }

        if out.last.as_ref().and_then(Phase::udid) != Some(udid.as_str()) {
            out.send(Phase::Connecting { udid: udid.clone() });
        }
        match open_and_read(&udid).await {
            Ok(info) => {
                t.was_ready = true;
                ready = true;
                hint = None;
                out.send(Phase::Ready {
                    info: Box::new(info),
                });
            }
            Err(DeviceError::NotPaired) => {
                // usbmuxd is probably asking already; keep polling for its record.
                let since = *t.unpaired_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= USBMUXD_PAIRING_WAIT {
                    let u = udid.clone();
                    t.pairing = Some(tokio::spawn(async move { pair(&u).await }));
                }
                out.send(Phase::WaitingForTrust { udid });
            }
            Err(DeviceError::StalePairing) => {
                let u = udid.clone();
                t.pairing = Some(tokio::spawn(async move { pair(&u).await }));
                out.send(Phase::WaitingForTrust { udid });
            }
            Err(DeviceError::PairingPending) => out.send(Phase::WaitingForTrust { udid }),
            Err(DeviceError::PairingDenied) => {
                t.denied = true;
                out.send(Phase::TrustDenied { udid });
            }
            Err(DeviceError::Locked) => {
                t.next_try = Instant::now() + RETRY;
                out.send(Phase::Locked { udid });
            }
            // Gone between listing and opening: the next poll handles it.
            Err(DeviceError::NotConnected | DeviceError::Disconnected(_)) => {}
            Err(e) => {
                t.next_try = Instant::now() + RETRY;
                out.send(Phase::Failed {
                    udid,
                    message: e.to_string(),
                });
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

async fn open_and_read(udid: &str) -> Result<DeviceInfo, DeviceError> {
    let mut s = Session::open(udid).await?;
    s.info().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_for_trust_is_not_a_failure() {
        let mut failures = 0;
        assert_eq!(
            after_pair("u", Err(DeviceError::PairingPending), &mut failures),
            PairOutcome::Retry(Phase::WaitingForTrust { udid: "u".into() }, RETRY)
        );
        assert_eq!(failures, 0);
        assert_eq!(after_pair("u", Ok(()), &mut failures), PairOutcome::Paired);
    }

    /// A pair request that keeps failing backs off, then stops: every
    /// request is a new Trust dialog on the phone (review C6).
    #[test]
    fn failing_pair_requests_back_off_then_stop() {
        let mut failures = 0;
        let fail = || Err(DeviceError::Other("save refused".into()));
        let waits: Vec<_> = (0..2)
            .map(|_| match after_pair("u", fail(), &mut failures) {
                PairOutcome::Retry(Phase::Failed { .. }, wait) => wait,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(waits, vec![RETRY, RETRY * 2]);
        assert!(matches!(
            after_pair("u", fail(), &mut failures),
            PairOutcome::Stop(Phase::PairingFailed { .. })
        ));
        // Don't Trust and a locked phone aren't counted.
        let mut failures = 0;
        assert!(matches!(
            after_pair("u", Err(DeviceError::PairingDenied), &mut failures),
            PairOutcome::Stop(Phase::TrustDenied { .. })
        ));
        assert!(matches!(
            after_pair("u", Err(DeviceError::Locked), &mut failures),
            PairOutcome::Retry(Phase::Locked { .. }, _)
        ));
        assert_eq!(failures, 0);
    }
}
