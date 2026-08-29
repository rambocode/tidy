//! Static Mole.app optimization metadata. Conditional availability belongs in
//! discovery/execution so the list never changes merely because a file is
//! temporarily absent.

use super::OptimizeTask;
use std::path::Path;

fn step(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

fn owned_step(parts: Vec<String>) -> Vec<String> {
    parts
}

/// Build the exact 22-task Mole.app 1.12.1 catalog for one home directory.
pub fn tasks(home: &str) -> Vec<OptimizeTask> {
    let home = Path::new(home);
    let quarantine = home
        .join("Library/Preferences/com.apple.LaunchServices.QuarantineEventsV2")
        .to_string_lossy()
        .into_owned();
    let knowledge = home
        .join("Library/Application Support/Knowledge/knowledgeC.db")
        .to_string_lossy()
        .into_owned();

    vec![
        OptimizeTask {
            id: "rebuildQuickLook",
            title: "Rebuild QuickLook cache",
            description: "Fixes broken file previews in Finder.",
            commands: vec![step(&["/usr/bin/qlmanage", "-r", "cache"])],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "rebuildQuickLookThumbnails",
            title: "Rebuild QuickLook thumbnails",
            description: "Reloads QuickLook generators when file thumbnails are stale.",
            commands: vec![step(&["/usr/bin/qlmanage", "-r"])],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "flushDNS",
            title: "Flush DNS cache",
            description: "Refreshes stale DNS answers after a network change.",
            commands: vec![
                step(&["/usr/bin/dscacheutil", "-flushcache"]),
                step(&["/usr/bin/killall", "-HUP", "mDNSResponder"]),
            ],
            requires_admin: true,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "reindexSpotlight",
            title: "Re-index Spotlight",
            description: "Rebuilds the search index only when indexing is enabled and search is slow.",
            commands: vec![step(&["/usr/bin/mdutil", "-E", "/"])],
            requires_admin: true,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "memoryPurge",
            title: "Purge inactive memory",
            description: "Releases inactive pages only when memory pressure is elevated.",
            commands: vec![step(&["/usr/sbin/purge"])],
            requires_admin: true,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "cleanSavedState",
            title: "Clear stale saved window state",
            description: "Moves saved window state older than 30 days to Trash.",
            commands: vec![step(&[
                "mole:trash",
                "~/Library/Saved Application State/*.savedState (older than 30 days)",
            ])],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "cleanQuarantineEvents",
            title: "Clear quarantine history",
            description: "Clears macOS download history without changing file quarantine flags.",
            commands: vec![owned_step(vec![
                "/usr/bin/sqlite3".into(),
                quarantine,
                "DELETE FROM LSQuarantineEvent; VACUUM;".into(),
            ])],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "cleanCoreDuet",
            title: "Trim oversized usage database",
            description: "Compacts the on-device usage database only after it exceeds 100 MB.",
            commands: vec![owned_step(vec![
                "/usr/bin/sqlite3".into(),
                knowledge,
                "DELETE FROM ZOBJECT WHERE ZCREATIONDATE < (strftime('%s','now','-90 days') - strftime('%s','2001-01-01')); VACUUM;".into(),
            ])],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "preventNetworkDSStore",
            title: "Prevent .DS_Store on network and USB drives",
            description: "Stops Finder writing .DS_Store files to shares and removable drives.",
            commands: vec![
                step(&[
                    "/usr/bin/defaults",
                    "write",
                    "com.apple.desktopservices",
                    "DSDontWriteNetworkStores",
                    "-bool",
                    "true",
                ]),
                step(&[
                    "/usr/bin/defaults",
                    "write",
                    "com.apple.desktopservices",
                    "DSDontWriteUSBStores",
                    "-bool",
                    "true",
                ]),
            ],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "auditLegacyOverrides",
            title: "Remove legacy App Nap and disk-image overrides",
            description: "Removes only hidden overrides left by old tuning tools.",
            commands: vec![
                step(&["/usr/bin/defaults", "delete", "NSGlobalDomain", "NSAppSleepDisabled"]),
                step(&["/usr/bin/defaults", "delete", "com.apple.frameworks.diskimages", "skip-verify-locked"]),
                step(&["/usr/bin/defaults", "delete", "com.apple.frameworks.diskimages", "skip-verify-remote"]),
            ],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "rebuildFontCache",
            title: "Rebuild font cache",
            description: "Rebuilds the user font database after browsers are closed.",
            commands: vec![
                step(&["/System/Library/Frameworks/ApplicationServices.framework/Versions/A/Frameworks/ATS.framework/Versions/A/Support/atsutil", "databases", "-removeUser"]),
                step(&["/System/Library/Frameworks/ApplicationServices.framework/Versions/A/Frameworks/ATS.framework/Versions/A/Support/atsutil", "server", "-shutdown"]),
            ],
            requires_admin: false,
            guard_processes: &[
                "Safari",
                "Google Chrome",
                "Chromium",
                "Firefox",
                "Brave Browser",
                "Microsoft Edge",
                "Arc",
            ],
        },
        OptimizeTask {
            id: "rebuildLaunchServices",
            title: "Rebuild Launch Services database",
            description: "Rebuilds Open With registrations and removes stale app entries.",
            commands: vec![
                step(&["/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister", "-gc"]),
                step(&["/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister", "-r", "-f", "-domain", "local", "-domain", "user", "-domain", "system"]),
            ],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "repairDiskPermissions",
            title: "Repair disk permissions",
            description: "Resets current-user home permissions only when ownership or writability is wrong.",
            commands: vec![step(&["/usr/sbin/diskutil", "resetUserPermissions", "/", "<current-uid>"])],
            requires_admin: true,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "runPeriodicMaintenance",
            title: "Run periodic maintenance",
            description: "Runs daily, weekly, and monthly scripts only when the daily log is stale.",
            commands: vec![step(&["/usr/sbin/periodic", "daily", "weekly", "monthly"])],
            requires_admin: true,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "vacuumSQLiteDatabases",
            title: "Compact Mail and Messages databases",
            description: "Vacuums small Mail, Messages, and Safari databases only when fragmentation exceeds 5%.",
            commands: vec![step(&["/usr/bin/sqlite3", "<eligible-database>", "PRAGMA integrity_check; VACUUM;"])],
            requires_admin: false,
            guard_processes: &["Mail", "Messages", "Safari"],
        },
        OptimizeTask {
            id: "repairSharedFileLists",
            title: "Clean broken shared file list entries",
            description: "Moves invalid .sfl2/.sfl3 Finder lists to Trash while keeping recent-document data.",
            commands: vec![step(&["mole:trash", "invalid non-recent .sfl2/.sfl3 files"])],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "cleanBrokenLaunchAgents",
            title: "Remove broken Launch Agents",
            description: "Moves user LaunchAgent plists to Trash only when their absolute executable is gone.",
            commands: vec![step(&["mole:trash", "broken ~/Library/LaunchAgents/*.plist"])],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "repairBrokenPlists",
            title: "Recover corrupted preferences",
            description: "Moves invalid third-party preference plists to Trash so macOS can regenerate them.",
            commands: vec![step(&["mole:trash", "invalid third-party ~/Library/Preferences/*.plist"])],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "pruneNotificationDB",
            title: "Prune Notification Center database",
            description: "Deletes delivered records older than 30 days only after the database exceeds 50 MB.",
            commands: vec![step(&["/usr/bin/sqlite3", "<notification-db>", "DELETE old delivered records; VACUUM;"])],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "pruneSpotlightOrphanRules",
            title: "Remove orphaned Spotlight rules",
            description: "Removes only valid third-party bundle IDs proven absent from Spotlight and app roots.",
            commands: vec![step(&["/usr/bin/defaults", "write", "com.apple.spotlight", "EnabledPreferenceRules", "-array", "<verified-rules>"])],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "auditLoginItems",
            title: "Audit Login Items",
            description: "Read-only audit that reports launch items whose absolute executable no longer exists.",
            commands: vec![step(&["mole:inspect", "login items and user LaunchAgents"])],
            requires_admin: false,
            guard_processes: &[],
        },
        OptimizeTask {
            id: "flushNetworkStack",
            title: "Flush network stack",
            description: "Flushes route and ARP caches only when network checks fail and no VPN or proxy is active.",
            commands: vec![
                step(&["/sbin/route", "-n", "flush"]),
                step(&["/usr/sbin/arp", "-d", "-a"]),
            ],
            requires_admin: true,
            guard_processes: &[],
        },
    ]
}
