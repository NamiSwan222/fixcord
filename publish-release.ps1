# Uploads Hearth's source code to GitHub and publishes the release in release\v<version>.
#
# Run it yourself: right-click this file -> "Run with PowerShell".
# It needs Git and the GitHub CLI (https://cli.github.com), signed in once with:  gh auth login
# Nothing is uploaded until you type "yes".

$ErrorActionPreference = "Stop"
$repo = "NamiSwan222/fixcord"
Set-Location $PSScriptRoot

function Stop-With($msg) {
    Write-Host ""
    Write-Host $msg -ForegroundColor Red
    Read-Host "Press Enter to close"
    exit 1
}

# Version from Cargo.toml
$version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$tag = "v$version"
$files = @("release\$tag\Hearth.exe", "release\$tag\Hearth.exe.sha256", "release\$tag\Hearth-$version.zip")
$notes = "release\$tag\RELEASE-NOTES.md"

if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Stop-With "Git isn't installed. Get it from https://git-scm.com and run this again." }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Stop-With "The GitHub CLI isn't installed. Get it from https://cli.github.com, run 'gh auth login', then run this again." }
gh auth status *> $null
if ($LASTEXITCODE -ne 0) { Stop-With "You're not signed in to GitHub. Run 'gh auth login' in a terminal, then run this again." }
foreach ($f in $files + $notes) { if (-not (Test-Path $f)) { Stop-With "Missing $f. Ask the builder to prepare the release files again." } }

Write-Host "This will:" -ForegroundColor Cyan
Write-Host "  1. Upload Hearth's source code to https://github.com/$repo (branch main)"
Write-Host "  2. Publish release $tag with Hearth.exe, its checksum and the zip"
Write-Host "  Everyone running Hearth 0.2.0 or newer will then update to $version."
$answer = Read-Host "Type yes to continue"
if ($answer -ne "yes") { Stop-With "Nothing was uploaded." }

# Git identity: use the GitHub account if none is set.
if (-not (git config user.name)) {
    $login = gh api user --jq .login
    $id = gh api user --jq .id
    git config user.name $login
    git config user.email "$id+$login@users.noreply.github.com"
}

# Source code
if (-not (Test-Path ".git")) { git init -b main | Out-Null }
if (-not (git remote 2>$null | Select-String -SimpleMatch "origin")) { git remote add origin "https://github.com/$repo.git" }
git fetch origin 2>$null
$remoteMain = git ls-remote --heads origin main
git add -A
git diff --cached --quiet
if ($LASTEXITCODE -ne 0) {
    git commit -q -m "Hearth $version" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
}
if ($remoteMain) {
    git merge origin/main --allow-unrelated-histories -m "Merge existing repository content" 2>$null
    if ($LASTEXITCODE -ne 0) { git merge --abort 2>$null; Stop-With "The repository already has files that clash with Hearth's. Nothing was uploaded; ask the builder for help." }
}
git push -u origin main
if ($LASTEXITCODE -ne 0) { Stop-With "Uploading the source code failed (see above)." }

# Release
gh release view $tag --repo $repo *> $null
if ($LASTEXITCODE -eq 0) {
    gh release upload $tag @files --repo $repo --clobber
} else {
    gh release create $tag @files --repo $repo --title "Hearth $version" --notes-file $notes --latest
}
if ($LASTEXITCODE -ne 0) { Stop-With "Publishing the release failed (see above)." }

Write-Host ""
Write-Host "Done: https://github.com/$repo/releases/tag/$tag" -ForegroundColor Green
Read-Host "Press Enter to close"
