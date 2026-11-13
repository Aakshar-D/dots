# Open Dot

Open Dot is the voice assistant inside Dots. You talk to it and it talks back. It can tell you what needs you, read a dot's question, approve or deny a request, send a message to a dot, run a dot, start a new dot, and open a screen.

It is off until you turn it on. This page says how, what to say, and what it does with what you say.

## Turn it on

1. Open Settings, then the **Voice** tab.
2. Turn on **Open Dot**.
3. Dots downloads the speech files, about 258 MB. This happens once. They stay on this computer. A bar shows the progress and there is a **Cancel** button. If you cancel, or Dots quits, the next try carries on from where it stopped.
4. When the status line says **Ready**, Dot says "Hi, I'm Dot." once.

If the system has blocked the microphone, the Voice tab says so. See Troubleshooting.

## Start a conversation

Any of these opens the stage, a small panel at the bottom of the window, and plays a soft tone:

- Press the hotkey. The default is Ctrl+Shift+Space (Cmd+Shift+Space on a Mac). You can change it in Settings, Voice, Listening, or clear it. If the Dots window is hidden, the hotkey brings it forward.
- Click the Open Dot mark at the bottom of the rail.
- In the tray menu, choose **Talk to Dot**. It is available once Open Dot is ready.
- Say "Hey Dot". This is off until you turn on **Hey Dot** in Settings. See Privacy.

You do not press a button to finish. Dot notices when you stop talking. After it answers, it listens a few more seconds so you can follow up. Esc closes the stage.

## What you can say

| You say something like | Dot does | Does it ask first? |
|---|---|---|
| "What needs me?" or "What's going on?" | Says how many things need you, then the first ones. | No |
| "Read that question." or "Tell it yes, use the staging server." | Reads a dot's question, or sends your answer to it. | Only when it was unsure of your words. Then it reads them back. |
| "Approve it." or "Deny that." | Decides an approval. | Always. It says which dot, which tool and on what, then waits for a clear yes. |
| "Set up a dot that checks my inbox every morning." | Runs the setup agent, asks its follow-up questions aloud and shows the proposal. | Always, before it creates the dot. |
| "Ask the research dot to compare the three vendors." | Sends a message to that dot as a new topic. Say "here" to send it into the topic you are looking at. | Not when you named the dot clearly. Then it says what it sent and to whom. If it was not sure which dot, it asks first. |
| "Run the backup dot now." | Starts a run. | Always. |
| "Open the research dot." or "Show settings." | Opens that screen. It can open a dot, Settings, the Inbox, Usage and Home. | No |
| "Stop." or "Never mind." or "Cancel." | Stops speaking and drops the step it was waiting on. | No |
| "Say that again." | Repeats its last reply. | No |

If Dot does not know which dot you mean, it asks and offers the closest names as numbered options. You can say the number or the name, or click one. If you ask for something it cannot do, it says so in one sentence.

A private approval cannot be decided by voice. Dot only counts it and opens it on screen.

## Confirmations

Before it approves, denies, runs a dot or creates a dot, Dot asks a question. The question is built from the real record, not from the model's words. Examples:

- "Research wants to use Bash on git push origin main. Approve this request?"
- "Run Backup now?"
- "Message for Research: compare the three vendors. Send this message?"

A long or cut-off command is not read out. Dot says it is on screen, and you look.

What counts as a yes:

- yes, yeah, yep, yup, yes please, yes do it, yes yes, yes go ahead
- do it, go ahead, go for it, please do
- confirm, absolutely
- A click on **Yes**.

Each kind of step also takes its own word. For an approval: "approve" or "approve it". For a run: "run it". For creating a dot: "create it" or "make it". For sending a message: "send it" or "send". For sending an answer: "send it" or "send that". For a deny question, "deny", "deny it" or "deny that".

"Okay", "sure", "right" and "correct" are not enough. Speech recognition often turns noise into those words, and people say them while still thinking. If you say one, Dot says "I need a clear yes for that. Say yes, or no." and the step stays. A second weak answer drops the step. "Go ahead" is also not enough for a deny question, because it can mean "let it go ahead".

What counts as a no: no, nope, nah, don't, do not, no thanks, no thank you, deny, deny it, deny that, not now, leave it. A click on **No** also counts. A no ends the step.

A spoken number ("one", "the first") never counts as yes or no. Any other reply drops the step and Dot treats your words as a new request.

A step waits 60 seconds. If you say "say that again", the wait starts again, at most twice. After three waits in all, 180 seconds, it is dropped. A question left unanswered for 20 seconds is answered "No answer, so I did nothing."

A yes counts only for a question Dot said to its end. If you stop Dot or talk over it before it has finished asking, the step is dropped and a yes does nothing. Ask again.

Just before it acts, Dot checks that the approval, dot or question is still there. If it is not, it says "That one is already handled." or "I can't find that dot anymore." and does nothing.

## Stopping and interrupting

- Esc closes the stage and stops any speech. Work already started in a dot carries on.
- The hotkey, or a click on the animated mark in the stage (The Breath), stops speech at once and listens.
- Say "Dot, stop" or "Hey Dot" while Dot is speaking. On headphones this works well. On speakers it is a best effort, because Dot's own voice can drown yours. The hand check has you count how often it works. The key and the click always work.
- Setting **Interrupt by just talking** (off by default): any speech stops Dot. It is meant for headphones. On speakers Dot's own voice would stop it.

Dot listens for at most five follow-ups in a row. Then it rests until you use the hotkey, click the mark or say "Hey Dot".

## Privacy

- Speech is recognised and spoken on this computer. Audio is never sent anywhere.
- No audio and no transcript is saved. What Dot heard and said is kept in memory for the conversation and dropped when you close the stage or after 10 minutes without a turn. Logs say that a turn happened and how long it took, not what was said.
- To understand a request, Dot sends the model your words and a short picture of now: your dots, the pending approvals and the open questions. The request goes through your own Claude CLI sign-in. Nothing else is sent. Vault values are never in the picture.
- A private approval is only counted ("one private approval needs you"). Its tool and target are not sent, not read aloud, and it opens on screen.
- With **Hey Dot** on, the microphone stays open while Dots is running, so the phrase can be heard. Only the on-device phrase detector hears it until the phrase is said. Whenever the microphone is open, the rail mark shows it and the tray tooltip says "Dot is listening".

Each request to the model is recorded in the Usage tab under "Open Dot" and counts against your spending caps. At a cap Dot says so and stops asking the model. Stop, cancel, yes, no and picking an option still work.

## Settings

All in Settings, Voice.

| Setting | What it does |
|---|---|
| **Open Dot** | Turns Open Dot on or off. Turning it on downloads the speech files the first time. |
| **Voice** | Ten voices, Voice 1 to Voice 10, each with a play button. The play buttons work once Open Dot is ready. |
| **Speaking speed** | A slider from 0.8 to 1.4. The default is 1.0. |
| **Tell me aloud when something needs me** | Off by default. Dot says a short line when an approval or a question needs you. It never says more than one line in 30 seconds. It speaks only while Dot is resting, and does not announce the dot you are looking at. |
| **Hotkey** | Default Ctrl+Shift+Space (Cmd+Shift+Space on a Mac). Write names like Ctrl, Shift, Alt and a key, joined with +. **Clear** removes it. |
| **Hey Dot** | Off by default. Lets you start by saying "Hey Dot". The microphone stays on while Dots runs. |
| **Interrupt by just talking** | Off by default. Best with headphones. |
| **Play a tone when Dot starts listening** | On by default. |
| **Microphone** and **Speaker** | Choose a device, or System default. |
| **Which model thinks** | Fast (the default), Balanced or Most capable. Fast is the smallest model and the quickest. |
| **Remove speech files** | Shown once the files are downloaded. Turns Open Dot off and deletes the files. Turning it on again downloads them again. |

## How fast is it

Dot answers short things itself, with no model: stop, cancel, repeat, yes, no, and picking an option. Anything else goes to the model.

One measurement, on one computer, with the Fast model (haiku):

- Dots' own time is about 1.3 seconds from the end of your sentence to the first sound. About 0.7 seconds of that is the quiet that tells Dot you finished.
- With the real model, "One moment." came about 3 seconds after you stopped talking. It is said if nothing has been said 1.5 seconds after the request starts.
- The answer itself was first heard about 6 seconds after you stopped talking (median 5.7 seconds, range 4.5 to 7 seconds).

That is slower than the two-second goal. Most of the time is the Claude CLI starting and the model answering. Your own results will vary with the computer, the network and the model you pick.

## Troubleshooting

**The microphone is blocked.** Dot says "I can't open the microphone. Check that one is connected and that Dots may use it." The Voice tab shows the reason. Allow Dots to use the microphone in the system's privacy settings (Windows: Settings, Privacy and security, Microphone; macOS: System Settings, Privacy and Security, Microphone), then press the hotkey again. If you pick a microphone in Settings, check it is the one that is plugged in.

**Dot says it can't hear anything.** It heard only silence for a whole listen after you pressed the hotkey, clicked or said "Hey Dot". Check the microphone is not muted and that Dots may use it.

**The hotkey does nothing, or the Voice tab shows a hotkey error.** Another program probably holds that key combination. Type a different one in Hotkey. You can also use the rail mark or the tray's **Talk to Dot**.

**The download stopped.** Turn Open Dot on again, or press the hotkey. It picks up where it left off. If the network was down, Dot says "The download stopped. It picks up where it left off the next time you open Open Dot." and stays on.

**There is no sound.** Check the Speaker setting and your system volume. Play a voice in Settings. If a speaker or headphones were just plugged in or unplugged, choose the device again. If the tone and speech are both missing, Dot cannot tell you by voice; look at the stage and the Voice tab.

**Dot heard itself on speakers.** Dots has no echo cancellation. Use headphones, or turn down the speaker, and leave **Interrupt by just talking** off.

**"Hey Dot" was missed.** It is a small on-device detector and it will miss sometimes, more in a noisy room, from far away or with music playing. Say it a little slower, or use the hotkey. "Hey Dot" must be turned on in Settings, and the microphone must be allowed. Say "Hey Dot" and then wait a moment, or carry straight on with your request in the same breath.

**Dot says "Sorry, I didn't catch that."** It heard sound it could not make a sentence of (noise can come out as one word over and over). Say it again.
