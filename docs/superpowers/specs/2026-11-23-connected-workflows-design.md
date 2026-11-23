# Connected workflows: Telegram in and out, and what an action gives

- Date: 2026-11-23
- Status: built (plan 3 of 5 of the workflows refresh), on top of plan 2 (`2026-11-23-general-workflows-design.md`).
- Builds on: plan 2's action steps, marks, inputs and the team's own folder.

## 1. Why, and what this plan is not

The person's first example of a pipeline is "a Telegram and email pipeline for a client". Plan 2 gave workflows a step that sends an HTTP request, which already reaches any service with a web API: a mail service, a posting service, a chat's incoming hook. What it could not do is hear anything. A workflow started only by hand, from a chat, on a schedule, or from a webhook that nothing outside this PC can reach.

This plan gives workflows one channel that works both ways with no public address: Telegram. A bot's messages are fetched by asking Telegram for them, so nothing has to reach this PC.

**Scoped down from the first sketch**, each for a reason:

- **Mail in.** A workflow is not started by an arriving mail. The mail watcher wakes a sol, and only the person's own chat may start a workflow (teams spec section 7, decided 2026-10-24): letting text that arrives from outside start workflows through a sol would undo that. A trigger that starts a workflow from a mail directly, with no sol deciding, needs the watchers' cursors, back-off and flood cap moved to workflows. That is its own plan.
- **Mail out, and posting to social networks.** Not built as steps of their own. Both go through plan 2's HTTP action to a service's API, with the key as a secret. Plan 4 ships templates for them. A step that speaks SMTP itself, or each network's own API, is not built.
- **Calling a connector's tool with no model.** Not built: a Library connector's session is opened for a sol's run, not for a workflow.

## 2. A Telegram action

An action step gains a third kind, `telegram`: `token`, `chat`, `text`.

- `token` is the name of a Personal variable that holds the bot's token. The token is read when the step runs. It is named, never written: a `{{secret.…}}` mark in the chat or the message is refused.
- `chat` and `text` may hold marks. A chat that is a number goes to Telegram as a number; anything else as it is (a channel's `@name`).
- The step calls the bot API's `sendMessage`. It fails with Telegram's own words when Telegram refuses (`Telegram refused the message: Bad Request: chat not found`), when the message is over 4,096 characters, and when Telegram cannot be reached.
- It sends something out, so it needs a check before it unless `Runs without my check` is on, as an HTTP action does.
- A Test run sends nothing.
- Only a team in Personal reads the variable, as for every secret.

## 3. A Telegram trigger

A trigger gains a fifth kind, `telegram`: `token` (the variable's name), `chats` (ids), `project` (blank: the team's own folder).

- **Only the chats it names start it.** Anyone can write to a bot, so a message from any other chat is read past and starts nothing. A trigger needs at least one chat, and a chat's id is a number (a group's has a minus).
- **Finding a chat's id.** The trigger's settings ask the bot which chats it heard from lately (`Find chats`) and offer each by name. The person writes to the bot from the chat, then picks it. Looking takes no message away.
- **What a run gets.** The message's text is the run's task. A workflow that asks for inputs named `chat_id`, `sender` or `message_id` is given them, so a Telegram action answers the same chat with `{{input.chat_id}}`.
- **Fetching.** One loop asks every 5 seconds, once per bot, while Sage is open. Workflows that name the same variable share the bot and each start on a message from a chat they listen to.
- **At most once.** Where a bot's messages were read up to is kept by the variable's name (`telegram_offsets`, migration 0063), and moved past a message before its runs start. A crash between the two loses that message's run rather than starting it twice.
- **Not too many.** A workflow with 3 runs in flight (the webhook's cap) leaves the next message where it is: it starts a run on a later pass. Telegram keeps a bot's unread messages for about a day.
- A workflow that is off or has a problem does not listen. A team of an account does not listen: its variables are not read here.
- Messages with no text (a photo with no caption, a join, an edit) are read past.

## 4. What an action gives

A later action can use what an earlier action did, as `{{<step id>.<name>}}`:

| Action | Gives |
|---|---|
| `http` | `status` (the answer's number) and `answer` (its body, up to 20,000 characters) |
| `telegram` | `answer` (Telegram's answer, as JSON text) |
| `file` | `path` |

So a request can use the id an earlier request got back. In a Test run nothing is sent, and `status` and `answer` are empty. No secret is kept in them.

## 5. The UI

- **The trigger's sheet** offers `On a Telegram message`: the bot's variable, the chats it listens to (typed by id, or found from the bot), and the project.
- **An action's sheet** offers `Send a Telegram message` first: the bot's variable, the chat, the message, `Runs without my check`, and the marks its texts may hold. The marks now list what earlier actions gave.
- Runs a message started read `On a Telegram message` wherever a run's trigger is named.

## 6. Testing

- **Core:** a Telegram action against a local server that answers as the bot API does (the call as made, the chat as a number, Telegram's refusal, a Test run, the answer used by a later step); a pass that starts one run for a message from a listened chat, reads past a stranger's and an edit, keeps the offset and starts nothing twice; a workflow that is off and a bot with no token; the chats offered; the rules of the trigger and the action; which marks name an earlier action's values.
- **UI:** the trigger's fields and what a save sends, a failure to find chats, the action's fields and marks.
- **Not tested:** the real Telegram. Every test runs against a local stand-in built from the bot API's documented shapes.

## 7. Known gaps

- Mail in, mail out as its own step, social posting as its own step, and calling a connector's tool with no model: see section 1.
- Messages arrive only while Sage is open. Telegram holds unread ones for about a day; older ones are lost.
- A message over 4,096 characters is refused, not split.
- Only text is read and sent: no photos, files, buttons or formatting.
- A bot can be listened to by one PC: two PCs asking for the same bot's messages take them from each other.
- A message whose runs could not start (the workflow's project is gone, its start was refused) is not tried again.
