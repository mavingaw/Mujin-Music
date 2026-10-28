# The releases page the server's one-click updates come from, from
# MUJIN_RELEASES_BASE (the same setting and rules as tools/releases-base.cjs
# and src/updater.rs, which reads it when the executable is compiled).
function Get-ReleasesBase {
    $default = "https://github.com/mavingaw/Mujin-Music/releases"
    $base = "$env:MUJIN_RELEASES_BASE".Trim().TrimEnd("/")
    if (-not $base) { $base = $default }
    if ($base -notmatch '^https://[^/?#@\s]+(/[^/?#\s]+){2,}/releases$') {
        throw "MUJIN_RELEASES_BASE must be an HTTPS releases page such as https://git.example.com/owner/repo/releases (got '$base')."
    }
    return $base
}
