# Open Sage

Open Sage is the voice assistant inside Sage. You talk to it and it talks back. It can tell you what needs you, read a sol's question, approve or deny a request, send a message to a sol, run a sol, start a new sol, and open a screen.

It is off until you turn it on. This page says how, what to say, and what it does with what you say.

## Turn it on

1. Open Settings, then the **Voice** tab.
2. Turn on **Open Sage**.
3. Sage downloads the speech files, about 258 MB. This happens once. They stay on this computer. A bar shows the progress and there is a **Cancel** button. If you cancel, or Sage quits, the next try carries on from where it stopped.
4. When the status line says **Ready**, Sage says "Hi, I'm Sage." once.

If the system has blocked the microphone, the Voice tab says so. See Troubleshooting.

## Start a conversation

Any of these opens the stage, a small panel at the bottom of the window, and plays a soft tone:

- Press the hotkey. The default is Ctrl+Shift+Space (Cmd+Shift+Space on a Mac). You can change it in Settings, Voice, Listening, or clear it. If the Sage window is hidden, the hotkey brings it forward.
- Click the Open Sage mark at the bottom of the rail.
- In the tray menu, choose **Talk to Sage**. It is available once Open Sage is ready.
- Say "Hey Sage". This is off until you turn on **Hey Sage** in Settings. See Privacy. If you then wait, Sage says "How can I help?" and listens. If you carry straight on with your request, it just listens. Saying "Hey Sage" again while it already listens is not taken as your request.

You can say a lot in one go: Sage listens to a message of up to three minutes and takes all of it. If you stop for more than a second or two it takes what you said so far, but when you go on within about two seconds, before it has said anything back, it drops the answer it was working on and takes everything you said as one message. Only real speech counts as going on, about half a second of it: a breath, a cough or a noise in the room does not take the answer back, so once you stop, Sage knows you are done. It answers in a sentence or three by default, and at more length (up to about eight sentences) when you ask it to explain, sum up or draft something. You can also just talk to it or ask it general questions.

You do not press a button to finish. Sage notices when you stop talking: it waits a little over a second of quiet, and about two seconds when your words so far stop in the middle of a thought ("open the research sol and..."), so you can pause to think. After it answers, it listens a few more seconds so you can follow up. Esc closes the stage. So does "thank you", "goodbye" or "close".

**Enlarge** in the stage's bottom row makes it fill the window. Enlarged, it shows the whole conversation, every reply in full, in text you can select. Each reply has a copy button and **Copy all** copies the lot. The enlarged stage does not close by itself: it stays until you press Esc or **Close**, and the conversation shown goes with it. **Smaller** brings the small panel back. Sage remembers which size you chose.

 A list Sage showed stays 10 seconds after it rests, and goes at once when you say the next thing.

## What you can say

| You say something like | Sage does | Does it ask first? |
|---|---|---|
| "What needs me?" or "What's going on?" | Says how many things need you, then the first ones. | No |
| "Read that question." or "Tell it yes, use the staging server." | Reads a sol's question, or sends your answer to it. | Only when it was unsure of your words. Then it reads them back. |
| "Approve it." or "Deny that." | Decides an approval. | Always. It says which sol, which tool and on what, then waits for a clear yes. |
| "Set up a sol that checks my inbox every morning." | Runs the setup agent, asks its follow-up questions aloud and shows the proposal. | Always, before it creates the sol. |
| "Ask the research sol to compare the three vendors." | Sends a message to that sol as a new topic. Say "here" to send it into the topic you are looking at. | Not when you named the sol clearly. Then it says what it sent and to whom. If it was not sure which sol, it asks first. |
| "Run the backup sol now." | Starts a run. | Always. |
| "Open the research sol." or "Show settings." | Opens that screen. It can open a sol, Settings, the Inbox, Usage, Home, Connectors, Apps, the Library, Teams and Intents. | No |
| "Check my mail for anything from the bank." | Sage has no connector of its own. It sends the request to a sol that uses that connector (it knows which sols use which). If none does, or the connector is not connected, it says so and opens Connectors. | As for any message to a sol |
| "Build me an app that tracks my invoices." | Apps are built by sols. Sage sends a sol the request, asking which sol when you named none, and tells you the app will show up under Apps for you to review. | As for any message to a sol |
| "Make a note: buy milk and call the bank." or "Write that in Notepad." | Writes a new file in **Sage Notes** in your Documents folder and opens it: Markdown, or a plain text file when you ask for Notepad or a text file. It never writes over a file. | No |
| "Open github dot com." | Opens the site's front page in your browser. | No |
| "Search for flights to Lisbon." | Opens a web search in your browser. | No |
| "Open the calculator." or "Open my Downloads." | Opens an app or folder: Notepad, Calculator, your files, a terminal, Paint, the system settings, your browser, Documents, Downloads, Desktop, Pictures or your notes folder. | No |
| "Remember that I like short answers." or "Call the research sol Scout." | Keeps one short note about you for later conversations and says "I'll remember that." | No |
| "Forget what you know about me." | Forgets every note and what it counted about how you talk. | No |
| "Close Notepad." | Asks the app to close, as its close button does, so it still asks about unsaved work. Works for Notepad, Calculator, Paint, the terminal and the system settings. | No |
| "Close Chrome." or "Make a folder called Invoices on my desktop." or "Turn the volume down." | Anything else on your computer: Sage writes one command for it, shows it on screen and says what it will do. | Always. It runs only on a clear yes. |
| "Stop." or "Never mind." or "Cancel." | Stops speaking and drops the step it was waiting on. | No |
| "Say that again." | Repeats its last reply. | No |
| "Thank you." or "Goodbye." or "Close." or "Thank you, I'll call you later." | Says nothing, drops the step it was waiting on and closes the stage. | No |

If Sage does not know which sol you mean, it asks and offers the closest names as numbered options. You can say the number or the name, or click one. If you ask for something it cannot do, it says so in one sentence.

A private approval cannot be decided by voice. Sage only counts it and opens it on screen.

### What it does on your computer, and what it does not

Sage writes notes, opens sites and web searches, and opens and closes a fixed list of apps and folders without asking first. Each of those is kept harmless:

- A note is always a new file in **Sage Notes**. Nothing is changed or written over.
- A site is opened at its front page, by name. If more of an address was asked for, Sage opens the front page and says so. Ask for a search to reach a page inside a site.
- Apps are opened from a fixed list. Sage never runs a command or a program by a name the model gives.

- Closing asks the app to close. Nothing is forced, so an app with unsaved work still asks you.

**Anything else is a command, and a command always asks first.** When you ask for something those cannot do (close another program, make or move files, change the volume, start something), Sage has the model write one command for your system's shell: PowerShell on Windows, zsh on a Mac. It puts the command on screen, says in a sentence what it is meant to do, and asks "Run this command?". It runs only on a clear yes ("yes", "do it", "go ahead"); "okay" is not enough, and saying anything else drops it.

Read the command before you say yes. The sentence about it is the model's own and can be wrong; what runs is what is on screen. It runs as you, with everything you are allowed to do, for at most a minute, and what it prints is shown afterwards. Sage does not undo it. For work that takes several steps, or that you want checked as it goes, ask a sol: it works with approvals.

## Confirmations

Before it approves, denies, runs a sol or creates a sol, Sage asks a question. The question is built from the real record, not from the model's words. Examples:

- "Research wants to use Bash on git push origin main. Approve this request?"
- "Run Backup now?"
- "Message for Research: compare the three vendors. Send this message?"

A long or cut-off command is not read out. Sage says it is on screen, and you look.

What counts as a yes:

- yes, yeah, yep, yup, yes please, yes do it, yes yes, yes go ahead
- do it, go ahead, go for it, please do
- confirm, absolutely
- A click on **Yes**.

Each kind of step also takes its own word. For an approval: "approve" or "approve it". For a run: "run it". For creating a sol: "create it" or "make it". For sending a message: "send it" or "send". For sending an answer: "send it" or "send that". For a deny question, "deny", "deny it" or "deny that".

"Okay", "sure", "right" and "correct" are not enough. Speech recognition often turns noise into those words, and people say them while still thinking. If you say one, Sage says "I need a clear yes for that. Say yes, or no." and the step stays. A second weak answer drops the step. "Go ahead" is also not enough for a deny question, because it can mean "let it go ahead".

What counts as a no: no, nope, nah, don't, do not, no thanks, no thank you, deny, deny it, deny that, not now, leave it. A click on **No** also counts. A no ends the step.

A spoken number ("one", "the first") never counts as yes or no. Any other reply drops the step and Sage treats your words as a new request.

A step waits 60 seconds. If you say "say that again", the wait starts again, at most twice. After three waits in all, 180 seconds, it is dropped. A question left unanswered for 20 seconds is answered "No answer, so I did nothing."

A yes counts only for a question Sage said to its end. If you stop Sage or talk over it before it has finished asking, the step is dropped and a yes does nothing. Ask again.

Just before it acts, Sage checks that the approval, sol or question is still there. If it is not, it says "That one is already handled." or "I can't find that sol anymore." and does nothing.

## What Sage learns about you

Sage gets to know you as you talk to it, so that it answers the way you like and guesses better what you mean.

- **Notes.** When you tell it how you like to be answered, what you call things or what you are working on, it keeps one short sentence about that. It keeps at most 30; the oldest goes when one more is learned.
- **Habits.** It counts how many words you usually say at a time and which kinds of things you ask for (seeing what needs you, messaging a sol, opening a screen and so on). After five requests it tells the model, for example, that you like it brief and what you ask for most.

It never keeps what you said, only those notes and counts. They are in `open-sage-memory.json` in Sage's data folder on this computer, and they go to the model at the start of each request.

In Settings, Voice, **What Sage has learned** lists the notes. Remove any one with its ✕, or **Forget everything**. Turn off **Learn how I talk and what I need** and Sage keeps nothing more and sends nothing it learned; what it had learned stays in the list until you forget it.

## Stopping and interrupting

- Esc closes the stage and stops any speech. Work already started in a sol carries on.
- The hotkey, or a click on the animated mark in the stage (The Breath), stops speech at once and listens.
- Say "Sage, stop" or "Hey Sage" while Sage is speaking. Say "Sage, stop" in one go: with a pause after "Sage" it is often missed. On headphones this works well. On speakers it is a best effort, because Sage's own voice can drown yours. The hand check has you count how often it works. The key and the click always work.
- Setting **Interrupt by just talking** (off by default): any speech stops Sage. It is meant for headphones. On speakers Sage's own voice would stop it.

Sage listens for at most five follow-ups in a row. Then it rests until you use the hotkey, click the mark or say "Hey Sage".

## Privacy

- Speech is recognised and spoken on this computer. Audio is never sent anywhere.
- **Teach Sage your voice** keeps no sound. The five takes are held in memory for the few seconds it takes to check them. What is stored is the setting it picked, how loud your voice was, and how many takes it heard.
- What Sage learns about you (short notes and counts, never what you said) is kept on this computer and sent to the model with your requests. See "What Sage learns about you"; you can turn it off.
- No audio and no transcript is saved. What Sage heard and said is kept in memory for the conversation and dropped when you close the stage or after 10 minutes without a turn. Logs say that a turn happened and how long it took, not what was said.
- To understand a request, Sage sends the model your words and a short picture of now: your sols, the pending approvals, the open questions, the names of your connectors with whether each is connected and which sols use it, and the names of the apps in the workspace. The request goes through your own Claude CLI sign-in. Nothing else is sent. Vault values are never in the picture.
- A private approval is only counted ("one private approval needs you"). Its tool and target are not sent, not read aloud, and it opens on screen.
- With **Hey Sage** on, the microphone stays open while Sage is running, so the phrase can be heard. Only the on-device phrase detector hears it until the phrase is said. Whenever the microphone is open, the rail mark shows it and the tray tooltip says "Dot is listening".

Each request to the model is recorded in the Usage tab under "Open Sage" and counts against your spending caps. At a cap Sage says so and stops asking the model. Stop, cancel, yes, no and picking an option still work.

## Settings

All in Settings, Voice.

| Setting | What it does |
|---|---|
| **Open Sage** | Turns Open Sage on or off. Turning it on downloads the speech files the first time. |
| **Voice** | Ten voices, Voice 1 to Voice 10, each with a play button. The play buttons work once Open Sage is ready. |
| **Speaking speed** | A slider from 0.8 to 1.4. The default is 1.0. |
| **Tell me aloud when something needs me** | Off by default. Sage says a short line when an approval or a question needs you. It never says more than one line in 30 seconds. It speaks only while Sage is resting, and does not announce the sol you are looking at. |
| **Hotkey** | Default Ctrl+Shift+Space (Cmd+Shift+Space on a Mac). Write names like Ctrl, Shift, Alt and a key, joined with +. **Clear** removes it. |
| **Hey Sage** | Off by default. Lets you start by saying "Hey Sage". The microphone stays on while Sage runs. |
| **Teach Sage your voice** | Under **Hey Sage**. You say "Hey Sage" five times, about three seconds apart, from where you usually sit. Sage sets the phrase up for how you sound at your microphone, starts its microphone boost at your level, and tells you how many of the five it heard. It takes about 20 seconds. No recording is kept: only the setting, your voice level and the count, in `voice-setup.json` beside the speech files. Do it again whenever you move or change microphone. |
| **Interrupt by just talking** | Off by default. Best with headphones. |
| **Play a tone when Sage starts listening** | On by default. The tone plays for the hotkey, the rail mark and the tray. After "Hey Sage" and a pause Sage says "How can I help?" instead; with this off it says nothing and just listens. |
| **Microphone** and **Speaker** | Choose a device, or System default. |
| **Which model thinks** | Fast (the default), Balanced or Most capable. Fast is the smallest model and the quickest. |
| **Remove speech files** | Shown once the files are downloaded. Turns Open Sage off and deletes the files. Turning it on again downloads them again. |

## How fast is it

Sage answers short things itself, with no model: stop, cancel, repeat, yes, no, and picking an option. Anything else goes to the model.

One measurement, on one computer, with the Fast model (haiku):

- Sage's own time is about 1.3 seconds from the end of your sentence to the first sound. That was measured when 0.7 seconds of quiet told Sage you had finished. It now waits 1.1 seconds, so add 0.4 seconds.
- With the real model, "One moment." came about 3 seconds after you stopped talking. It is said if nothing has been said 1.5 seconds after the request starts.
- The answer itself was first heard about 6 seconds after you stopped talking (median 5.7 seconds, range 4.5 to 7 seconds).

Since then Sage starts the Claude CLI the moment you pause, about a second before it knows your sentence is over, so the CLI is ready when the sentence is. Measured on one computer with the Fast model: the first words of the answer came 1.6 to 1.7 seconds after the request with a CLI started ahead, against 2.1 to 2.9 seconds without. If you go on speaking, the CLI started for nothing is ended; no request was made, so it costs nothing.

That is slower than the two-second goal. Most of the time is the Claude CLI starting and the model answering. Your own results will vary with the computer, the network and the model you pick.

## Troubleshooting

**The microphone is blocked.** Sage says "I can't open the microphone. Check that one is connected and that Sage may use it." The Voice tab shows the reason. Allow Sage to use the microphone in the system's privacy settings (Windows: Settings, Privacy and security, Microphone; macOS: System Settings, Privacy and Security, Microphone), then press the hotkey again. If you pick a microphone in Settings, check it is the one that is plugged in.

**Sage says it can't hear anything.** It heard only silence for a whole listen after you pressed the hotkey, clicked or said "Hey Sage". Check the microphone is not muted and that Sage may use it.

**The hotkey does nothing, or the Voice tab shows a hotkey error.** Another program probably holds that key combination. Type a different one in Hotkey. You can also use the rail mark or the tray's **Talk to Sage**.

**The download stopped.** Turn Open Sage on again, or press the hotkey. It picks up where it left off. If the network was down, Sage says "The download stopped. It picks up where it left off the next time you open Open Sage." and stays on.

**There is no sound.** Check the Speaker setting and your system volume. Play a voice in Settings. If a speaker or headphones were just plugged in or unplugged, choose the device again. If the tone and speech are both missing, Sage cannot tell you by voice; look at the stage and the Voice tab.

**Sage heard itself on speakers.** Sage has no echo cancellation. Use headphones, or turn down the speaker, and leave **Interrupt by just talking** off.

**"Hey Sage" was missed.** First do **Teach Sage your voice** in Settings, Voice: it tells you how well Sage hears you where you sit. It is a small on-device detector and it will miss sometimes, more in a noisy room, from far away or with music playing. Sage makes a faint microphone louder by itself, up to 40 times, once it has heard your voice; the first thing you say after Sage starts gets less of that help. Say it a little slower, or use the hotkey. "Hey Sage" must be turned on in Settings, and the microphone must be allowed. Say "Hey Sage" and then wait a moment, or carry straight on with your request in the same breath.

**Sage says "Sorry, I didn't catch that."** It heard sound it could not make a sentence of (noise can come out as one word over and over). Say it again.
