# Sage

Sage is a desktop app for Windows and Mac that runs AI agents, called sols, on your own computer: one at a time in a chat, on a schedule, or together as a team that follows a workflow.

## Install

### Windows

Open PowerShell and run:

```powershell
irm https://raw.githubusercontent.com/Aakshar-D/dots/master/install.ps1 | iex
```

It downloads the newest installer for your PC from this repository's [releases](https://github.com/Aakshar-D/dots/releases) and runs it.

### Mac

Open Terminal and run:

```sh
curl -fsSL https://raw.githubusercontent.com/Aakshar-D/dots/master/install.sh | sh
```

It downloads the newest app from this repository's [releases](https://github.com/Aakshar-D/dots/releases) and puts it in Applications.

Both commands always install the newest release. To install a particular version, set `SAGE_VERSION` first (`$env:SAGE_VERSION = '0.5.4'` in PowerShell, `SAGE_VERSION=0.5.4` before `sh` on a Mac).

### Or download it yourself

Pick the file for your computer on the [newest release](https://github.com/Aakshar-D/dots/releases/latest):

| Computer | File |
|---|---|
| Windows | `Sage_<version>_x64-setup.exe` |
| Windows on ARM | `Sage_<version>_arm64-setup.exe` |
| Mac | `Sage_<version>_universal.dmg` |

Sage's installers are not yet signed with a paid certificate, so your browser and your computer ask before they run one you downloaded yourself. The file is the same one the commands above install.

- **Your browser** may say the file is not commonly downloaded. Choose **Keep**.
- **Windows** may say "Windows protected your PC". Choose **More info**, then **Run anyway**.
- **Mac** may say Sage cannot be opened. Open **System Settings**, then **Privacy & Security**, scroll down and choose **Open Anyway**. On older versions of macOS, right-click the app and choose **Open**.

The install commands above do not meet these questions.

## Updates

Sage updates itself. It checks each update against its own signing key before installing it, so an update is never asked about.
