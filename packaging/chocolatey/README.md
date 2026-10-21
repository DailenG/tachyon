# Chocolatey package

Source of the [`tachyon`](https://community.chocolatey.org/packages/tachyon) package on the
Chocolatey Community Repository. It embeds the signed Windows zip from the GitHub release;
`legal/VERIFICATION.txt` gives its SHA256.

## What it does

| Step | Script | Effect |
|---|---|---|
| Install | `tools/chocolateyinstall.ps1` | Unzips the release into the package's `tools\` folder (flattened, so `tachyon.exe` keeps the same path across upgrades), marks `tachyon.exe` as a GUI app so Chocolatey's shim returns at once, adds `Tachyon` to the all-users Start menu |
| Upgrade / uninstall | `tools/chocolateybeforemodify.ps1` | Closes Tachyon if it runs from this package (process paths from `Win32_Process`, so other users' instances count too): `tachyon --quit` (hot exit keeps unsaved documents) when it is the only Tachyon in the session, otherwise stops the package's processes; never touches another install such as the MSIX build. If a Tachyon process's path cannot be read, it refuses the change rather than leave files half-replaced |
| Uninstall | `tools/chocolateyuninstall.ps1` | Removes the Start menu shortcut and, for users signed in at the time (loaded registry hives), a sign-in autostart entry that points at this package's `tachyon.exe`; other users keep a harmless stale entry |

User data (`%APPDATA%\Tachyon`, `%LOCALAPPDATA%\Tachyon`) is never touched.

## Releasing a new version

1. Set `<version>` in `tachyon.nuspec`, and the version and the zip's SHA256 in
   `legal/VERIFICATION.txt` (from `SHA256SUMS.txt` on the release page). Update the `iconUrl`
   tag and `releaseNotes` link to the new tag.
2. Download the zip and pack:

   ```powershell
   gh release download v<version> -R DailenG/tachyon -p "*-x86_64-windows.zip" -D tools
   choco pack
   ```

3. Test before pushing: `choco install tachyon -s . -y`, then `tachyon --version`,
   `choco upgrade tachyon -s . -y --force` and `choco uninstall tachyon -y`.
4. `choco push tachyon.<version>.nupkg --source https://push.chocolatey.org/`.
