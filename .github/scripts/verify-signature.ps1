# Verifies that a file carries a valid Authenticode signature. Used by the release workflow
# right after Azure Trusted Signing, so a signing failure that the action did not catch still
# fails the job instead of shipping an unsigned executable.
#
# Usage: pwsh -File verify-signature.ps1 -Path <file>

param(
    [Parameter(Mandatory = $true)]
    [string]$Path
)

$ErrorActionPreference = 'Stop'

$signature = Get-AuthenticodeSignature -FilePath $Path

if ($signature.Status -ne 'Valid') {
    Write-Error "Authenticode signature for $Path is $($signature.Status): $($signature.StatusMessage)"
    exit 1
}

Write-Host "Authenticode signature for $Path is valid (signer: $($signature.SignerCertificate.Subject))"
