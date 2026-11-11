# Open Dot: the hand check after step 3 (app wiring)

These are the steps to run on the real app, with your own microphone and speaker, after step 3. Each step says what you do and what should happen. Write down anything that does not match, with the step number.

What you can and cannot see yet: step 3 joins the voice to Dots, but the stage (the panel with The Breath, the live words, the option pills and the card) is step 4. If the build you check does not have step 4 in it, you will hear Open Dot but see very little: the Voice tab's status line ("Ready", "Getting ready", "Off", a trouble sentence, and "Listening" while the microphone is open), the tray tooltip, and the screens Open Dot opens for you. Questions with options are spoken, but there are no pills to click. Where a step needs the stage, it says so; skip those parts if the stage is not there.

The soft tone when Open Dot opens, the "Hey Dot" wake phrase and talking over it on real audio, and the spoken notices ("Tell me aloud when something needs me") are step 5. Turning those settings on is fine, but do not judge how they behave yet.

## Before you start

1. Build and start the development build from the repository folder, in PowerShell:

   ```powershell
   $env:PATH = "C:\Program Files\LLVM\bin;$env:PATH"
   bun run tauri dev
   ```

2. If the installed Dots is running, the development build cannot share its data or its web view. Either quit the installed Dots from its tray first, or start the development build with its own scratch folders:

   ```powershell
   $scratch = "C:\dev\dots-hand-check"
   New-Item -ItemType Directory -Force $scratch | Out-Null
   $env:DOTS_DATA_DIR = "$scratch\data"
   $env:WEBVIEW2_USER_DATA_FOLDER = "$scratch\webview"
   bun run tauri dev
   ```

   With a scratch data folder you start with no dots. Make one or two simple dots first (for example "Research" and "Deploy watcher") so there is something to talk about. The development build cannot use port 47321 while the installed Dots holds it; the log says so, and that is fine.

3. Have a headset (or headphones and a microphone) and normal speakers ready. Some steps are done on both.

## Turning it on

4. Open Settings, then the **Voice** tab. Open Dot is off. Two lines explain that turning it on downloads about 260 MB of speech files once, and that they stay on this computer.
   - Should: nothing is downloading, the microphone light on your computer is off, and no hotkey works yet (press Ctrl+Shift+Space: nothing happens).

5. Turn **Open Dot** on.
   - Should: a progress bar shows megabytes done of the total and moves steadily. A **Cancel** button is there. The status line says "Getting ready".

6. While it downloads, press **Cancel**.
   - Should: the bar goes away, the switch goes back to off, and no error is shown (a cancel is not trouble).

7. Turn Open Dot on again.
   - Should: the download picks up where it stopped (the bar starts part of the way along, not from zero).

8. While it is still downloading, quit Dots from the tray (Quit), then start it again the same way as in step 1 or 2.
   - Should: Dots quits promptly. After the restart, the Voice tab shows Open Dot still on, and the download goes on from where it was without you touching anything.
   - Optional: do the same with the network off before the restart. The download should stop with the sentence "The download stopped. It picks up where it left off the next time you open Open Dot." and Open Dot should stay **on**. Turn the network back on and press the hotkey: the Voice tab should show "Open Dot is still getting ready.", and the download should go on.

9. Let the download finish.
   - Should: the status line changes to "Ready", and you hear "Hi, I'm Dot." once, in the chosen voice. Turning the switch off and on again does not say it again (it greets only after a download).

## Talking to it

10. Press the hotkey: **Ctrl+Shift+Space** (Cmd+Shift+Space on a Mac). Try it once with the Dots window hidden behind another window or minimised.
    - Should: the Dots window comes to the front, the status line adds "Listening", the tray tooltip gains the line "Dot is listening", and your computer's microphone light comes on.

11. Say **"What needs me?"**
    - Should: it notices by itself when you stop talking (no button). Within about two seconds it starts answering aloud with how many things need you, then the first ones. If it takes longer than about a second and a half, it says "One moment." once first. After it finishes it listens for a few more seconds for a follow-up, then the microphone closes and "Listening" goes away.

12. Press the hotkey again and ask a dot something, for example **"Ask Research to compare the three vendors."**
    - Should: if you named the dot clearly, it sends the message and says so ("Sent to Research." or similar), and the message appears in that dot's topic. If it was not sure which dot or what you said, it asks first ("Send to Research: ...?"); say "yes" and it sends.

13. Approve something by voice. You need an approval waiting: start a dot that asks before running a command, so an approval appears in the Inbox. Then press the hotkey and say **"Approve it."**
    - Should: it says which dot, what action and on what, and asks for a yes. It does not approve yet.
    - Say **"okay"**. Should: it answers "I need a clear yes for that. Say yes, or no." and the approval is still waiting in the Inbox.
    - Say **"yes"**. Should: it says "Approved." (or that it was already handled), and the Inbox shows it decided.
    - Repeat with a fresh approval, but this time stay silent for more than a minute after the question. Should: nothing is approved; the question lapses.

14. Click an option instead of speaking (needs the stage, step 4). When it asks a question with options, click an option, or press its number key.
    - Should: the click counts the same as saying it. On a yes-or-no question, clicking Yes approves and clicking No does not. Only the first click counts.

15. Ask it to open a screen: **"Open the Research dot"**, then **"Show settings."**
    - Should: the app goes to that dot, then to Settings.

## Interrupting

16. Ask something with a long answer (for example "What's going on?" with several dots busy). While it is speaking, press the hotkey.
    - Should: it stops speaking at once and listens again.

17. On headphones only: in the Voice tab turn on **Interrupt by just talking**, save, then ask something and start talking while it answers.
    - Should: it stops and listens to you. (How well this works on speakers is step 5's job; on speakers its own voice may set it off.) Turn the setting off again afterwards.

## Voice settings

18. In the Voice tab, choose another voice and press its play button. Move the speaking speed slider and press play again.
    - Should: you hear "Hi, I'm Dot." in that voice at that speed, at once. After the preview, Open Dot goes back to the voice and speed that are saved until you save the new ones. Save, then ask it something: it now answers in the new voice and speed.

19. Choose another microphone and another speaker in the Devices part, and save.
    - Should: the save is instant; within a few seconds the status line is "Ready" again, and the next answer plays on the new speaker and hears you on the new microphone. Anything that was waiting for a yes is forgotten (that is expected).

20. With a headset as both microphone and speaker, ask something with a long answer and unplug the headset while it speaks.
    - Should: no crash and no endless silence. Either the answer goes on through the system's next speaker, or the status line shows a sentence starting "The speaker ..." or "The microphone ...". Plug it back in and press the hotkey: it should work again, and the sentence goes away once it is listening.

21. Set the hotkey to one another app already holds (for example a shortcut your screen-recorder or chat app uses), and save.
    - Should: the setting is saved, and under the hotkey field it says "That shortcut is already used by another app. Pick a different one." The old hotkey keeps working. Nothing else changes.
    - Also try a single plain key such as **K**. Should: "Add Ctrl, Alt or Shift to that shortcut, or use a function key." A function key on its own, such as **F9**, is allowed.
    - Put the default back (Ctrl+Shift+Space) and save; the message goes away.

## The tray

22. Open the tray menu.
    - Should: there is a **Talk to Dot** item right after "Open Dots". It is enabled only while Open Dot is ready. Choosing it brings the window forward and starts listening, like the hotkey.

23. Hover the tray icon while Open Dot is listening, and again after it stops.
    - Should: while listening the tooltip ends with "Dot is listening"; once the microphone closes that line goes away.

## Turning it off and removing it

24. Turn Open Dot off in the Voice tab.
    - Should: your computer's microphone light goes out at once (within a second), the hotkey does nothing, "Talk to Dot" in the tray is disabled, and the status line says "Off".

25. Turn it on again (no download this time; it is ready within a few seconds), then press **Remove speech files** and confirm.
    - Should: Open Dot turns off and the speech files are deleted (about 260 MB freed under the data folder's `voice\models`). Turning it on again downloads them again.

## Short answers and noise

26. Ask something that ends in a yes-or-no question (for example "Run the Deploy watcher now"). Answer with a curt **"yes."**, as short as you naturally would.
    - Should: the short yes is heard and acted on. Try "no." too.

27. After a question, cough once instead of answering.
    - Should: the cough is not taken as an answer; nothing is approved or run. It keeps waiting, or the question lapses.

## Speakers and headphones

28. Do steps 10 to 13 and 16 once on speakers and once on headphones.
    - Should: on both, it never hears the tail of its own answer as you talking (it does not answer itself). Write down how long it takes from the end of your sentence to its first word, on both.

## When you are done

29. If you used a scratch folder, quit the development build and delete `C:\dev\dots-hand-check`. Nothing heard or said was written there or in the logs; the logs only say that turns happened and how long things took.
