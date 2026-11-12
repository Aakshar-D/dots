//! Spending caps (spend caps design, `docs/superpowers/specs/2026-11-06-spend-caps-design.md`).
//! A dot may have a daily cap on its estimated API cost and on its tokens; Settings may cap
//! every dot together by the day (cost, tokens) and by the month (cost). What a day spent is
//! read from the runs themselves: the cost and tokens of every run started since local
//! midnight, a run still going with what it has reported so far. A run whose cap is reached
//! waits instead of starting, and one that goes over its cap while it runs is stopped.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::model::Dot;
use crate::settings::Settings;
use crate::store::Store;
use crate::{Error, Result};

/// The lowest and highest cost cap, in USD.
pub const MIN_CAP_USD: f64 = 0.01;
pub const MAX_CAP_USD: f64 = 100_000.0;
/// The highest token cap (ten billion).
pub const MAX_CAP_TOKENS: u64 = 10_000_000_000;
/// The share of a cap at which the UI warns.
pub const WARN_AT: f64 = 0.8;
/// The words when a cost cap is out of bounds.
pub const USD_RULE: &str = "A cost cap is $0.01 to $100,000.";
/// The words when a token cap is out of bounds.
pub const TOKENS_RULE: &str = "A token cap is 1 to 10,000,000,000 tokens.";
/// How the error of a run stopped by a cap begins: "stopped: Scout reached its $2.00 daily
/// cap".
pub const STOPPED_BY_CAP: &str = "stopped: ";
/// **Run once anyway** on a run no cap holds.
pub const NOT_CAPPED: &str = "This run is not waiting for a spending cap.";
/// **Run once anyway** on a background run: those wait for the cap.
pub const BACKGROUND_WAITS: &str =
    "A background run waits for the cap: raise the cap or wait for tomorrow.";

/// A dot's daily caps; `None` is no cap.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DotCaps {
    pub daily_usd: Option<f64>,
    pub daily_tokens: Option<u64>,
}

impl DotCaps {
    pub fn validate(&self) -> Result<()> {
        check_usd(self.daily_usd)?;
        check_tokens(self.daily_tokens)
    }

    pub fn is_empty(&self) -> bool {
        self.daily_usd.is_none() && self.daily_tokens.is_none()
    }
}

/// The caps for every dot together, from Settings.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct GlobalCaps {
    pub daily_usd: Option<f64>,
    pub daily_tokens: Option<u64>,
    pub monthly_usd: Option<f64>,
}

impl From<&Settings> for GlobalCaps {
    fn from(s: &Settings) -> Self {
        Self {
            daily_usd: s.spend_daily_usd,
            daily_tokens: s.spend_daily_tokens,
            monthly_usd: s.spend_monthly_usd,
        }
    }
}

impl GlobalCaps {
    pub fn validate(&self) -> Result<()> {
        check_usd(self.daily_usd)?;
        check_tokens(self.daily_tokens)?;
        check_usd(self.monthly_usd)
    }

    fn has_daily(&self) -> bool {
        self.daily_usd.is_some() || self.daily_tokens.is_some()
    }
}

/// A cost cap, if set, is a finite number from $0.01 to $100,000.
pub fn check_usd(cap: Option<f64>) -> Result<()> {
    match cap {
        Some(v) if !(v.is_finite() && (MIN_CAP_USD..=MAX_CAP_USD).contains(&v)) => {
            Err(Error::Invalid(USD_RULE.into()))
        }
        _ => Ok(()),
    }
}

/// A token cap, if set, is 1 to ten billion.
pub fn check_tokens(cap: Option<u64>) -> Result<()> {
    match cap {
        Some(v) if !(1..=MAX_CAP_TOKENS).contains(&v) => Err(Error::Invalid(TOKENS_RULE.into())),
        _ => Ok(()),
    }
}

/// What runs spent: the estimated API cost the engine reported (a Claude subscription bills no
/// run; this is what it would cost on the API) and the tokens in + out.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Spend {
    pub cost_usd: f64,
    pub tokens: i64,
}

str_enum!(CapScope { Dot => "dot", All => "all" });

/// A cap that is reached, and the words for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapHit {
    pub text: String,
    pub scope: CapScope,
}

/// `$2.00`; whole cents.
pub fn usd(v: f64) -> String {
    format!("${v:.2}")
}

/// `200,000`.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub(crate) fn cost_reached(spent: f64, cap: Option<f64>) -> Option<f64> {
    cap.filter(|cap| spent + 1e-9 >= *cap)
}

pub(crate) fn tokens_reached(spent: i64, cap: Option<u64>) -> Option<u64> {
    cap.filter(|cap| spent >= 0 && spent as u64 >= *cap)
}

/// The first cap reached, if any: the dot's daily cost, its daily tokens, then every dot's
/// daily cost, daily tokens and monthly cost. A cap is reached once the spend is at it or over.
pub fn first_hit(
    dot_name: &str,
    dot: &DotCaps,
    dot_today: Spend,
    global: &GlobalCaps,
    all_today: Spend,
    all_month: Spend,
) -> Option<CapHit> {
    let of_dot = |text: String| CapHit {
        text,
        scope: CapScope::Dot,
    };
    let of_all = |text: String| CapHit {
        text,
        scope: CapScope::All,
    };
    if let Some(cap) = cost_reached(dot_today.cost_usd, dot.daily_usd) {
        return Some(of_dot(format!(
            "{dot_name} reached its {} daily cap",
            usd(cap)
        )));
    }
    if let Some(cap) = tokens_reached(dot_today.tokens, dot.daily_tokens) {
        return Some(of_dot(format!(
            "{dot_name} reached its {}-token daily cap",
            thousands(cap)
        )));
    }
    if let Some(cap) = cost_reached(all_today.cost_usd, global.daily_usd) {
        return Some(of_all(format!(
            "All dots reached the {} daily cap",
            usd(cap)
        )));
    }
    if let Some(cap) = tokens_reached(all_today.tokens, global.daily_tokens) {
        return Some(of_all(format!(
            "All dots reached the {}-token daily cap",
            thousands(cap)
        )));
    }
    if let Some(cap) = cost_reached(all_month.cost_usd, global.monthly_usd) {
        return Some(of_all(format!(
            "All dots reached the {} monthly cap",
            usd(cap)
        )));
    }
    None
}

/// The local day of `now` in `tz`, `[start, end)`, as stored times are written.
pub fn day_bounds_in<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> (String, String) {
    let (start, end) = crate::delivery::local_day_bounds(now, tz);
    (crate::util::timestamp(start), crate::util::timestamp(end))
}

/// The local month of `now` in `tz`, `[start, end)`: midnight of the 1st to midnight of the
/// next month's 1st.
pub fn month_bounds_in<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> (String, String) {
    let date = now.with_timezone(tz).date_naive();
    let first = NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date);
    let next = if date.month() == 12 {
        NaiveDate::from_ymd_opt(date.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1)
    }
    .unwrap_or(date);
    let at = |d| crate::util::timestamp(crate::delivery::midnight(tz, d, now));
    (at(first), at(next))
}

/// What one look at the caps reads: Settings' caps and the spend of today and of the month,
/// each read once, the dots' as they are asked for. The dispatcher makes one per pass.
pub struct Ledger {
    global: GlobalCaps,
    day: (String, String),
    month: (String, String),
    all_today: Option<Spend>,
    all_month: Option<Spend>,
    dots: HashMap<String, Spend>,
}

impl Ledger {
    /// The caps of `settings`, for the local day and month of `now`.
    pub fn new(settings: &Settings, now: DateTime<Utc>) -> Self {
        Self {
            global: settings.into(),
            day: day_bounds_in(now, &Local),
            month: month_bounds_in(now, &Local),
            all_today: None,
            all_month: None,
            dots: HashMap::new(),
        }
    }

    /// Reads Settings from the store, for now.
    pub async fn load(store: &Store) -> Result<Self> {
        Ok(Self::new(&store.get_settings().await?, Utc::now()))
    }

    /// What `dot_id` spent today.
    pub async fn dot_today(&mut self, store: &Store, dot_id: &str) -> Result<Spend> {
        if let Some(s) = self.dots.get(dot_id) {
            return Ok(*s);
        }
        let s = store
            .spend_between(Some(dot_id), &self.day.0, &self.day.1)
            .await?;
        self.dots.insert(dot_id.to_string(), s);
        Ok(s)
    }

    /// The first cap reached for a run of `dot`, if any. Nothing is read for a cap not set.
    pub async fn hit(&mut self, store: &Store, dot: &Dot) -> Result<Option<CapHit>> {
        let caps = dot.spec.spend_cap;
        let dot_today = if caps.is_empty() {
            Spend::default()
        } else {
            self.dot_today(store, &dot.id).await?
        };
        let all_today = match (self.global.has_daily(), self.all_today) {
            (false, _) => Spend::default(),
            (true, Some(s)) => s,
            (true, None) => {
                let s = store.spend_between(None, &self.day.0, &self.day.1).await?;
                self.all_today = Some(s);
                s
            }
        };
        let all_month = match (self.global.monthly_usd.is_some(), self.all_month) {
            (false, _) => Spend::default(),
            (true, Some(s)) => s,
            (true, None) => {
                let s = store
                    .spend_between(None, &self.month.0, &self.month.1)
                    .await?;
                self.all_month = Some(s);
                s
            }
        };
        Ok(first_hit(
            &dot.spec.name,
            &caps,
            dot_today,
            &self.global,
            all_today,
            all_month,
        ))
    }

    /// `hit` for the dot `dot_id`; `None` for a dot that is gone.
    pub async fn hit_for(&mut self, store: &Store, dot_id: &str) -> Result<Option<CapHit>> {
        match store.get_dot(dot_id).await {
            Ok(dot) => self.hit(store, &dot).await,
            Err(Error::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Today's spend against the caps, for Settings, the Usage page and the dot editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpendToday {
    /// Local midnight today and tomorrow, RFC 3339 UTC.
    pub day_start: String,
    pub day_end: String,
    /// Local midnight of the 1st of this month.
    pub month_start: String,
    pub today: Spend,
    pub month: Spend,
    pub caps: GlobalCaps,
    /// Each dot with a cap or with a run today, by name.
    pub dots: Vec<DotSpend>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DotSpend {
    pub dot_id: String,
    pub dot_name: String,
    pub today: Spend,
    pub caps: DotCaps,
}

/// Today's spend of every dot, at `now`.
pub async fn today(store: &Store, now: DateTime<Utc>) -> Result<SpendToday> {
    let settings = store.get_settings().await?;
    let (day_start, day_end) = day_bounds_in(now, &Local);
    let (month_start, month_end) = month_bounds_in(now, &Local);
    let by_dot: HashMap<String, Spend> = store
        .spend_by_dot(&day_start, &day_end)
        .await?
        .into_iter()
        .collect();
    let mut dots = Vec::new();
    for dot in store.list_dots().await? {
        let spent = by_dot.get(&dot.id).copied();
        if spent.is_none() && dot.spec.spend_cap.is_empty() {
            continue;
        }
        dots.push(DotSpend {
            dot_id: dot.id,
            dot_name: dot.spec.name,
            today: spent.unwrap_or_default(),
            caps: dot.spec.spend_cap,
        });
    }
    Ok(SpendToday {
        today: store.spend_between(None, &day_start, &day_end).await?,
        month: store.spend_between(None, &month_start, &month_end).await?,
        caps: (&settings).into(),
        day_start,
        day_end,
        month_start,
        dots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_groups_by_three() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(200_000), "200,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn caps_are_checked_in_bounds() {
        assert!(check_usd(None).is_ok());
        assert!(check_usd(Some(0.01)).is_ok());
        assert!(check_usd(Some(100_000.0)).is_ok());
        for bad in [0.0, -1.0, 0.001, 100_000.5, f64::NAN, f64::INFINITY] {
            assert!(check_usd(Some(bad)).is_err(), "{bad}");
        }
        assert!(check_tokens(Some(1)).is_ok());
        assert!(check_tokens(Some(0)).is_err());
        assert!(check_tokens(Some(MAX_CAP_TOKENS + 1)).is_err());
    }
}
