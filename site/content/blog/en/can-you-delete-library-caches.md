---
title: "Can you just delete ~/Library/Caches?"
description: "You can, but you shouldn't. What lives in that directory, which parts cost you something to remove, and why running apps are the exception."
lang: en
permalink: can-you-delete-library-caches
translationKey: can-you-delete-library-caches
category: space
tags: ["disk space", "cache"]
publishedAt: 2026-08-10
updatedAt: null
readingMinutes: null
author: null
cover: null
coverAlt: null
featured: true
draft: false
noindex: false
canonical: null
toc: true
cta: source
series: null
seriesOrder: null
related: []
---

## The short answer

Technically yes. Everything under `~/Library/Caches` is rebuildable by definition, and macOS plus almost every app will start fine after you wipe it.

But wiping it is a blunt instrument. It costs you things you did not need to pay — signing in again, downloading again, a build that takes fifteen extra minutes — and the space you get back is concentrated in a handful of directories anyway.

## What is actually in there

`~/Library/Caches` is the conventional location for user-level caches. Each app creates a subdirectory named after its bundle identifier. Start by looking at the distribution:

```bash
du -sh ~/Library/Caches/* 2>/dev/null | sort -rh | head -20
```

The top entries are usually some mix of these:

| Directory | What it holds | Cost of removing it |
| --- | --- | --- |
| `com.apple.dt.Xcode`, `org.swift.swiftpm` | Xcode build products, fetched dependencies | The next build is noticeably slower and re-fetches |
| `Homebrew` | Downloaded bottle archives | Reinstalling the same package downloads again |
| `com.google.Chrome`, `Firefox`, … | Browser disk caches | The first few page loads are slower; logins and history are elsewhere |
| `Figma`, `Sketch`, … | Design tool assets and font caches | Fetched again on next use; your files are elsewhere |
| Messaging apps | Cached images and video | Old media re-downloads when needed |

## What costs you almost nothing

- **Browser caches.** Cookies, sessions, and history live under `~/Library/Application Support`, not in Caches. Clearing the cache does not sign you out.
- **Log-style caches.** Diagnostic material the app rebuilds on demand.
- **Leftover installers.** Strictly not in Caches, but ignored alongside it — stray `.dmg` / `.pkg` / `.xip` files are often larger than the caches and cost nothing to delete.

## What genuinely costs something

- **Xcode.** Removing `DerivedData` and the SwiftPM cache can add fifteen minutes to the next clean build. Worth it when you are out of space, not worth it routinely.
- **Homebrew bottles.** `brew cleanup` already handles this; removing them by hand is fine, you just re-download on reinstall.
- **Caches of running apps.** See the next section.

## Running apps are the exception

This is the one place in the directory where something can actually break. A running app may hold open handles to its cache files and may keep an in-memory index that corresponds to what is on disk. Pull the files out from under it and, at best, the cache rebuilds on next launch; at worst the app lands in an error state because a resource it expected has vanished.

So the correct order is: **quit the app, then clear its cache.**

That step is hard to get right in Finder, because you cannot tell which directory belongs to which running process. The command line can tell you:

```bash
lsof +D ~/Library/Caches/com.example.app 2>/dev/null | head
```

Doing that for two dozen directories one at a time is not realistic.

## The step Tidy removes

[Tidy](/en/) folds this judgement into the scan itself. Candidates are grouped by app caches, logs, developer tools, AI tools, browsers, design tools, messaging, and installers, each with its size — and **running apps are called out separately**, with a concrete number like "Close Slack to reclaim another 2.2 GB" rather than a vague warning.

The part that matters is the execution boundary: the scan result is stored, and execution accepts only the subset you ticked in the preview. Nothing is re-scanned after confirmation. The list you looked at is exactly what gets removed — no more, no less. Deletions go to the Trash, so a mistake is recoverable.

> [!NOTE]
> System-level caches (`/Library/Caches` rather than `~/Library/Caches`) need the privileged helper. That transport has not shipped, so Tidy refuses those with `requires_admin` instead of showing a sudo prompt.

## A practical order

1. Look at the distribution first and find out what is actually consuming the space.
2. Clear the free ones: browsers, messaging, design tools.
3. Remove leftover installers — usually the best ratio of space to regret.
4. Only touch Xcode and Homebrew when space is genuinely tight, knowing the next build is slower.
5. Quit an app before clearing its cache.
