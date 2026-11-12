# Open Dot: the hand check after steps 3 to 5

These are the steps to run on the real app, with your own microphone and speaker. Each step says what you do and what should happen. Write down anything that does not match, with the step number.

Steps 1 to 28 were written after step 3 (app wiring). Steps 29 to 57 were added after step 4 (the stage and The Breath) and step 5 (the wake phrase, the tone, stopping it by voice, and spoken notices). A build of the `feat/open-dot` branch has all of them; do them in order.

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

## The stage

29. Press the hotkey.
    - Should: a panel rises from the bottom middle of the window, over whatever screen shows, without covering the rail on the left. In its middle is The Breath, the three dots of the logo. A thin line at the bottom holds a **Close** button with an **Esc** key next to it.

30. Say a sentence slowly, for example **"What needs me today?"**
    - Should: your words appear under The Breath as you speak, in grey, and change as you go. When you stop, the finished sentence stays; then Dot's answer replaces it, one or two lines, the newest darker and larger.

31. Watch The Breath through one whole turn.
    - Should: resting, it breathes slowly (about one breath every four and a half seconds). Listening, the dots lean apart and swell as you talk, the yellow one most; when you are quiet they go back to a small breath. Thinking, they circle each other and touch now and then. Speaking, they melt into one shape that pulses with Dot's voice. When it asks with options, they part into one body per option. When the turn ends, they settle back to the logo once and the panel goes away about two and a half seconds later. Every change is smooth; nothing jumps.

32. Ask something that ends with options (for example "Run the Deploy watcher now"). Click one option. Then do it again and answer with the key **1**, **2**, **3** or **4** instead.
    - Should: the question is printed above the options, which are numbered pills. A click or a number key picks that option; the picked pill is highlighted, the others fade, and a second click or key does nothing. A number key typed while the search box or another text field has focus does not pick.

33. Ask something that shows a card (for example "Set up a dot that tidies my Downloads every morning", or "Approve it" with an approval waiting).
    - Should: a card under the options shows the proposal (name, what it does, when it runs, where it works) or the approval (dot, tool, target). A long card scrolls inside the panel; the options and the Close button stay in view.

34. While the stage shows, press **Esc**. Open it again, click into the search box, and press **Esc** there too.
    - Should: the stage closes and any speech stops at once. Esc closes it even from a text field. With a menu or a dialog open, Esc closes only that, and the stage stays.

35. Ask something with a long answer and, while it speaks, click The Breath.
    - Should: speech stops at once and it listens again.

36. Look at the bottom of the rail on the left.
    - Should: a small, still copy of the logo sits above the usage meter. Click it: Open Dot listens. While the microphone is open the mark breathes and has a thin ring around it; the ring goes away when the microphone closes. With voice off, clicking it opens the Voice settings.

37. Make the window as small as it goes (800 by 560) and repeat steps 32 and 33.
    - Should: the panel fits inside the window, the card scrolls, and the options and Close stay visible. The rail's links scroll if they do not fit.

38. Switch between the light and dark themes (Settings) with the stage showing.
    - Should: the panel, the words, the pills and the card are easy to read in both; the dots keep their colours, and their soft glow is not cut off at the edges.

39. Turn on reduced motion in Windows (Settings, Accessibility, Visual effects, Animation effects off) and restart Dots. Talk to it.
    - Should: the stage fades in without rising; The Breath holds the logo still and only grows brighter and dimmer with the sound.

## The tone

40. With **Play a tone when Dot starts listening** on (the default), press the hotkey; then click the rail mark; then choose **Talk to Dot** in the tray.
    - Should: each time a short, soft two-note tone plays, and what you say right after it is heard. The tone itself is never heard as words.

41. Turn on **Hey Dot** and save. Say "Hey Dot", stop, and wait.
    - Should: about a third of a second after you stop, the tone plays. Then say a sentence: it is heard.

42. Say "Hey Dot" and go straight on talking.
    - Should: no tone plays, and your sentence is heard.

43. Turn **Play a tone when Dot starts listening** off and save. Press the hotkey.
    - Should: no tone; it listens at once.

## Hey Dot

44. With **Hey Dot** on, stand across the room (three or four metres away) and say "Hey Dot".
    - Should: it wakes and listens. Try five times and write down how many worked.

45. Play music on the speakers at a normal level and say "Hey Dot" from your seat.
    - Should: it wakes. Write down how many of five tries worked, and whether the music ever woke it by itself.

46. Say **"Hey Dot, read me the first message"** in one breath, without stopping after "Dot".
    - Should: the whole sentence is heard from "read" on (watch the live words), and no tone plays.

47. Hide the Dots window (minimise it, or close it to the tray) and say "Hey Dot, what needs me?"
    - Should: Dot answers aloud and the window stays hidden. The tray tooltip says "Dot is listening" while the microphone is open.

## Stopping it by voice

48. On speakers: ask something with a long answer and say **"Dot, stop"** while it speaks. Do this ten times.
    - Should: it stops and listens. How many of ten worked on speakers: ____ / 10.

49. On headphones: do the same five times.
    - Should: it stops every time.

50. On headphones, with **Interrupt by just talking** on: ask something with a long answer and start talking over it.
    - Should: it stops and hears what you say. Turn the setting off again afterwards.

## Spoken notices

51. Turn on **Tell me aloud when something needs me** and save, make sure the stage is closed, and make one approval appear (start a dot that asks before running a command).
    - Should: within a second or two Dot says one line, for example "Research needs your approval." It never says the command or what it acts on.

52. Make several approvals or questions appear within a few seconds.
    - Should: one line for the first, then nothing for half a minute, then one line counting what still waits ("Three things are waiting for you."). Anything you answered in between is not counted.

53. Make an approval appear that touches something private (a private folder or a vault value).
    - Should: "One private approval is waiting for you." No dot name.

54. Make an approval appear while you are talking to Dot (the stage is open).
    - Should: nothing is said until the turn is over and the stage has closed; then one line.

55. Rename a dot to a long name (for example "Weekly research summary for the whole team") and make it ask for an approval.
    - Should: "An approval is waiting for you." Long names, names with sentence punctuation, and names that sound like "Hey Dot" or "Dot stop" are not read out.

## What to judge by eye

56. The Breath at the normal window size:
    - The rail mark: is it large enough to read as the logo, and does it sit in line with the other rail icons?
    - Speaking: are all three colours visible inside the melted shape, not mostly one?
    - Asking with two options: the dots part into two bodies (the blue one joins the green one). Does that read as "two choices"? With four options a fourth, teal body appears.
    - The edges at full size: smooth, not jagged.

57. Write down anything that felt slow, jumpy or out of place.

## When you are done

58. If you used a scratch folder, quit the development build and delete `C:\dev\dots-hand-check`. Nothing heard or said was written there or in the logs; the logs only say that turns happened and how long things took.
