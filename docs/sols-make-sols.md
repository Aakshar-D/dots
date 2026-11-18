# Sols that make sols

A sol can make other sols. It can change or delete the ones it made, and give them jobs. It does this by itself, but only inside what it may touch itself. For anything more, Sage asks you on a card.

This is off for every sol until you turn it on.

## Turn it on

1. Open the sol's editor.
2. Find the section **Making sols**.
3. Turn on **Can make sols**.
4. Two limits show. Change them if you want, then **Save**.

| Limit | Default | Range |
|---|---|---|
| Sols it may make in one run | 3 | 1 to 10 |
| Sols it may keep in total | 10 | 1 to 50 |

A sol with the switch on is called a maker. A sol it makes is a made sol.

A made sol cannot make sols. You can turn the switch on for a made sol yourself. It then has more than its maker, so Sage asks you before its maker gives it a job or changes it.

A maker's run that is a step of a workflow does not get the making tools. If the sol's permissions deny `MakeDots`, it does not get them either. The editor says so.

## What a maker may do without asking

A maker may make a sol, change it, delete it, and give it a job, as long as the made sol gets nothing the maker does not have. That means:

- The same account, workspace and engine as the maker. The maker cannot choose these.
- A new empty folder, the maker's own folder, or a folder below it.
- The maker's own permissions, or a weaker preset.
- Library items the maker uses.
- Variables the maker is granted.
- The browser, only if the maker has it.
- Turn and time limits no higher than the maker's.
- Any schedule.

Taking things away from a made sol never asks.

What the made sols spend counts toward the maker's daily cap too.

## What a maker may not do without asking

Sage asks you when a maker wants:

- A sol with something the maker does not have: a Library item, a variable, a stronger preset, a folder outside its own, the browser, a higher limit.
- One more sol than a limit allows.
- A change to a made sol that gives it more than the maker has.
- A change or a job for a made sol that already has more than its maker. This happens when you allowed it more, gave it more yourself, or narrowed the maker later.
- To give back something you took away from a made sol.

Some things a maker can never get, and Sage does not ask: another account or workspace, another engine, or a made sol that can make sols.

A maker can only change, delete or give a job to a sol it made itself. You can still open, edit, run and delete any made sol like any other sol.

At most three requests of one maker wait at a time.

## The card

The card shows in the maker's conversation and in the Inbox. Sage also sends a notification.

It shows:

- What the maker asks.
- What lies beyond the maker's reach, one line each.
- The limit it passes, if any.
- The instructions, the model, and the first job. A sol wrote these, so they are set apart from Sage's own words and shown as plain text. Read them before you answer.

The buttons:

| Button | What it does |
|---|---|
| **Allow for this sol** | The sol is made, or changed, as asked. The extras belong to that sol only. The maker itself gets nothing more. |
| **Make it without these** (or **Change it without these**) | The sol is made, or changed, without what was beyond the maker's reach. Not offered when only a limit is passed. |
| **Don't make it** (or **Don't change it**) | Nothing is made or changed. |

A card for a job has **Allow the job** and **Don't give it**. It shows the job in full and says that the made sol's final message goes back to the maker.

After any answer, Sage tells the maker what you chose.

Your answer covers only what the card showed. If things change between the card and your answer, Sage refuses the answer, shows the card as it is now, and tells you it changed. Then you can answer again.

If something the request names has gone, such as a folder or a Library item, the card says what went. Only the refusal is left.

**Allow** may be off, with a line that says why. It needs the right to change sols in the maker's account, and the right to give variables when the sol gets any. Allowing a job needs the right to run the made sol. Refusing needs no right.

## In the conversation

In the maker's conversation you see, in the order things happen:

- **Made sol: name**, with what it is for, its schedule and what it uses. **Open sol** opens it.
- **name is working**, with the job, and **Open run**. Then **name finished**, **failed** or **was stopped**.
- **name is waiting for your answer**, when the made sol asked you a question. The maker is not told until the job ends.
- The card, when the maker asked for more.
- **maker changed name**, with what changed.
- **maker deleted name**.

When a job ends, Sage tells the maker. That shows in the maker's conversation as **Report from Sage**. In the made sol's conversation, a job shows as **Job from maker**. Neither is drawn as a message you typed.

A maker may give six jobs in a conversation between two messages you type. After that it has to report to you.

## In the sols list

- A maker's row says **can make sols · 2 of 10**: how many it made and how many it may keep.
- Made sols sit under their maker, stepped in, with **made by maker**.
- When a maker is deleted, its made sols stay. They move to the top level and say **made by maker (deleted)**. Its requests that waited for your answer are dropped.

A made sol's editor says **Made by maker** near the top, with a link to the maker.

## Deleting

A maker deletes a made sol for good, history included. There is no undo. While **Always ask before sending or deleting** is on in the maker's permissions, which it is in every preset, Sage asks you first on an approval card that names the sol.

A made sol with a run going or waiting cannot be deleted, by you or by its maker.

## Voice

With **Tell me aloud when a sol finishes** set to **What I started**, Open Sage does not announce a job a maker gave or a report Sage sent to a maker. You did not start those. Set to **Everything**, it announces them too.
