# Writes the benchmark document (the bench's SECTION repeated to at least -Bytes, default 5 MiB)
# and prints its size and SHA-256.
param([Parameter(Mandatory = $true)] [string]$Repo, [Parameter(Mandatory = $true)] [string]$Out, [int]$Bytes = 5242880)
$ErrorActionPreference = "Stop"
$rs = Join-Path $Repo "crates\tachyon-doc\benches\reparse.rs"
$src = [IO.File]::ReadAllText($rs) -replace "`r`n", "`n"
$m = [regex]::Match($src, 'const SECTION: &str = r#"(.*?)"#;', [Text.RegularExpressions.RegexOptions]::Singleline)
if (-not $m.Success) { throw "SECTION not found in $rs" }
$section = $m.Groups[1].Value
$utf8 = New-Object System.Text.UTF8Encoding($false)
$sb = New-Object System.Text.StringBuilder
$size = 0
$n = 0
while ($size -lt $Bytes) {
    $s = $section.Replace("{n}", "$n")
    [void]$sb.Append($s)
    $size += $utf8.GetByteCount($s)
    $n++
}
[IO.File]::WriteAllText($Out, $sb.ToString(), $utf8)
$sha = [System.Security.Cryptography.SHA256]::Create()
$stream = [IO.File]::OpenRead($Out)
$hash = -join ($sha.ComputeHash($stream) | ForEach-Object { $_.ToString("X2") })
$stream.Dispose()
Write-Output "bytes=$size sections=$n sha256=$hash"
