# What Optimize changes

Optimize performs a bounded maintenance pass. It does not install packages,
patch apps, reset privacy permissions, or delete personal documents.

The pass covers four areas:

- **Launch speed**: refresh QuickLook, fonts, Launch Services, stale window
  state, and report broken login items or LaunchAgents.
- **System databases**: compact eligible Mail/Messages/Safari and usage
  databases, prune oversized Notification Center history, clear download
  history, and repair invalid preference/shared-list files.
- **Network and search**: refresh DNS, route/ARP caches, Spotlight index/rules,
  and enable Finder's no-`.DS_Store` preferences for network and USB drives.
- **System maintenance**: remove three legacy tuning overrides, release inactive
  memory when pressure is high, repair current-user permissions when broken,
  and run stale periodic scripts.

## Why a task may not run

- `unchanged`: the target is missing, already healthy, below its size limit, or
  not fragmented enough to justify the I/O.
- `apps_running`: close the named app first.
- `probe_failed`: Mole could not prove a database/app/rule was safe to touch.
- `requires_admin`: the action needs the signed maintenance helper. Mole does
  not fall back to a shell elevation prompt.
- `skipped`: the target is busy, whitelisted, or another safety condition says
  to leave it alone.

Files selected for removal are moved to Trash and recorded in the same history
logs as other desktop and CLI operations. Database and preference tasks act on
the exact paths or keys named in the task; they do not use vendor-wide or
display-name wildcards.
