//! Pairing (phone view spec section 5) and the rate limits of the listener. All of it lives in
//! memory: a pairing that Dots forgets on a restart is simply started again.
//!
//! 1. Settings starts a pairing: a one-time token (256 bits, in the QR code's link) and a short
//!    code (8 characters, for typing), both good for 5 minutes and for one claim between them.
//!    Five wrong guesses end the pairing.
//! 2. The phone claims it with the token or the code: the pairing is used up, and the phone
//!    gets a claim secret and a 4-digit number to compare with the PC's dialog.
//! 3. The person allows the phone on the PC, in a dialog the phone cannot answer.
//! 4. The phone collects its credential with the claim secret: only then is it stored, as a
//!    hash.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::util::{ct_eq, new_token};

/// How long a pairing's token and code are good for.
pub const PAIRING_LIFETIME: Duration = Duration::from_secs(5 * 60);
/// Wrong tokens or codes after which a pairing ends.
pub const PAIRING_MAX_FAILURES: u8 = 5;
/// How long a claim waits for the person's answer on the PC.
pub const CLAIM_WAIT: Duration = Duration::from_secs(2 * 60);
/// How long an allowed phone has to collect its credential.
pub const COLLECT_WITHIN: Duration = Duration::from_secs(60);
/// The failures (bad credential, pairing secret or claim) one address may make in
/// `LIMIT_WINDOW` before it is refused until the window ends.
pub const LIMIT_FAILURES: u32 = 10;
pub const LIMIT_WINDOW: Duration = Duration::from_secs(10 * 60);
/// The addresses the limiter remembers at most; past that, the oldest windows are dropped.
const LIMIT_ADDRESSES: usize = 1024;

/// The characters of a short code: no 0/O, 1/I/L, which read alike.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 8;

/// A pairing that waits for a phone.
#[derive(Debug, Clone)]
pub struct Pairing {
    pub token: String,
    /// Without the dash.
    pub code: String,
    pub expires: Instant,
    failures: u8,
}

/// A phone that presented a pairing's token or code.
#[derive(Debug, Clone)]
pub struct Claim {
    pub secret: String,
    pub name: String,
    pub ip: IpAddr,
    pub match_code: String,
    pub state: ClaimState,
    /// When it stops waiting (`Waiting`) or stops being collectable (`Allowed`).
    pub until: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimState {
    Waiting,
    Allowed,
    Refused,
}

/// What a claim's secret answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collected {
    Waiting,
    /// Allowed: the phone's name, to store with its credential.
    Allowed {
        name: String,
    },
    Refused,
    Expired,
}

/// Why a claim was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimRefusal {
    /// No pairing is waiting (none started, expired, ended, or used).
    NoPairing,
    /// The token or code is wrong; the pairing may have ended because of it.
    Wrong,
    /// Another phone's claim waits for the person.
    Busy,
}

/// Random characters from `CODE_ALPHABET`, without bias.
fn random_code() -> String {
    let mut out = String::with_capacity(CODE_LEN);
    let limit = 256 - (256 % CODE_ALPHABET.len());
    while out.len() < CODE_LEN {
        let mut buf = [0u8; 16];
        getrandom::fill(&mut buf).expect("OS random number generator unavailable");
        for b in buf {
            if (b as usize) < limit && out.len() < CODE_LEN {
                out.push(CODE_ALPHABET[b as usize % CODE_ALPHABET.len()] as char);
            }
        }
    }
    out
}

/// Four random digits.
fn random_match() -> String {
    let mut buf = [0u8; 4];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    format!("{:04}", u32::from_le_bytes(buf) % 10_000)
}

/// A typed code as stored: capitals, without spaces or dashes.
pub fn normalize_code(typed: &str) -> String {
    typed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// The code as shown: `ABCD-EFGH`.
pub fn show_code(code: &str) -> String {
    let (a, b) = code.split_at(code.len() / 2);
    format!("{a}-{b}")
}

/// The pairing in progress and the claim made on it.
#[derive(Debug, Default)]
pub struct Pairings {
    pairing: Option<Pairing>,
    claim: Option<Claim>,
}

impl Pairings {
    /// A new pairing, replacing one that waits.
    pub fn start(&mut self, now: Instant) -> Pairing {
        let pairing = Pairing {
            token: new_token(),
            code: random_code(),
            expires: now + PAIRING_LIFETIME,
            failures: 0,
        };
        self.pairing = Some(pairing.clone());
        pairing
    }

    /// Ends the pairing that waits, and a claim that was not allowed yet.
    pub fn cancel(&mut self) {
        self.pairing = None;
        if self
            .claim
            .as_ref()
            .is_some_and(|c| c.state == ClaimState::Waiting)
        {
            self.claim = None;
        }
    }

    /// Forgets everything (the listener stopped).
    pub fn clear(&mut self) {
        self.pairing = None;
        self.claim = None;
    }

    /// Whether a pairing waits now.
    pub fn waiting(&self, now: Instant) -> bool {
        self.pairing.as_ref().is_some_and(|p| now < p.expires)
    }

    /// Claims the pairing with `secret`, its token or its code. The pairing is used up by a
    /// right secret, and ended by the fifth wrong one.
    pub fn claim(
        &mut self,
        secret: &str,
        name: String,
        ip: IpAddr,
        now: Instant,
    ) -> Result<Claim, ClaimRefusal> {
        if let Some(c) = &self.claim {
            let over = match c.state {
                ClaimState::Waiting | ClaimState::Allowed => now >= c.until,
                ClaimState::Refused => true,
            };
            if over {
                self.claim = None;
            }
        }
        let Some(pairing) = self.pairing.as_mut().filter(|p| now < p.expires) else {
            self.pairing = None;
            return Err(ClaimRefusal::NoPairing);
        };
        let right = ct_eq(secret, &pairing.token) || ct_eq(&normalize_code(secret), &pairing.code);
        if !right {
            pairing.failures += 1;
            if pairing.failures >= PAIRING_MAX_FAILURES {
                self.pairing = None;
            }
            return Err(ClaimRefusal::Wrong);
        }
        if self.claim.is_some() {
            return Err(ClaimRefusal::Busy);
        }
        self.pairing = None;
        let claim = Claim {
            secret: new_token(),
            name,
            ip,
            match_code: random_match(),
            state: ClaimState::Waiting,
            until: now + CLAIM_WAIT,
        };
        self.claim = Some(claim.clone());
        Ok(claim)
    }

    /// Records the person's answer to the claim `secret`, if it still waits. False when it no
    /// longer does (expired, cancelled, or replaced).
    pub fn answer(&mut self, secret: &str, allowed: bool, now: Instant) -> bool {
        match self.claim.as_mut() {
            Some(c) if ct_eq(&c.secret, secret) && c.state == ClaimState::Waiting => {
                if now >= c.until {
                    self.claim = None;
                    return false;
                }
                if allowed {
                    c.state = ClaimState::Allowed;
                    c.until = now + COLLECT_WITHIN;
                } else {
                    c.state = ClaimState::Refused;
                }
                true
            }
            _ => false,
        }
    }

    /// What the claim `secret` stands at. `None` when no claim has that secret. An allowed,
    /// refused or expired claim is answered once and then forgotten.
    pub fn collect(&mut self, secret: &str, now: Instant) -> Option<Collected> {
        let claim = self.claim.as_ref().filter(|c| ct_eq(&c.secret, secret))?;
        let answer = match (claim.state, now < claim.until) {
            (ClaimState::Waiting, true) => return Some(Collected::Waiting),
            (ClaimState::Allowed, true) => Collected::Allowed {
                name: claim.name.clone(),
            },
            (ClaimState::Refused, _) => Collected::Refused,
            (_, false) => Collected::Expired,
        };
        self.claim = None;
        Some(answer)
    }
}

/// Failures per address in a fixed window (`LIMIT_FAILURES` per `LIMIT_WINDOW`).
#[derive(Debug, Default)]
pub struct Limiter {
    windows: HashMap<IpAddr, (Instant, u32)>,
}

impl Limiter {
    /// Whether `ip` is refused for now.
    pub fn blocked(&mut self, ip: IpAddr, now: Instant) -> bool {
        match self.windows.get(&ip) {
            Some((start, n)) if now.duration_since(*start) < LIMIT_WINDOW => *n >= LIMIT_FAILURES,
            Some(_) => {
                self.windows.remove(&ip);
                false
            }
            None => false,
        }
    }

    /// Counts a failure of `ip`.
    pub fn fail(&mut self, ip: IpAddr, now: Instant) {
        if self.windows.len() >= LIMIT_ADDRESSES && !self.windows.contains_key(&ip) {
            self.windows
                .retain(|_, (start, _)| now.duration_since(*start) < LIMIT_WINDOW);
            if self.windows.len() >= LIMIT_ADDRESSES {
                // Still full of live windows: drop the oldest.
                if let Some(oldest) = self
                    .windows
                    .iter()
                    .min_by_key(|(_, (start, _))| *start)
                    .map(|(ip, _)| *ip)
                {
                    self.windows.remove(&oldest);
                }
            }
        }
        let entry = self.windows.entry(ip).or_insert((now, 0));
        if now.duration_since(entry.0) >= LIMIT_WINDOW {
            *entry = (now, 0);
        }
        entry.1 += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip() -> IpAddr {
        "192.168.1.30".parse().unwrap()
    }

    #[test]
    fn codes_are_eight_unambiguous_characters() {
        for _ in 0..50 {
            let code = random_code();
            assert_eq!(code.len(), 8);
            assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)), "{code}");
        }
        assert_eq!(show_code("ABCDEFGH"), "ABCD-EFGH");
        assert_eq!(normalize_code(" abcd-efgh "), "ABCDEFGH");
        assert_eq!(random_match().len(), 4);
    }

    #[test]
    fn a_token_claims_once() {
        let now = Instant::now();
        let mut p = Pairings::default();
        let pairing = p.start(now);
        let claim = p.claim(&pairing.token, "iPhone".into(), ip(), now).unwrap();
        assert_eq!(claim.state, ClaimState::Waiting);
        // Used up: the same token, or the code, claims nothing more.
        assert_eq!(
            p.claim(&pairing.token, "x".into(), ip(), now).unwrap_err(),
            ClaimRefusal::NoPairing
        );
        assert_eq!(
            p.claim(&pairing.code, "x".into(), ip(), now).unwrap_err(),
            ClaimRefusal::NoPairing
        );
    }

    #[test]
    fn the_typed_code_claims_too() {
        let now = Instant::now();
        let mut p = Pairings::default();
        let pairing = p.start(now);
        let typed = show_code(&pairing.code).to_lowercase();
        assert!(p.claim(&typed, "Pixel".into(), ip(), now).is_ok());
    }

    #[test]
    fn a_pairing_expires_after_five_minutes() {
        let now = Instant::now();
        let mut p = Pairings::default();
        let pairing = p.start(now);
        assert!(p.waiting(now));
        let later = now + PAIRING_LIFETIME;
        assert!(!p.waiting(later));
        assert_eq!(
            p.claim(&pairing.token, "x".into(), ip(), later)
                .unwrap_err(),
            ClaimRefusal::NoPairing
        );
    }

    #[test]
    fn five_wrong_guesses_end_the_pairing() {
        let now = Instant::now();
        let mut p = Pairings::default();
        let pairing = p.start(now);
        for _ in 0..PAIRING_MAX_FAILURES {
            assert_eq!(
                p.claim("WRONGCOD", "x".into(), ip(), now).unwrap_err(),
                ClaimRefusal::Wrong
            );
        }
        assert_eq!(
            p.claim(&pairing.token, "x".into(), ip(), now).unwrap_err(),
            ClaimRefusal::NoPairing
        );
    }

    #[test]
    fn a_claim_needs_the_persons_yes_before_it_can_be_collected() {
        let now = Instant::now();
        let mut p = Pairings::default();
        let pairing = p.start(now);
        let claim = p.claim(&pairing.token, "iPhone".into(), ip(), now).unwrap();
        assert_eq!(p.collect(&claim.secret, now), Some(Collected::Waiting));
        assert_eq!(p.collect("not-the-secret", now), None);
        assert!(p.answer(&claim.secret, true, now));
        assert_eq!(
            p.collect(&claim.secret, now),
            Some(Collected::Allowed {
                name: "iPhone".into()
            })
        );
        // Collected once.
        assert_eq!(p.collect(&claim.secret, now), None);
    }

    #[test]
    fn a_refused_or_late_claim_gives_no_credential() {
        let now = Instant::now();
        let mut p = Pairings::default();
        let pairing = p.start(now);
        let claim = p.claim(&pairing.token, "a".into(), ip(), now).unwrap();
        assert!(p.answer(&claim.secret, false, now));
        assert_eq!(p.collect(&claim.secret, now), Some(Collected::Refused));

        let pairing = p.start(now);
        let claim = p.claim(&pairing.token, "b".into(), ip(), now).unwrap();
        // The person answers after the claim stopped waiting: nothing is allowed.
        assert!(!p.answer(&claim.secret, true, now + CLAIM_WAIT));

        let pairing = p.start(now);
        let claim = p.claim(&pairing.token, "c".into(), ip(), now).unwrap();
        assert!(p.answer(&claim.secret, true, now));
        assert_eq!(
            p.collect(&claim.secret, now + COLLECT_WITHIN),
            Some(Collected::Expired)
        );
    }

    #[test]
    fn a_second_claim_waits_for_the_first() {
        let now = Instant::now();
        let mut p = Pairings::default();
        let first = p.start(now);
        let claim = p.claim(&first.token, "a".into(), ip(), now).unwrap();
        let second = p.start(now);
        assert_eq!(
            p.claim(&second.token, "b".into(), ip(), now).unwrap_err(),
            ClaimRefusal::Busy
        );
        // Cancelling ends a claim that waits.
        p.cancel();
        assert_eq!(p.collect(&claim.secret, now), None);
    }

    #[test]
    fn the_limiter_refuses_after_ten_failures_until_the_window_ends() {
        let now = Instant::now();
        let mut l = Limiter::default();
        for _ in 0..LIMIT_FAILURES {
            assert!(!l.blocked(ip(), now));
            l.fail(ip(), now);
        }
        assert!(l.blocked(ip(), now));
        let other: IpAddr = "192.168.1.31".parse().unwrap();
        assert!(!l.blocked(other, now));
        assert!(!l.blocked(ip(), now + LIMIT_WINDOW));
    }
}
