# Security policy

Earshot Notes records and stores private conversations, so security problems matter even when they seem small.

## Report a vulnerability

Report it privately through GitHub: on the repository's **Security** tab, select **Report a vulnerability**. Don't open a public issue, and don't include real meeting content in the report.

Include what you found, the steps to reproduce it, the Earshot Notes and macOS versions, and what an attacker could do with it. You'll get an answer within a week. Once a fix is released, the advisory credits you unless you'd rather stay anonymous.

## Supported versions

Only the [latest release](https://github.com/tapresle/Earshot-Notes/releases/latest) gets security fixes. Releases before 0.1.6 aren't notarized by Apple.

## In scope

- Anything that sends meeting audio, transcripts, notes, summaries, or chats off your Mac.
- Network requests the app makes beyond the model downloads you start and the update check.
- Model downloads that skip their pinned checksum, or that can be pointed at another file.
- Model output, or a transcript, that injects HTML or script into the app.
- Ways for another app or user on your Mac to read Earshot Notes' data without your permission.

## Out of scope

- Someone with full access to your unlocked Mac reading your meetings. The app's data is protected by macOS, not encrypted by Earshot Notes.
- A local AI model writing inaccurate notes. That's a quality bug, so open an issue.
