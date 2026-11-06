# Spending caps

Part of "Permissions you control": the person can cap what dots spend. Before this, a run's
cost was recorded (`runs.cost_usd`, from the engine's `Usage` events) but limited nothing.

## 1. Caps

- **Per dot** (dot editor, Spending caps): a daily cost cap in USD and a daily token cap.
  Stored as `dots.spend_daily_usd` and `dots.spend_daily_tokens` (migration 0039); in the
  spec as `spend_cap: { daily_usd, daily_tokens }`.
- **Every dot together** (Settings, Runs & server tab): a daily cost cap, a daily token cap
  and a monthly cost cap (`spend_daily_usd`, `spend_daily_tokens`, `spend_monthly_usd`).
- Empty (null) is no cap; that is the default. A cost cap is $0.01 to $100,000; a token cap
  is 1 to 10,000,000,000. The core checks this (`DotSpec::validate`, `Settings::validate`)
  and the UI shows the same rule beside the field before a save.

## 2. What counts

- A day runs from **local midnight** to the next local midnight (`delivery::local_day_bounds`:
  on a day when the clocks change it is 23 or 25 hours long). A month runs from local midnight
  on the 1st to local midnight on the next 1st.
- The spend of a period is the sum over the runs **started** in it (`runs.started_at`) of
  their `cost_usd` and of `tokens_in + tokens_out` (the Usage page's tokens, cache reads
  included). A run that crosses midnight counts entirely to the day it started.
- A run still going counts with what it has reported so far: `Store::add_usage` adds each
  `Usage` event to its row as it arrives. A queued run has spent nothing.
- Cost is **estimated**: it is what the run would cost on the API, as the engine reports it.
  A Claude subscription bills no run, and a local engine reports no cost (its tokens still
  count). The UI says "estimated" with that explanation.

## 3. Before a run starts

The dispatcher reads the caps once per pass (`spend::Ledger`). A queued run whose dot has
reached a cap (its own daily cap, or a cap for every dot) is not started: it **waits**, never
fails. The order the caps are checked in, and the words for each:

1. "Scout reached its $2.00 daily cap"
2. "Scout reached its 200,000-token daily cap"
3. "All dots reached the $10.00 daily cap"
4. "All dots reached the 1,000,000-token daily cap"
5. "All dots reached the $50.00 monthly cap"

A cap is reached when the spend is at it or over. The run starts on its own once the period
rolls over (the dispatcher looks every 2 seconds) or the cap is raised or cleared (saving a
dot or Settings wakes the dispatcher).

- The wait reason is `spend_cap`, with the words above in `Run::wait_detail`. It comes after
  the usage hold and before an update, a busy dot or busy slots.
- The delivery sweeper leaves a capped run alone, as it does a usage hold: the wait is
  deliberate, so the run is never marked not delivered for it.
- Every trigger waits: chat, Run now, resumes, workflow steps and background runs.

## 4. The person's own runs: Raise cap / Run once anyway

A run the person started (chat, Run now, a resume, an app or workflow run: anything that is
not schedule, watcher, follow-up, webhook or API) that waits for a cap shows in the Inbox's
Waiting to run section and under its message in the conversation, with:

- **Raise cap**: opens the dot's editor (its own cap) or Settings › Runs & server (a cap for
  every dot).
- **Run once anyway** (`run_release_cap`): marks the run `cap_released` (migration 0039) and
  notes "Run once past the cap: …" on the run. That run starts, and no cap stops it while it
  runs. The next run waits again.

A background run that waits for a cap is listed in the Inbox too, with Raise cap only:
Run once anyway refuses it ("A background run waits for the cap: raise the cap or wait for
tomorrow.").

## 5. While a run runs

After each `Usage` event the runner reads the caps again for the run's dot. When one is
reached, the run is stopped half a second later, the way the time limit and the quiet-limit
watchdog stop a run (cancel, then the grace period), and ends **failed** with the error
"stopped: Scout reached its $2.00 daily cap". Its pending approvals expire.

The half second is there because the Claude CLI reports usage only with its result: the
usage that reaches the cap usually comes with the run's own end. A run that finishes in that
half second ends as it was ending (succeeded), with the notice "Scout reached its $2.00
daily cap: the next run waits". So a cap may be overshot by the run that reaches it; it
holds back the runs after it. A run let past the cap once is never stopped by one.

## 6. Showing spend

`spend_today` answers today's spend and this month's, the caps for every dot, and each dot
that has a cap or spent something today. Any run update marks it stale.

- Settings › Runs & server › Spending caps: today against the daily caps, this month against
  the monthly cap.
- The dot editor: the dot's spend today against its own caps.
- The Usage page: today and this month for every dot, and each dot with a cap of its own.

Each line warns (amber, "Over 80% of the cap") from 80% of a cap and turns red with "Cap
reached" at the cap.

## 7. Not covered

- No notification when a cap is reached; the Inbox lists the runs it holds.
- Caps are not per account or workspace; Settings' caps cover every dot on this PC.
- The cost a CLI reports for a subscription run is taken as it is; Dots does not price
  tokens itself.
