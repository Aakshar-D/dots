# Always-on workflows: being there when a workflow needs Sage

- Date: 2026-11-24
- Status: built (plan 5 of 5 of the workflows refresh), on top of plan 4 (`2026-11-23-plain-workflows-design.md`).

## 1. Why, and what this plan is not

A workflow that starts on a schedule, listens for Telegram messages or waits for a time only does so while Sage is running. The first sketch of this plan was a Sage that runs with no window as a background service on a machine that is always on.

Looking at what the app has changed that. Sage already starts when the person signs in, hidden in the tray (`autostart`, on by default), and closing its window hides it. So Sage is already there whenever the PC is on and the person is signed in. Plans 2 and 3 made the rest forgiving: a wait whose time passed ends when Sage next starts, a schedule can catch up, and Telegram holds unread messages for about a day.

What was still missing is small, and is what this plan builds:

- The person cannot see that anything depends on Sage being open, or whether it will be.
- The PC going to sleep on its own stops everything, and nothing could prevent it.

**Not built:** a service that runs before anyone signs in, or with the PC off, and a hosted Sage. A pipeline that must run on a machine nobody is signed in to needs one of those. It is a product decision with its own costs (where it runs, whose account it uses, how its approvals reach a person), not a step of this refresh.

## 2. What needs Sage open

The core counts what depends on Sage running (`workflows_standing`):

- workflows that are on, break no rule and start on a schedule;
- workflows that are on, break no rule and start on a Telegram message;
- runs at a wait step, with the time the soonest of them ends.

The Teams screen shows a note while any of these is above zero: what they are, when the next wait ends, whether Sage starts by itself at sign-in, whether the PC is kept awake, and a link to Settings. With nothing depending on Sage the note is not shown.

## 3. Keeping the PC awake

A setting, **Keep this PC awake while a workflow needs Sage** (`keep_awake`), off by default since it costs power.

- While it is on and section 2 counts anything, the PC does not go to sleep on its own for being idle. It still sleeps when the person tells it to or closes the lid, and the screen may still turn off.
- The loop that moves workflow runs decides after each pass, so the PC is let sleep again within a pass of the last schedule being turned off or the last wait ending.
- Windows: the thread execution state `ES_SYSTEM_REQUIRED`, held by one thread of its own, since the request belongs to the thread that made it. macOS: a `caffeinate -i` that runs for as long as it is wanted. Other systems: nothing.
- When Sage quits, the request goes with it.

## 4. Testing

- **Core:** the counts with a schedule that is on and one that is off, a listener, a workflow started by hand and a run at a wait; the keep-awake calls do not fail or block.
- **UI:** the note with nothing and with something depending on Sage, and its words for each setting.
- **Not tested:** that a PC really stays awake. Nothing here can observe the system's sleep. It is a hand check on Windows and on a Mac, and the Mac path has never run on a real Mac.

## 5. Known gaps

- No service before sign-in, with the PC off, or hosted: see section 1.
- A closed lid or a low battery still puts the PC to sleep.
- The note is on the Teams screen only.
- Keep-awake follows workflows only, not a sol's own schedule or watchers.
