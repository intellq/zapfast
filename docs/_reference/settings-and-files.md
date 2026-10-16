---
title: Settings & Files
description: Settings and paths for the archive, configuration, caches, and logs.
nav_order: 0
---

## File locations

ZapFast follows each platform's conventions. On Linux:

| What | Where | Safe to delete? |
| --- | --- | --- |
| Settings | `~/.config/zapfast/settings.json` | Yes, you lose preferences |
| Message archive | `~/.local/state/zapfast/archive.db` | Yes; only history available from WhatsApp can be restored |
| Session keys | `~/.local/state/zapfast/session.db` | Yes; you must link again |
| Attachments | `~/.cache/zapfast/media/` | Yes; available files download again when viewed |
| Profile pictures | `~/.cache/zapfast/avatars/` | Always |
| Stickers | `~/.cache/zapfast/stickers/` | Always |
| GIF search stills | `~/.cache/zapfast/gifs/` | Always |
| Last run's log | `~/.local/state/zapfast/zapfast.log` | Always |
| Crash log | `~/.local/state/zapfast/panic.log` | Always |

Back up the archive if you need its history. WhatsApp sends only recent
history to a new device, although ZapFast can request some older messages from
the phone. Clearing the media cache makes ZapFast download attachments again.
Expired attachments may still be available through the phone.
**Settings > Files > Storage used** shows how much space the account on screen
has in its media cache, split into images, videos and GIFs, stickers, and other
files. **Clear…** there deletes that account's cached attachments after asking:
its messages, its other caches, and the other accounts' files stay, and so do
files in a download folder you chose.
**Settings > Files > Change…** sends new downloads to another folder, leaving
earlier ones in place. There, documents keep the name they were sent with; a
file already in the folder is never replaced, and the download becomes
`name (1).pdf`, `name (2).pdf`, and so on. Characters a file system refuses
become `_`.

On macOS, settings, state, and the logs are in
`~/Library/Application Support/me.paolino.zapfast` and the caches in
`~/Library/Caches/me.paolino.zapfast`. On Windows, settings are in
`%APPDATA%\paolino\zapfast\config`, state and the logs in
`%LOCALAPPDATA%\paolino\zapfast\data`, and the caches in
`%LOCALAPPDATA%\paolino\zapfast\cache`.

On first start, ZapFast moves the corresponding `fastsapp` directories (or
`fastwhatsapp` from earlier versions), including the session, archive, saved
stickers, and window state. Existing ZapFast directories are never overwritten.
Quit FastsApp first; launching ZapFast while it is running brings the existing
window forward.

## Settings

Changes on the Settings page are saved to `settings.json` immediately. The
search field at the top (`Ctrl+F`, Command+F on macOS) finds a setting by its
name or description, in the interface language or in English.

**Appearance**

- **Theme**: dark, light, follow the system, or a local JSON palette from the
  themes folder. See [Making a theme]({{ '/themes/' | relative_url }}).
- **Wallpaper**: WhatsApp's light and dark chat wallpaper colours, with or
  without doodles.
- **Zoom**: interface scale, also `Ctrl+Plus`, `Ctrl+Minus` and `Ctrl+0`.
- **Language**: the interface language, or **Auto** to follow the system.

**Chats**

- **Enter sends**: when off, Enter adds a line and `Ctrl+Enter` (Command+Enter
  on macOS) sends.
- **Download files automatically**: files up to 64 MiB, stickers included,
  download as they come into view. When off, click one to download it.
- **Pause other media while recording or playing**: pause music and videos in
  other apps while you record, or while a voice message, audio, or video plays
  with sound, and resume them afterwards. Linux and Windows only.
- **Locked chats code**: the local code that opens the **Locked** tab. It hides
  chats; it does not encrypt them.

**Notifications**

- **Desktop notifications**: for chats you are not looking at. Muted chats stay
  quiet.
- **Show message content in notifications**: on by default. When off, every
  notification says only "New message" from ZapFast, without the chat, the
  sender, the text, or a picture, as behind the app lock, and plays only the
  message sound. Clicking one still opens the message. Applies to every
  account.
- **Message sound** and **Mention sound**: Pidgin's sounds, the system sound,
  none, or an audio file.
- **Play sounds for group messages**: when off, only mentions and replies to
  you make a sound in groups.
- **Limit repeat notifications**: off by default. Choose **Direct messages**,
  **Groups**, or **Direct messages and groups**. A limited chat sends one
  notification (and its sound), then stays quiet for 10 minutes. Reading the
  chat, here or on another device, starts over. In groups, a mention of you or a reply to you always
  notifies. Settings for this account.

**Privacy**

- **Send read receipts**: the blue ticks others see, subject to the account
  setting below. When account read receipts are off, read ticks are also hidden
  on outgoing direct messages in the conversation, chat list, and Message info,
  as on the phone. Group receipts and voice-message played receipts are
  unaffected.
- **Show when you are typing**: send typing and recording state.
- **Last seen**, **Online**, **Profile photo**, **About**, **Groups**, **Read
  receipts**, **Calls**: your WhatsApp account privacy, stored on WhatsApp's
  servers and shared with your phone. They can be changed while connected.

**System**

- **Keep running when the window closes**: keep ZapFast linked in the tray.
- **Start at login**: start in the tray without a window, where the platform
  supports it. If the desktop's tray is not up yet, ZapFast waits up to 30
  seconds for it, then opens the window, since a desktop without a tray could
  not bring it back.
- **Check for updates**: ask GitHub once a day whether a newer release exists.
- **Download updates automatically**: download and verify a new release in the
  background; restarting stays your choice. Package managers and Flatpak update
  ZapFast themselves.
- **Proxy**: for WhatsApp, media, and updates. Empty uses `ALL_PROXY` or
  `HTTPS_PROXY`.
- **GIPHY API key**: for GIF search, unless the build includes one. Set
  `ZAPFAST_GIPHY_KEY` at compile time to include a default key. The earlier
  `FASTSAPP_GIPHY_KEY` remains a fallback for existing builds.

**Account** edits your WhatsApp name, About, and picture, and unlinks this
computer. **Files** shows the archive, the downloads folder (which you can
change; earlier downloads stay where they are), and this run's log.

Some choices are made where they are used and remembered in `settings.json`:
the shortcut hints bar under the composer (its × hides it, and **Show shortcut
hints under the message box** in the Keyboard shortcuts dialog brings it back),
**Also save to your phone's contacts** in the new-contact dialog, voice
playback speed, and the chat list and search pane widths.

Labels always get a chip each, in a row under the filter chips, once a label
exists. They are kept in the message archive, next to your chats, and never
leave this computer. Sender pictures appear in groups only, and names from your
address book come before public profile names. Hiding the chat list (`Ctrl+B`)
collapses it to a column of avatars with unread badges.

Settings from earlier versions that no longer exist are ignored and dropped
the next time settings are saved. The two earlier media pause switches become
the one above: it stays on only if both were on.

## The log

Each run replaces `zapfast.log` and records warnings and errors. Include the
end of this file when reporting an issue.
