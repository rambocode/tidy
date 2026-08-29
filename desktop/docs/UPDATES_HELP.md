# Updating software

The Updates tab checks five update sources: Homebrew, the Mac App Store,
Sparkle, Electron, and official websites.

- Homebrew apps and packages show an exact command to run in Terminal. Mole
  does not execute package-manager upgrades.
- App Store updates open the exact product in the App Store.
- Sparkle and Electron updates open the installed app so its own signed updater
  can finish the installation.
- “Source diagnostics” means one source timed out, returned invalid metadata,
  or could not be reached. Mole will not call that source “up to date”.

Use **Ignore** to move an update to Hidden Updates. **Show again** restores it.
Delegated App updates can run as a batch. Actions run one item at a time and
can be cancelled; Homebrew rows remain read-only.

Mole does not directly replace App Store, Sparkle, Electron, or website app
bundles in this open-source build. That avoids installing an unverified download
or bypassing macOS App Management permissions.
